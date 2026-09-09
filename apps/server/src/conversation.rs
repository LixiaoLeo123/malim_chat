use super::*;

/// Conversations and their messages: pagination, optimistic writes, the context-token
/// bookkeeping the meter reads, and the sync events clients replay against.

pub(crate) async fn list_conversations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<Conversation>>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let limit = page
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let cursor = page
        .cursor
        .as_deref()
        .and_then(|s| s.parse::<DateTime<Utc>>().ok());
    let rows=sqlx::query_as::<_,Conversation>("SELECT id,title,model_provider_id,model,context_window,context_tokens,is_favorite,generation_settings,revision,created_at,updated_at FROM conversations WHERE user_id=$1 AND archived_at IS NULL AND ($2::timestamptz IS NULL OR updated_at < $2) ORDER BY updated_at DESC,id DESC LIMIT $3").bind(user_id).bind(cursor).bind(limit+1).fetch_all(&state.db).await?;
    let next = rows.get(limit as usize).map(|c| c.updated_at.to_rfc3339());
    Ok(Json(Page {
        items: rows.into_iter().take(limit as usize).collect(),
        next_cursor: next,
    }))
}

pub(crate) async fn create_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateConversation>,
) -> Result<Json<Conversation>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let (model, context_window) = if let Some(pid) = request.provider_id {
        own_provider(&state.db, user_id, pid)
            .await
            .map_err(|_| ApiError::bad("Selected provider does not belong to this account."))?;
        let model = match request.model {
            Some(value) => value,
            None => provider_first_model(&state.db, pid).await?.ok_or_else(|| {
                ApiError::bad("Add a configured model to this provider before creating a chat.")
            })?,
        };
        if !configured_model(&state.db, pid, &model).await? {
            return Err(ApiError::bad(
                "Choose a configured model for this provider.",
            ));
        }
        let context_window: (i32,) = sqlx::query_as(
            "SELECT context_window FROM provider_models WHERE provider_id=$1 AND model=$2",
        )
        .bind(pid)
        .bind(&model)
        .fetch_one(&state.db)
        .await?;
        (Some(model), context_window.0)
    } else {
        (request.model, 128_000)
    };
    let conversation:Conversation=sqlx::query_as("INSERT INTO conversations (id,user_id,title,model_provider_id,model,context_window) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id,title,model_provider_id,model,context_window,context_tokens,is_favorite,generation_settings,revision,created_at,updated_at").bind(Uuid::new_v4()).bind(user_id).bind(request.title.unwrap_or_else(||"New chat".into()).trim()).bind(request.provider_id).bind(model).bind(context_window).fetch_one(&state.db).await?;
    event(
        &state.db,
        user_id,
        "conversation",
        conversation.id,
        "created",
        conversation.revision,
    )
    .await?;
    Ok(Json(conversation))
}

pub(crate) async fn update_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateConversation>,
) -> Result<Json<Conversation>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let existing = own_conversation(&state.db, user_id, id).await?;
    let generation_settings = request
        .generation_settings
        .map(validate_generation_settings)
        .transpose()?;
    let model_changed = request.model.is_some();
    let model = request.model.map(|value| value.trim().to_string());
    if model.as_ref().is_some_and(String::is_empty) {
        return Err(ApiError::bad("Model name cannot be empty."));
    }
    let provider_id = request.provider_id.or(existing.model_provider_id);
    let selected_model = model.as_deref().or(existing.model.as_deref());
    let mut selected_context_window = None;
    if let (Some(provider_id), Some(model)) = (provider_id, selected_model) {
        own_provider(&state.db, user_id, provider_id)
            .await
            .map_err(|_| ApiError::bad("Selected provider does not belong to this account."))?;
        if !configured_model(&state.db, provider_id, model).await? {
            return Err(ApiError::bad(
                "Choose a configured model for this provider.",
            ));
        }
        if request.provider_id.is_some() || model_changed {
            selected_context_window = Some(
                sqlx::query_as::<_, (i32,)>(
                    "SELECT context_window FROM provider_models WHERE provider_id=$1 AND model=$2",
                )
                .bind(provider_id)
                .bind(model)
                .fetch_one(&state.db)
                .await?
                .0,
            );
        }
    }
    let c:Conversation=sqlx::query_as("UPDATE conversations SET title=COALESCE($3,title),archived_at=CASE WHEN $4::boolean IS TRUE THEN now() WHEN $4::boolean IS FALSE THEN NULL ELSE archived_at END,model_provider_id=COALESCE($5,model_provider_id),model=COALESCE($6,model),context_window=COALESCE($7,context_window),generation_settings=COALESCE($8,generation_settings),is_favorite=COALESCE($9,is_favorite),chain_response_id=CASE WHEN $5::uuid IS DISTINCT FROM model_provider_id OR $6::text IS DISTINCT FROM model THEN NULL ELSE chain_response_id END,chain_sequence=CASE WHEN $5::uuid IS DISTINCT FROM model_provider_id OR $6::text IS DISTINCT FROM model THEN 0 ELSE chain_sequence END,revision=revision+1 WHERE id=$1 AND user_id=$2 RETURNING id,title,model_provider_id,model,context_window,context_tokens,is_favorite,generation_settings,revision,created_at,updated_at").bind(id).bind(user_id).bind(request.title.map(|v|v.trim().to_string())).bind(request.archived).bind(request.provider_id).bind(model).bind(selected_context_window).bind(generation_settings).bind(request.is_favorite).fetch_one(&state.db).await?;
    event(
        &state.db,
        user_id,
        "conversation",
        id,
        "updated",
        c.revision,
    )
    .await?;
    Ok(Json(c))
}

