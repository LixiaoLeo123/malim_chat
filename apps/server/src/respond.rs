use super::*;
use crate::agent::load_failed_agent_run;

/// The turn pipeline: assemble what the provider sees, decide between the hosted
/// search, the ReAct loop and a plain answer, then store the reply.

/// The formatting instruction is re-sent per turn; it is not part of the stored chain.
pub(crate) fn instruction_message(enable_markdown: bool) -> Value {
    json!({"role":"system","content": if enable_markdown { "Format the answer with GitHub-flavored Markdown when it improves readability. Use fenced code blocks with a language label for code." } else { "Respond in plain text only. Do not use Markdown syntax, headings, lists, tables, fenced code blocks, or inline formatting markers." }})
}

pub(crate) async fn build_transcript(
    pool: &PgPool,
    conversation_id: Uuid,
    kind: &str,
    supports_images: bool,
    prior_summary: Option<&(String, i64)>,
    context_rounds: Option<u8>,
    enable_markdown: bool,
) -> Result<Vec<Value>, ApiError> {
    let rows: Vec<(String, String, Value)> = sqlx::query_as("SELECT role,content,images FROM messages WHERE conversation_id=$1 AND sequence > $2 AND deleted_at IS NULL AND status='complete' AND role <> 'summary' ORDER BY sequence DESC LIMIT 80")
        .bind(conversation_id)
        .bind(prior_summary.map(|summary| summary.1).unwrap_or(0))
        .fetch_all(pool)
        .await?;
    let mut transcript: Vec<Value> = rows
        .into_iter()
        .rev()
        .map(|(role, content, images)| {
            let images = images.as_array().map(Vec::as_slice).unwrap_or(&[]);
            json!({"role":role,"content":content_part(kind, supports_images, &strip_thinking(&content), images)})
        })
        .collect();
    if let Some(rounds) = context_rounds {
        let keep = usize::from(rounds).saturating_mul(2);
        let start = transcript.len().saturating_sub(keep);
        transcript = transcript.split_off(start);
    }
    if let Some((summary, _)) = prior_summary {
        transcript.insert(0, json!({"role":"system","content":format!("Previous conversation context, compressed by malim_chat:\n{summary}")}));
    }
    transcript.insert(0, instruction_message(enable_markdown));
    Ok(transcript)
}

/// The stored Responses chain is only usable when it ends at the message directly
/// before the one being answered; anything else means the conversation branched.
pub(crate) async fn load_chain(
    pool: &PgPool,
    conversation_id: Uuid,
    user_sequence: i64,
) -> Result<Option<String>, ApiError> {
    let chain: Option<(Option<String>, i64)> =
        sqlx::query_as("SELECT chain_response_id, chain_sequence FROM conversations WHERE id=$1")
            .bind(conversation_id)
            .fetch_optional(pool)
            .await?;
    Ok(chain
        .filter(|(_, sequence)| *sequence == user_sequence - 1)
        .and_then(|(response_id, _)| response_id))
}

