use super::*;
use crate::agent_events::{self, AgentEventSink};
use crate::providers::{Tools, call_provider, call_provider_with_tools};
use crate::thinking::split_thinking;
use crate::web_tools::execute_web_tool;

pub(super) struct WebAgentAnswer {
    pub(super) answer: String,
    pub(super) reasoning: String,
    pub(super) sources: Vec<Value>,
}

/// Checkpoints the run so a retry can resume it. A failed checkpoint is never fatal:
/// the answer is still worth delivering, and the next turn rebuilds from the transcript.
#[allow(clippy::too_many_arguments)]
async fn checkpoint(
    state: &AppState,
    conversation_id: Uuid,
    user_message_id: Uuid,
    status: &str,
    transcript: &[Value],
    sources: &[Value],
    events: Option<&AgentEventSink>,
    round: usize,
    error_message: Option<&str>,
    stage: &str,
) {
    let history = events.map(|sink| sink.history()).unwrap_or_default();
    if let Err(checkpoint_error) = save_agent_run(
        state,
        conversation_id,
        user_message_id,
        status,
        transcript,
        sources,
        &history,
        round,
        error_message,
    )
    .await
    {
        warn!(conversation_id=%conversation_id, error=%checkpoint_error.message, stage, "could not save agent checkpoint");
    }
}