pub(crate) async fn delete_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let r = sqlx::query("DELETE FROM conversations WHERE id=$1 AND user_id=$2")
        .bind(id)
        .bind(user_id)
        .execute(&state.db)
        .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    };
    event(&state.db, user_id, "conversation", id, "deleted", 0).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<Message>>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    own_conversation(&state.db, user_id, id).await?;
    let limit = page
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let cursor = page.cursor.as_deref().and_then(|s| s.parse::<i64>().ok());
    let mut rows=sqlx::query_as::<_,Message>("SELECT id,conversation_id,sequence,client_mutation_id,role,content,reasoning_content,content_format,status,model,token_count,search_sources,images,edited_at,created_at,updated_at FROM messages WHERE conversation_id=$1 AND deleted_at IS NULL AND ($2::bigint IS NULL OR sequence < $2) ORDER BY sequence DESC LIMIT $3").bind(id).bind(cursor).bind(limit+1).fetch_all(&state.db).await?;
    let next = rows.get(limit as usize).map(|m| m.sequence.to_string());
    rows.truncate(limit as usize);
    rows.reverse();
    Ok(Json(Page {
        items: rows,
        next_cursor: next,
    }))
}

pub(crate) async fn create_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(request): Json<CreateMessage>,
) -> Result<Json<Message>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let c = own_conversation(&state.db, user_id, id).await?;
    let content = request.content.trim();
    let images = request.images.unwrap_or_default();
    if images.len() > 8 {
        return Err(ApiError::bad("A message may contain at most 8 images."));
    }
    for image in &images {
        if !image.starts_with("data:image/") || image.len() > 7_000_000 {
            return Err(ApiError::bad(
                "Each image must be a data URL of at most 5 MB.",
            ));
        }
        if parse_data_url(image).is_none() {
            return Err(ApiError::bad(
                "Each image must be a valid base64 image data URL.",
            ));
        }
    }
    if content.is_empty() && images.is_empty() {
        return Err(ApiError::bad(
            "Message must contain text or at least one image.",
        ));
    }
    if content.len() > 200_000 {
        return Err(ApiError::bad(
            "Message text must be at most 200,000 characters.",
        ));
    };
    if let Some(existing)=sqlx::query_as::<_,Message>("SELECT id,conversation_id,sequence,client_mutation_id,role,content,reasoning_content,content_format,status,model,token_count,search_sources,images,edited_at,created_at,updated_at FROM messages WHERE conversation_id=$1 AND client_mutation_id=$2").bind(id).bind(request.client_mutation_id).fetch_optional(&state.db).await? { return Ok(Json(existing)); }
    let mut tx = state.db.begin().await?;
    let is_first_user_message: bool = sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM messages WHERE conversation_id=$1 AND role='user' AND deleted_at IS NULL)").bind(id).fetch_one(&mut *tx).await?;
    let tokens = estimate_tokens(content) + estimate_image_tokens(&images);
    let (sequence, mut revision): (i64, i64) = sqlx::query_as("UPDATE conversations SET next_sequence=next_sequence+1,context_tokens=context_tokens+$3,revision=revision+1,updated_at=now() WHERE id=$1 AND user_id=$2 RETURNING next_sequence-1, revision").bind(id).bind(user_id).bind(tokens).fetch_one(&mut *tx).await?;
    let m:Message=sqlx::query_as("INSERT INTO messages (id,conversation_id,sequence,client_mutation_id,role,content,images,token_count,search_sources) VALUES ($1,$2,$3,$4,'user',$5,$6,$7,$8) RETURNING id,conversation_id,sequence,client_mutation_id,role,content,reasoning_content,content_format,status,model,token_count,search_sources,images,edited_at,created_at,updated_at").bind(Uuid::new_v4()).bind(id).bind(sequence).bind(request.client_mutation_id).bind(content).bind(json!(images)).bind(tokens).bind(if request.search.unwrap_or(false){json!([])}else{json!([])}).fetch_one(&mut *tx).await?;
    let mut title_changed = false;
    if is_first_user_message && c.title == "New chat" {
        let title_source = if content.is_empty() {
            "[Image]".to_string()
        } else {
            content.to_string()
        };
        let title: String = title_source
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(40)
            .collect();
        if !title.is_empty() {
            let updated: (i64,) = sqlx::query_as("UPDATE conversations SET title=$3,revision=revision+1 WHERE id=$1 AND user_id=$2 RETURNING revision").bind(id).bind(user_id).bind(&title).fetch_one(&mut *tx).await?;
            revision = updated.0;
            title_changed = true;
        }
    }
    tx.commit().await?;
    event(&state.db, user_id, "message", m.id, "created", revision).await?;
    if title_changed {
        event(&state.db, user_id, "conversation", id, "updated", revision).await?;
    }
    Ok(Json(m))
}