pub(crate) async fn clear_chain(pool: &PgPool, conversation_id: Uuid) -> Result<(), ApiError> {
    sqlx::query("UPDATE conversations SET chain_response_id=NULL, chain_sequence=0 WHERE id=$1")
        .bind(conversation_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn respond(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(request): Json<RespondRequest>,
) -> Result<Response, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let conversation = own_conversation(&state.db, user_id, id).await?;
    let input:Message=sqlx::query_as("SELECT id,conversation_id,sequence,client_mutation_id,role,content,reasoning_content,content_format,status,model,token_count,search_sources,images,edited_at,created_at,updated_at FROM messages WHERE id=$1 AND conversation_id=$2 AND role='user' AND deleted_at IS NULL").bind(request.message_id).bind(id).fetch_optional(&state.db).await?.ok_or_else(ApiError::not_found)?;
    let provider_id = conversation
        .model_provider_id
        .ok_or_else(|| ApiError::bad("Choose a provider before requesting a response."))?;
    let p:(String,String,String,Vec<u8>,Vec<u8>)=sqlx::query_as("SELECT kind,base_url,default_model,encrypted_api_key,key_nonce FROM providers WHERE id=$1 AND user_id=$2").bind(provider_id).bind(user_id).fetch_optional(&state.db).await?.ok_or_else(||ApiError::bad("The selected provider was removed."))?;
    let prior_summary: Option<(String, i64)> = sqlx::query_as("SELECT content,ends_at_sequence FROM conversation_summaries WHERE conversation_id=$1 ORDER BY ends_at_sequence DESC LIMIT 1").bind(id).fetch_optional(&state.db).await?;
    let api_key = decrypt(&state, &p.3, &p.4)?;
    let model = match conversation
        .model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        Some(value) => value.to_string(),
        None => provider_first_model(&state.db, provider_id)
            .await?
            .unwrap_or_else(|| p.2.clone()),
    };
    let kind = provider_model_kind(&state.db, provider_id, &model)
        .await?
        .unwrap_or_else(|| p.0.clone());
    let supports_images = provider_model_supports_images(&state.db, provider_id, &model)
        .await?
        .unwrap_or(false);
    let context_rounds = request.context_rounds.unwrap_or(Some(8));
    let tool_rounds = request
        .tool_rounds
        .unwrap_or(Some(DEFAULT_WEB_TOOL_ROUNDS as u8));
    let explicit_search = web_tools::content_requests_web_search(&input.content);
    // Responses providers get OpenAI's hosted web_search_preview tool on every call, so
    // the retrieved-evidence ReAct loop never applies to them.
    let hosted_tools = kind == "openai_responses";
    let search_requested = !hosted_tools && (request.search.unwrap_or(false) || explicit_search);
    let tools = if hosted_tools {
        providers::Tools::Hosted
    } else if search_requested {
        providers::Tools::Retrieved
    } else {
        providers::Tools::None
    };
    let enable_markdown = request.enable_markdown.unwrap_or(true);
    // A Responses endpoint stores the conversation on its own side, so a chained call sends
    // only this turn. Images travel as inlined data URLs and cannot ride along in a chain.
    let chain_id = if hosted_tools
        && input.images.as_array().is_none_or(|images| images.is_empty())
        && provider_model_chain_enabled(&state.db, provider_id, &model).await?
    {
        load_chain(&state.db, id, input.sequence).await?
    } else {
        None
    };
    let mut transcript = match &chain_id {
        Some(_) => vec![
            instruction_message(enable_markdown),
            json!({"role":"user","content":content_part(&kind, supports_images, &input.content, &[])}),
        ],
        None => {
            build_transcript(
                &state.db,
                id,
                &kind,
                supports_images,
                prior_summary.as_ref(),
                context_rounds,
                enable_markdown,
            )
            .await?
        }
    };
    info!(conversation_id=%id, message_id=%input.id, provider_kind=%kind, search_toggle=request.search.unwrap_or(false), explicit_search, hosted_tools, chained=chain_id.is_some(), stream=request.stream.unwrap_or(false), "response request received");
    let resumed_sources = if search_requested {
        if let Some((saved_transcript, saved_sources, _, _)) =
            load_failed_agent_run(&state, id, input.id).await?
        {
            transcript = saved_transcript;
            saved_sources
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    if search_requested && !input.content.trim().is_empty() {
        if request.stream.unwrap_or(false) {
            return Ok(stream_web_agent_response(
                state,
                id,
                user_id,
                kind,
                p.1,
                api_key,
                model,
                transcript,
                input.id,
                request.temperature,
                request.reasoning_effort,
                enable_markdown,
                tool_rounds,
                resumed_sources.clone(),
            ));
        }
        let result = agent::run_web_agent(
            &state,
            &kind,
            &p.1,
            &api_key,
            &model,
            transcript,
            request.temperature,
            request.reasoning_effort.as_deref(),
            None,
            tool_rounds,
            id,
            input.id,
            resumed_sources,
        )
        .await?;
        let message = persist_assistant_message(
            &state,
            id,
            user_id,
            &model,
            result.answer,
            result.reasoning,
            &result.sources,
            enable_markdown,
            None,
        )
        .await?;
        return Ok(Json(message).into_response());
    }
    if request.stream.unwrap_or(false) {
        let upstream = match providers::call_provider_stream(
            &state.http,
            &kind,
            &p.1,
            &api_key,
            &model,
            &transcript,
            request.temperature,
            request.reasoning_effort.as_deref(),
            tools,
            chain_id.as_deref(),
        )
        .await
        {
            Ok(upstream) => upstream,
            Err(error) if error.code == "provider_chain_stale" => {
                warn!(conversation_id=%id, "Responses rejected the stored chain; resending the full conversation");
                clear_chain(&state.db, id).await?;
                transcript = build_transcript(&state.db, id, &kind, supports_images, prior_summary.as_ref(), context_rounds, enable_markdown).await?;
                providers::call_provider_stream(
                    &state.http,
                    &kind,
                    &p.1,
                    &api_key,
                    &model,
                    &transcript,
                    request.temperature,
                    request.reasoning_effort.as_deref(),
                    tools,
                    None,
                )
                .await?
            }
            Err(error) => return Err(error),
        };
        return Ok(stream_response(
            state,
            upstream,
            id,
            user_id,
            model,
            vec![],
            enable_markdown,
            kind,
        ));
    }
    let reply = match providers::call_provider(
        &state.http,
        &kind,
        &p.1,
        &api_key,
        &model,
        &transcript,
        request.temperature,
        request.reasoning_effort.as_deref(),
        tools,
        chain_id.as_deref(),
    )
    .await
    {
        Ok(reply) => reply,
        Err(error) if error.code == "provider_chain_stale" => {
            warn!(conversation_id=%id, "Responses rejected the stored chain; resending the full conversation");
            clear_chain(&state.db, id).await?;
            transcript = build_transcript(&state.db, id, &kind, supports_images, prior_summary.as_ref(), context_rounds, enable_markdown).await?;
            providers::call_provider(
                &state.http,
                &kind,
                &p.1,
                &api_key,
                &model,
                &transcript,
                request.temperature,
                request.reasoning_effort.as_deref(),
                tools,
                None,
            )
            .await?
        }
        Err(error) => return Err(error),
    };
    let (answer, reasoning) = split_thinking(&reply.text);
    let reasoning = format!("{}{}", reply.reasoning, reasoning);
    let m = persist_assistant_message(
        &state,
        id,
        user_id,
        &model,
        answer,
        reasoning,
        &[],
        enable_markdown,
        reply.response_id.as_deref(),
    )
    .await?;
    Ok(Json(m).into_response())
}

pub(crate) async fn persist_assistant_message(
    state: &AppState,
    conversation_id: Uuid,
    user_id: Uuid,
    model: &str,
    answer: String,
    reasoning: String,
    sources: &[Value],
    enable_markdown: bool,
    chain_response_id: Option<&str>,
) -> Result<Message, ApiError> {
    // Providers sometimes emit thinking tags in normal content. Normalize at the
    // persistence boundary so private reasoning cannot become visible later.
    let (answer, leaked_reasoning) = split_thinking(&answer);
    let reasoning = format!("{reasoning}{leaked_reasoning}");
    let tokens = estimate_tokens(&answer);
    let mut tx = state.db.begin().await?;
    let seq:(i64,)=sqlx::query_as("UPDATE conversations SET next_sequence=next_sequence+1,context_tokens=context_tokens+$3,revision=revision+1,updated_at=now() WHERE id=$1 AND user_id=$2 RETURNING next_sequence-1").bind(conversation_id).bind(user_id).bind(tokens).fetch_one(&mut *tx).await?;
    let m:Message=sqlx::query_as("INSERT INTO messages (id,conversation_id,sequence,role,content,reasoning_content,content_format,model,token_count,search_sources) VALUES ($1,$2,$3,'assistant',$4,$5,$6,$7,$8,$9) RETURNING id,conversation_id,sequence,client_mutation_id,role,content,reasoning_content,content_format,status,model,token_count,search_sources,images,edited_at,created_at,updated_at").bind(Uuid::new_v4()).bind(conversation_id).bind(seq.0).bind(answer).bind(reasoning).bind(if enable_markdown { "markdown" } else { "plain" }).bind(model).bind(tokens).bind(json!(sources)).fetch_one(&mut *tx).await?;
    if let Some(response_id) = chain_response_id {
        sqlx::query("UPDATE conversations SET chain_response_id=$2, chain_sequence=$3 WHERE id=$1")
            .bind(conversation_id)
            .bind(response_id)
            .bind(seq.0)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    event(&state.db, user_id, "message", m.id, "created", seq.0).await?;
    let _ = auto_compact(&state.db, conversation_id).await;
    Ok(m)
}
