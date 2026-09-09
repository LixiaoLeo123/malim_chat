use super::*;
use crate::respond::clear_chain;

/// Summarising older turns into one durable message, automatically when the context
/// meter nears the model's window and on request from the composer.

pub(crate) async fn auto_compact(pool: &PgPool, conversation_id: Uuid) -> Result<(), ApiError> {
    let values: (i32, i32, i64) = sqlx::query_as(
        "SELECT context_tokens,context_window,next_sequence FROM conversations WHERE id=$1",
    )
    .bind(conversation_id)
    .fetch_one(pool)
    .await?;
    if values.0 < (values.1 as f32 * 0.70) as i32 {
        return Ok(());
    }
    let previous: Option<(String, i64)> = sqlx::query_as("SELECT content,ends_at_sequence FROM conversation_summaries WHERE conversation_id=$1 ORDER BY ends_at_sequence DESC LIMIT 1").bind(conversation_id).fetch_optional(pool).await?;
    let existing_end = previous.as_ref().map(|summary| summary.1).unwrap_or(0);
    let end = values.2 - 1;
    if end <= existing_end {
        return Ok(());
    }
    let rows: Vec<(String, String, Value)> = sqlx::query_as("SELECT role,content,images FROM messages WHERE conversation_id=$1 AND sequence > $2 AND sequence <= $3 AND deleted_at IS NULL ORDER BY sequence").bind(conversation_id).bind(existing_end).bind(end).fetch_all(pool).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let compacted = rows
        .iter()
        .map(|(role, content, images)| {
            let has_images = images
                .as_array()
                .map(|items| !items.is_empty())
                .unwrap_or(false);
            format!(
                "{role}: {}{}",
                strip_thinking(content),
                if has_images {
                    " [image attachments were present in this message]"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let content = format!(
        "Conversation summary through message {end}:\n{}\n{}",
        previous.map(|summary| summary.0).unwrap_or_default(),
        compacted
    )
    .chars()
    .take(30_000)
    .collect::<String>();
    sqlx::query("INSERT INTO conversation_summaries (id,conversation_id,starts_at_sequence,ends_at_sequence,content,token_count) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (conversation_id,starts_at_sequence,ends_at_sequence) DO UPDATE SET content=EXCLUDED.content,token_count=EXCLUDED.token_count").bind(Uuid::new_v4()).bind(conversation_id).bind(existing_end + 1).bind(end).bind(&content).bind(estimate_tokens(&content)).execute(pool).await?;
    sqlx::query("UPDATE conversations SET context_tokens=$2 WHERE id=$1")
        .bind(conversation_id)
        .bind(estimate_tokens(&content))
        .execute(pool)
        .await?;
    // The provider still holds the whole chain it was fed; the summary replaced it here,
    // so the next turn has to re-seed from the transcript.
    clear_chain(pool, conversation_id).await?;
    Ok(())
}

pub(crate) async fn compact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(request): Json<CompactRequest>,
) -> Result<Json<Value>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let c = own_conversation(&state.db, user_id, id).await?;
    if !request.force.unwrap_or(false) && c.context_tokens < (c.context_window as f32 * 0.70) as i32
    {
        return Ok(Json(
            json!({"compacted":false,"reason":"below_threshold","context_tokens":c.context_tokens,"context_window":c.context_window}),
        ));
    }
    let last:(i64,)=sqlx::query_as("SELECT COALESCE(MAX(sequence),0) FROM messages WHERE conversation_id=$1 AND deleted_at IS NULL").bind(id).fetch_one(&state.db).await?;
    let end = request.through_sequence.unwrap_or(last.0);
    let rows:Vec<(String,String,Value)>=sqlx::query_as("SELECT role,content,images FROM messages WHERE conversation_id=$1 AND sequence <= $2 AND deleted_at IS NULL ORDER BY sequence").bind(id).bind(end).fetch_all(&state.db).await?;
    if rows.is_empty() {
        return Err(ApiError::bad("There are no messages to compact."));
    }
    let source = rows
        .iter()
        .map(|(role, content, images)| {
            let has_images = images
                .as_array()
                .map(|items| !items.is_empty())
                .unwrap_or(false);
            format!(
                "{role}: {}{}",
                strip_thinking(content),
                if has_images {
                    " [image attachments were present in this message]"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let prior: Option<(String, i64)> = sqlx::query_as("SELECT content,ends_at_sequence FROM conversation_summaries WHERE conversation_id=$1 ORDER BY ends_at_sequence DESC LIMIT 1").bind(id).fetch_optional(&state.db).await?;
    let model_summary = if let Some(provider_id) = c.model_provider_id {
        let p:(String,String,String,Vec<u8>,Vec<u8>)=sqlx::query_as("SELECT kind,base_url,default_model,encrypted_api_key,key_nonce FROM providers WHERE id=$1 AND user_id=$2").bind(provider_id).bind(user_id).fetch_optional(&state.db).await?.ok_or_else(|| ApiError::bad("The selected provider was removed."))?;
        let api_key = decrypt(&state, &p.3, &p.4)?;
        let model = match c.model.as_deref().filter(|value| !value.trim().is_empty()) {
            Some(value) => value.to_string(),
            None => provider_first_model(&state.db, provider_id)
                .await?
                .unwrap_or_else(|| p.2.clone()),
        };
        let kind = provider_model_kind(&state.db, provider_id, &model)
            .await?
            .unwrap_or_else(|| p.0.clone());
        info!(conversation_id=%id, through_sequence=end, source_characters=source.len(), "manual compaction started");
        providers::call_provider(&state.http, &kind, &p.1, &api_key, &model, &[json!({"role":"system","content":"Create a concise, factual memory for continuing this conversation. Preserve decisions, constraints, user preferences, unresolved tasks, and important technical details. Do not use Markdown."}), json!({"role":"user","content":format!("Previous memory:\n{}\n\nConversation to compact:\n{}", prior.as_ref().map(|item| item.0.as_str()).unwrap_or(""), source.chars().take(120_000).collect::<String>())})], Some(0.2), None, providers::Tools::None, None).await?.text
    } else {
        source.chars().take(30_000).collect()
    };
    let concise = format!("Conversation summary through message {end}:\n{model_summary}");
    let summary_id = Uuid::new_v4();
    let summary_tokens = estimate_tokens(&concise);
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO conversation_summaries (id,conversation_id,starts_at_sequence,ends_at_sequence,content,token_count) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (conversation_id,starts_at_sequence,ends_at_sequence) DO UPDATE SET content=EXCLUDED.content,token_count=EXCLUDED.token_count").bind(summary_id).bind(id).bind(prior.as_ref().map(|item| item.1 + 1).unwrap_or(1)).bind(end).bind(&concise).bind(summary_tokens).execute(&mut *tx).await?;
    let sequence:(i64,)=sqlx::query_as("UPDATE conversations SET next_sequence=next_sequence+1,context_tokens=$3,revision=revision+1 WHERE id=$1 AND user_id=$2 RETURNING next_sequence-1").bind(id).bind(user_id).bind(summary_tokens).fetch_one(&mut *tx).await?;
    let marker = format!(
        "Context compacted through message {end}. The conversation now uses a durable summary ({summary_tokens} estimated tokens)."
    );
    let message:Message=sqlx::query_as("INSERT INTO messages (id,conversation_id,sequence,role,content,content_format,token_count) VALUES ($1,$2,$3,'summary',$4,'plain',0) RETURNING id,conversation_id,sequence,client_mutation_id,role,content,reasoning_content,content_format,status,model,token_count,search_sources,images,edited_at,created_at,updated_at").bind(Uuid::new_v4()).bind(id).bind(sequence.0).bind(marker).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    clear_chain(&state.db, id).await?;
    event(
        &state.db, user_id, "message", message.id, "created", sequence.0,
    )
    .await?;
    info!(conversation_id=%id, through_sequence=end, summary_tokens, "manual compaction completed");
    Ok(Json(
        json!({"compacted":true,"summary_id":summary_id,"through_sequence":end,"estimated_tokens":summary_tokens,"message":message,"context_tokens":summary_tokens}),
    ))
}
