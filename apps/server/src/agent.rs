use super::*;
use crate::providers::{call_provider, call_provider_with_tools};
use crate::web_tools::execute_web_tool;
use tokio::sync::mpsc::UnboundedSender;

pub(super) struct WebAgentAnswer {
    pub(super) answer: String,
    pub(super) reasoning: String,
    pub(super) sources: Vec<Value>,
}

fn emit_tool_event(
    events: Option<&UnboundedSender<Value>>,
    id: impl Into<String>,
    name: &str,
    status: &str,
    input: Value,
    round: usize,
    source_count: Option<usize>,
    detail: Option<&str>,
) {
    let Some(events) = events else {
        return;
    };
    let mut event = json!({"id":id.into(),"name":name,"status":status,"input":input,"round":round});
    if let Some(source_count) = source_count {
        event["source_count"] = json!(source_count);
    }
    if let Some(detail) = detail {
        event["detail"] = json!(detail);
    }
    let _ = events.send(event);
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
    events: Option<&UnboundedSender<Value>>,
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
    let _ = save_agent_run(
        state,
        conversation_id,
        user_message_id,
        "running",
        &transcript,
        &sources,
        &[],
        round,
        None,
    )
    .await;
    loop {
        if max_rounds.is_some_and(|limit| round >= limit) {
            break;
        }
        emit_tool_event(
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
                emit_tool_event(
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
                let (answer, reasoning) = split_thinking(
                    &call_provider(
                        &state.http,
                        kind,
                        base,
                        key,
                        model,
                        &transcript,
                        temperature,
                        reasoning_effort,
                    )
                    .await?,
                );
                return Ok(WebAgentAnswer {
                    answer,
                    reasoning,
                    sources,
                });
            }
            Err(error) => {
                emit_tool_event(
                    events,
                    format!("plan-{round}"),
                    "planning",
                    "failed",
                    json!({}),
                    round,
                    None,
                    Some(&error.message),
                );
                let _ = save_agent_run(
                    state,
                    conversation_id,
                    user_message_id,
                    "failed",
                    &transcript,
                    &sources,
                    &[],
                    round,
                    Some(&error.message),
                )
                .await;
                return Err(error);
            }
        };
        emit_tool_event(
            events,
            format!("plan-{round}"),
            "planning",
            "completed",
            json!({}),
            round,
            None,
            None,
        );
        let (_, turn_reasoning) = split_thinking(&turn.content);
        if !turn_reasoning.trim().is_empty() {
            if let Some(events) = events {
                let _ =
                    events.send(json!({"type":"reasoning","delta":turn_reasoning,"round":round}));
            }
        }

        if turn.tool_calls.is_empty() {
            let (answer, reasoning) = split_thinking(&turn.content);
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
            emit_tool_event(
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
            emit_tool_event(
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
        let _ = save_agent_run(
            state,
            conversation_id,
            user_message_id,
            "running",
            &transcript,
            &sources,
            &[],
            round + 1,
            None,
        )
        .await;
        round += 1;
    }

    emit_tool_event(
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
    let (answer, reasoning) = split_thinking(
        &call_provider(
            &state.http,
            kind,
            base,
            key,
            model,
            &transcript,
            temperature,
            reasoning_effort,
        )
        .await?,
    );
    emit_tool_event(
        events,
        "draft",
        "drafting",
        "completed",
        json!({}),
        round,
        None,
        None,
    );
    let _ = save_agent_run(
        state,
        conversation_id,
        user_message_id,
        "completed",
        &transcript,
        &sources,
        &[],
        round,
        None,
    )
    .await;
    Ok(WebAgentAnswer {
        answer,
        reasoning,
        sources,
    })
}