pub(crate) async fn recompute_context_tokens(pool: &PgPool, conversation_id: Uuid) -> Result<i32, ApiError> {
    let tokens: i32 = sqlx::query_scalar(
        "SELECT (COALESCE((SELECT token_count FROM conversation_summaries WHERE conversation_id=$1 ORDER BY ends_at_sequence DESC LIMIT 1),0) + COALESCE((SELECT SUM(token_count) FROM messages WHERE conversation_id=$1 AND deleted_at IS NULL AND sequence > COALESCE((SELECT ends_at_sequence FROM conversation_summaries WHERE conversation_id=$1 ORDER BY ends_at_sequence DESC LIMIT 1),0)),0))::int4",
    )
    .bind(conversation_id)
    .fetch_one(pool)
    .await?;
    Ok(tokens)
}

pub(crate) async fn update_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, message_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<UpdateMessage>,
) -> Result<Json<Message>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    own_conversation(&state.db, user_id, id).await?;
    let content = request.content.trim();
    if content.is_empty() {
        return Err(ApiError::bad("Message cannot be empty."));
    }
    let m:Message=sqlx::query_as("UPDATE messages SET content=$3,token_count=$4,edited_at=now() WHERE id=$1 AND conversation_id=$2 AND deleted_at IS NULL RETURNING id,conversation_id,sequence,client_mutation_id,role,content,reasoning_content,content_format,status,model,token_count,search_sources,images,edited_at,created_at,updated_at").bind(message_id).bind(id).bind(content).bind(estimate_tokens(content)).fetch_optional(&state.db).await?.ok_or_else(ApiError::not_found)?;
    let tokens = recompute_context_tokens(&state.db, id).await?;
    let (revision,): (i64,) = sqlx::query_as("UPDATE conversations SET context_tokens=$2,revision=revision+1 WHERE id=$1 AND user_id=$3 RETURNING revision").bind(id).bind(tokens).bind(user_id).fetch_one(&state.db).await?;
    clear_chain(&state.db, id).await?;
    event(
        &state.db, user_id, "message", message_id, "updated", m.sequence,
    )
    .await?;
    event(&state.db, user_id, "conversation", id, "updated", revision).await?;
    Ok(Json(m))
}

pub(crate) async fn delete_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, message_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    own_conversation(&state.db, user_id, id).await?;
    let r=sqlx::query("UPDATE messages SET deleted_at=now() WHERE id=$1 AND conversation_id=$2 AND deleted_at IS NULL").bind(message_id).bind(id).execute(&state.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    };
    let tokens = recompute_context_tokens(&state.db, id).await?;
    let (revision,): (i64,) = sqlx::query_as("UPDATE conversations SET context_tokens=$2,revision=revision+1 WHERE id=$1 AND user_id=$3 RETURNING revision").bind(id).bind(tokens).bind(user_id).fetch_one(&state.db).await?;
    clear_chain(&state.db, id).await?;
    event(&state.db, user_id, "message", message_id, "deleted", 0).await?;
    event(&state.db, user_id, "conversation", id, "updated", revision).await?;
    Ok(StatusCode::NO_CONTENT)
}