pub(super) async fn run_web_agent(
    state: &AppState,
    kind: &str,
    base: &str,
    key: &str,
    model: &str,
    mut transcript: Vec<Value>,
    temperature: Option<f32>,
    reasoning_effort: Option<&str>,
    events: Option<&AgentEventSink>,
    tool_rounds: Option<u8>,
    conversation_id: Uuid,
    user_message_id: Uuid,
    initial_sources: Vec<Value>,
) -> Result<WebAgentAnswer, ApiError> {
    transcript.insert(0, json!({"role":"system","content":"You are a web research agent. Before answering, use web_search with a concise, specific 3-12 word query derived from the user request. Never copy the full user message verbatim as the initial query. Refine the query when evidence is weak, and use open_web_page on important sources before relying on them. Do not invent tool results or citations. Give a direct answer only after gathering enough evidence, citing the supporting source URLs."}));
    let mut sources = initial_sources;
    let mut allowed_urls = Vec::new();

    let max_rounds = tool_rounds.map(usize::from);
    let mut round = 0usize;
    checkpoint(
        state,
        conversation_id,
        user_message_id,
        "running",
        &transcript,
        &sources,
        events,
        round,
        None,
        "could not save initial agent checkpoint",
    ).await;
    loop {
        if max_rounds.is_some_and(|limit| round >= limit) {
            break;
        }
        agent_events::emit(
            events,
            format!("plan-{round}"),
            "planning",
            "running",
            json!({}),
            round,
            None,
            Some("Choosing the next research step"),
        );
        let turn = match call_provider_with_tools(
            &state.http,
            kind,
            base,
            key,
            model,
            &transcript,
            temperature,
            reasoning_effort,
        )
        .await
        {
            Ok(turn) => turn,
            Err(error) if error.code == "provider_tool_unsupported" => {
                agent_events::emit(
                    events,
                    format!("plan-{round}"),
                    "planning",
                    "failed",
                    json!({}),
                    round,
                    None,
                    Some("This provider does not accept native tools"),
                );
                warn!(round, error=%error.message, "provider rejected native web tools; falling back to a normal answer");
                let reply = call_provider(
                    &state.http,
                    kind,
                    base,
                    key,
                    model,
                    &transcript,
                    temperature,
                    reasoning_effort,
                    Tools::None,
                    None,
                )
                .await?;
                let (answer, inline_reasoning) = split_thinking(&reply.text);
                return Ok(WebAgentAnswer {
                    answer,
                    reasoning: format!("{}{}", reply.reasoning, inline_reasoning),
                    sources,
                });
            }
            Err(error) => {
                agent_events::emit(
                    events,
                    format!("plan-{round}"),
                    "planning",
                    "failed",
                    json!({}),
                    round,
                    None,
                    Some(&error.message),
                );
                checkpoint(
        state,
        conversation_id,
        user_message_id,
        "failed",
        &transcript,
        &sources,
        events,
        round,
        Some(&error.message),
        "could not save failed agent checkpoint",
    ).await;
                return Err(error);
            }
        };
        agent_events::emit(
            events,
            format!("plan-{round}"),
            "planning",
            "completed",
            json!({}),
            round,
            None,
            None,
        );
        if !turn.reply.reasoning.trim().is_empty() {
            agent_events::emit_reasoning(events, &turn.reply.reasoning, round);
        }

        if turn.tool_calls.is_empty() {
            let (answer, inline_reasoning) = split_thinking(&turn.reply.text);
            let reasoning = format!("{}{}", turn.reply.reasoning, inline_reasoning);
            if !answer.trim().is_empty() {
                info!(round, source_count = sources.len(), "web agent completed");
                return Ok(WebAgentAnswer {
                    answer,
                    reasoning,
                    sources,
                });
            }
            break;
        }

        info!(
            round,
            tool_calls = turn.tool_calls.len(),
            "web agent requested tools"
        );
        transcript.push(turn.assistant_message);
        let mut anthropic_results = Vec::new();
        for (index, call) in turn.tool_calls.iter().enumerate() {
            let input = call.input.clone();
            agent_events::emit(
                events,
                &call.id,
                &call.name,
                "running",
                input.clone(),
                round,
                None,
                None,
            );
            let result = if index < 16 {
                execute_web_tool(state, call, &mut sources, &mut allowed_urls).await
            } else {
                json!({"error":"This tool-call batch exceeded the safety limit of sixteen calls. Continue using the results already returned."})
            };
            let detail = result["error"].as_str();
            let result_preview = serde_json::to_string(&result)
                .ok()
                .map(|value| value.chars().take(1200).collect::<String>());
            let source_count = result["results"].as_array().map(Vec::len);
            agent_events::emit(
                events,
                &call.id,
                &call.name,
                if detail.is_some() {
                    "failed"
                } else {
                    "completed"
                },
                input,
                round,
                source_count,
                detail.or(result_preview.as_deref()),
            );
            if kind == "anthropic" {
                anthropic_results.push(json!({"type":"tool_result","tool_use_id":call.id,"content":serde_json::to_string(&result).unwrap_or_else(|_| "{}".into())}));
            } else {
                transcript.push(json!({"role":"tool","tool_call_id":call.id,"content":serde_json::to_string(&result).unwrap_or_else(|_| "{}".into())}));
            }
        }
        if kind == "anthropic" && !anthropic_results.is_empty() {
            transcript.push(json!({"role":"user","content":anthropic_results}));
        }
        checkpoint(
            state,
            conversation_id,
            user_message_id,
            "running",
            &transcript,
            &sources,
            events,
            round + 1,
            None,
            "after tool round",
        )
        .await;
        round += 1;
    }

    agent_events::emit(
        events,
        "draft",
        "drafting",
        "running",
        json!({}),
        round,
        None,
        Some("Writing the answer from gathered evidence"),
    );
    transcript.insert(0, json!({"role":"system","content":"Finish now using the collected evidence. Clearly distinguish missing evidence from established facts, and do not call more tools."}));
    let draft = call_provider(
        &state.http,
        kind,
        base,
        key,
        model,
        &transcript,
        temperature,
        reasoning_effort,
        Tools::None,
        None,
    )
    .await;
    let draft = match draft {
        Ok(draft) => draft,
        Err(error) => {
            agent_events::emit(
                events,
                "draft",
                "drafting",
                "failed",
                json!({}),
                round,
                None,
                Some(&error.message),
            );
            checkpoint(
        state,
        conversation_id,
        user_message_id,
        "failed",
        &transcript,
        &sources,
        events,
        round,
        Some(&error.message),
        "could not save drafting failure checkpoint",
    ).await;
            return Err(error);
        }
    };
    let (answer, inline_reasoning) = split_thinking(&draft.text);
    let reasoning = format!("{}{}", draft.reasoning, inline_reasoning);
    agent_events::emit(
        events,
        "draft",
        "drafting",
        "completed",
        json!({}),
        round,
        None,
        None,
    );
    checkpoint(
        state,
        conversation_id,
        user_message_id,
        "completed",
        &transcript,
        &sources,
        events,
        round,
        None,
        "could not save completed agent checkpoint",
    ).await;
    Ok(WebAgentAnswer {
        answer,
        reasoning,
        sources,
    })
}

