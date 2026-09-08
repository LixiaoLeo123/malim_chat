use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

pub(super) struct AgentEventSink {
    sender: UnboundedSender<Value>,
    history: Mutex<Vec<Value>>,
}

impl AgentEventSink {
    pub(super) fn new(sender: UnboundedSender<Value>) -> Arc<Self> {
        Arc::new(Self {
            sender,
            history: Mutex::new(Vec::new()),
        })
    }
    pub(super) fn send(&self, event: Value) {
        if let Ok(mut history) = self.history.lock() {
            history.push(event.clone());
        }
        let _ = self.sender.send(event);
    }
    pub(super) fn history(&self) -> Vec<Value> {
        self.history
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default()
    }
}

pub(super) fn emit(
    sink: Option<&AgentEventSink>,
    id: impl Into<String>,
    name: &str,
    status: &str,
    input: Value,
    round: usize,
    source_count: Option<usize>,
    detail: Option<&str>,
) {
    let Some(sink) = sink else { return };
    let mut event = json!({
        "type": "tool",
        "id": id.into(),
        "name": name,
        "status": status,
        "input": input,
        "round": round
    });
    if let Some(source_count) = source_count {
        event["source_count"] = json!(source_count);
    }
    if let Some(detail) = detail {
        event["detail"] = json!(detail);
    }
    sink.send(event);
}

pub(super) fn emit_reasoning(sink: Option<&AgentEventSink>, text: &str, round: usize) {
    if text.trim().is_empty() {
        return;
    }
    if let Some(sink) = sink {
        sink.send(json!({"type":"reasoning","delta":text,"round":round}));
    }
}
