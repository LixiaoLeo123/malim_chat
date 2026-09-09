use super::*;
use crate::respond::persist_assistant_message;

/// Server-sent events for the two streaming modes: relaying a provider stream, and
/// driving the ReAct agent while its tool events are forwarded live.

pub(crate) fn stream_response(
    state: AppState,
    upstream: reqwest::Response,
    conversation_id: Uuid,
    user_id: Uuid,
    model: String,
    sources: Vec<Value>,
    enable_markdown: bool,
    kind: String,
) -> Response {
    let output = async_stream::stream! {
        let mut answer = String::new(); let mut reasoning = String::new(); let mut buffer = String::new(); let mut upstream = upstream.bytes_stream(); let mut thinking = ThinkingStream::new(); let mut reasoning_part: Option<String> = None; let mut reasoning_row = 0_u32; let mut provider_response_id: Option<String> = None;
        while let Some(chunk) = upstream.next().await {
            match chunk {
                Ok(chunk) => { buffer.push_str(&String::from_utf8_lossy(&chunk)); buffer = buffer.replace("\r\n", "\n"); while let Some(boundary) = buffer.find("\n\n") { let frame = buffer[..boundary].to_string(); buffer.drain(..boundary + 2); if let Some(fragment) = providers::provider_stream_delta(&kind, &frame) { let part = fragment.part; if fragment.response_id.is_some() { provider_response_id = fragment.response_id; } let fragments = if fragment.reasoning { vec![(true, fragment.text)] } else { thinking.push(&fragment.text) }; for (is_reasoning, text) in fragments { if text.is_empty() { continue; } if is_reasoning { if let Some(part) = &part { if reasoning_part.as_deref() != Some(part.as_str()) { reasoning_row += 1; if !reasoning.is_empty() { reasoning.push_str("\n\n"); } reasoning_part = Some(part.clone()); } } reasoning.push_str(&text); } else { answer.push_str(&text); } let mut payload = json!({"type":if is_reasoning { "reasoning" } else { "delta" },"delta":text}); if is_reasoning && part.is_some() { payload["round"] = json!(reasoning_row); } let payload = serde_json::to_string(&payload).unwrap_or_default(); yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(format!("data: {payload}\n\n"))); } } } }
                Err(error) => { warn!(conversation_id=%conversation_id, %error, "upstream stream interrupted"); let payload = serde_json::to_string(&json!({"type":"error","message":"The provider stream was interrupted."})).unwrap_or_default(); yield Ok(Bytes::from(format!("data: {payload}\n\n"))); return; }
            }
        }
        for (is_reasoning, text) in thinking.finish() { if is_reasoning { reasoning.push_str(&text); } else { answer.push_str(&text); } let payload = serde_json::to_string(&json!({"type":if is_reasoning { "reasoning" } else { "delta" },"delta":text})).unwrap_or_default(); yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(format!("data: {payload}\n\n"))); }
        if answer.trim().is_empty() { let payload = serde_json::to_string(&json!({"type":"error","message":"The provider returned no final answer."})).unwrap_or_default(); yield Ok(Bytes::from(format!("data: {payload}\n\n"))); return; }
        match persist_assistant_message(&state, conversation_id, user_id, &model, answer, reasoning, &sources, enable_markdown, provider_response_id.as_deref()).await {
            Ok(message) => { let payload = serde_json::to_string(&json!({"type":"done","message":message})).unwrap_or_default(); yield Ok(Bytes::from(format!("data: {payload}\n\n"))); }
            Err(error) => { error!(conversation_id=%conversation_id, error=%error.message, "could not persist streamed response"); let payload = serde_json::to_string(&json!({"type":"error","message":"The streamed response could not be saved."})).unwrap_or_default(); yield Ok(Bytes::from(format!("data: {payload}\n\n"))); }
        }
    };
    let mut response = Body::from_stream(output).into_response();
    response.headers_mut().insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response.headers_mut().insert(
        http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

pub(crate) fn stream_web_agent_response(
    state: AppState,
    conversation_id: Uuid,
    user_id: Uuid,
    kind: String,
    base: String,
    api_key: String,
    model: String,
    transcript: Vec<Value>,
    user_message_id: Uuid,
    temperature: Option<f32>,
    reasoning_effort: Option<String>,
    enable_markdown: bool,
    tool_rounds: Option<u8>,
    initial_sources: Vec<Value>,
) -> Response {
    let output = async_stream::stream! {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
        let event_sink = agent_events::AgentEventSink::new(event_tx);
        let worker_state = state.clone();
        let worker_model = model.clone();
        tokio::spawn(async move {
            let result = agent::run_web_agent(&worker_state, &kind, &base, &api_key, &worker_model, transcript, temperature, reasoning_effort.as_deref(), Some(&event_sink), tool_rounds, conversation_id, user_message_id, initial_sources).await;
            match result {
                Ok(result) if !result.answer.trim().is_empty() => {
                    if !result.reasoning.trim().is_empty() { let _ = event_sink.send(json!({"type":"reasoning","delta":result.reasoning})); }
                    for chunk in result.answer.as_bytes().chunks(1200) {
                        let _ = event_sink.send(json!({"type":"delta","delta":String::from_utf8_lossy(chunk)}));
                    }
                    if let Ok(message) = persist_assistant_message(&worker_state, conversation_id, user_id, &worker_model, result.answer, result.reasoning, &result.sources, enable_markdown, None).await {
                        let _ = event_sink.send(json!({"type":"done","message":message}));
                    } else { let _ = event_sink.send(json!({"type":"error","message":"The response could not be saved."})); }
                }
                Ok(_) => { let _ = event_sink.send(json!({"type":"error","message":"The provider returned no final answer."})); }
                Err(error) => { error!(conversation_id=%conversation_id, error=%error.message, "web agent response failed"); let _ = event_sink.send(json!({"type":"error","message":error.message})); }
            }
        });
        while let Some(event) = event_rx.recv().await {
            let event_type = event["type"].as_str().unwrap_or("tool");
            if event_type == "done" || event_type == "error" {
                let payload = serde_json::to_string(&event).unwrap_or_default();
                yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(format!("data: {payload}\n\n")));
                break;
            }
            let payload = if event_type == "reasoning" { serde_json::to_string(&event).unwrap_or_default() } else { serde_json::to_string(&json!({"type":"tool","tool":event})).unwrap_or_default() };
            yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(format!("data: {payload}\n\n")));
        }
    };
    let mut response = Body::from_stream(output).into_response();
    response.headers_mut().insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response.headers_mut().insert(
        http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}