// --- stored runs: the checkpoint a retry resumes from ---------------------------
/// Reads the transcript and sources a failed run already collected, so a retry resumes
/// instead of searching from scratch. The columns are single `jsonb` values, so they are
/// decoded as `Value` first: sqlx reads a `Vec<Value>` as `jsonb[]` and fails the row.
pub(crate) async fn load_failed_agent_run(
    state: &AppState,
    conversation_id: Uuid,
    user_message_id: Uuid,
) -> Result<Option<(Vec<Value>, Vec<Value>, Vec<Value>, i32)>, ApiError> {
    let row: Option<(Value, Value, Value, i32)> = sqlx::query_as("SELECT transcript,sources,events,round FROM agent_runs WHERE conversation_id=$1 AND user_message_id=$2 AND status='failed' ORDER BY updated_at DESC LIMIT 1")
        .bind(conversation_id).bind(user_message_id).fetch_optional(&state.db).await.map_err(ApiError::from)?;
    Ok(row.map(|(transcript, sources, events, round)| {
        (json_items(transcript), json_items(sources), json_items(events), round)
    }))
}

fn json_items(value: Value) -> Vec<Value> {
    value.as_array().cloned().unwrap_or_default()
}

pub(crate) async fn agent_run_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation_id, message_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    own_conversation(&state.db, user_id, conversation_id).await?;
    let run: Option<(String, Value, Value, Value, i32, Option<String>)> = sqlx::query_as("SELECT status,events,sources,transcript,round,error_message FROM agent_runs WHERE conversation_id=$1 AND user_message_id=$2 ORDER BY updated_at DESC LIMIT 1")
        .bind(conversation_id).bind(message_id).fetch_optional(&state.db).await?;
    let Some((status, events, sources, transcript, round, error_message)) = run else {
        return Err(ApiError::not_found());
    };
    Ok(Json(
        json!({"status":status,"events":events,"sources":sources,"transcript":transcript,"round":round,"error_message":error_message}),
    ))
}

pub(crate) async fn save_agent_run(
    state: &AppState,
    conversation_id: Uuid,
    user_message_id: Uuid,
    status: &str,
    transcript: &[Value],
    sources: &[Value],
    events: &[Value],
    round: usize,
    error_message: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO agent_runs (id,conversation_id,user_message_id,status,transcript,sources,events,round,error_message) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT (user_message_id) WHERE status IN ('running','failed') DO UPDATE SET status=EXCLUDED.status,transcript=EXCLUDED.transcript,sources=EXCLUDED.sources,events=EXCLUDED.events,round=EXCLUDED.round,error_message=EXCLUDED.error_message,updated_at=now()")
        .bind(Uuid::new_v4()).bind(conversation_id).bind(user_message_id).bind(status).bind(json!(transcript)).bind(json!(sources)).bind(json!(events)).bind(round as i32).bind(error_message).execute(&state.db).await.map_err(|error| {
            if error.as_database_error().and_then(|db| db.code()).as_deref() == Some("23505") { ApiError::conflict("This conversation already has an active agent run.") } else { ApiError::from(error) }
        })?;
    Ok(())
}
