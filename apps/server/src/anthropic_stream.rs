use super::*;
use crate::providers::{self, StreamFragment, host_tool_activity};

/// Decoding an Anthropic Messages stream.
///
/// Anthropic runs its own tools partway through a single reply, so one answer interleaves
/// text with `server_tool_use` calls and complete result blocks, and it can stop with
/// `pause_turn` — which is not a failure but a turn that is still open.

/// A continued turn can pause again, so the loop is bounded the way any retry loop is.
pub(crate) const MAX_CONTINUATIONS: usize = 6;

/// One frame of the stream, as far as the relay cares. The text and thinking deltas are the
/// answer; everything else is a hosted tool doing its work, which the caller shows in the
/// timeline because the stream goes quiet while a search runs and quiet reads as a stall.
pub(crate) fn fragment(value: &Value) -> Option<StreamFragment> {
    if let Some(text) = value["delta"]["text"].as_str() {
        return Some(StreamFragment {
            text: text.to_string(),
            ..Default::default()
        });
    }
    if let Some(thinking) = value["delta"]["thinking"].as_str() {
        return Some(StreamFragment {
            reasoning: true,
            text: thinking.to_string(),
            ..Default::default()
        });
    }
    match value["type"].as_str()? {
        "content_block_start" => {
            let block = &value["content_block"];
            Some(StreamFragment {
                activity: host_tool_activity(block),
                sources: providers::hosted_sources(std::slice::from_ref(block)),
                ..Default::default()
            })
        }
        // Only a pause is news; `end_turn` and `tool_use` frames carry nothing the relay
        // does not already know from the blocks it has seen.
        "message_delta" => (value["delta"]["stop_reason"].as_str() == Some("pause_turn"))
            .then(|| StreamFragment {
                paused: true,
                ..Default::default()
            }),
        _ => None,
    }
}

/// Rebuilds an assistant message from the frames we are relaying, because continuing a
/// paused turn means sending the paused content back unchanged and the stream is the only
/// copy of it we hold. Blocks keep the fields Anthropic gave them; a `server_tool_use`'s
/// input is the exception, assembled from its `input_json_delta` frames.
#[derive(Default)]
pub(crate) struct AnthropicMessage {
    blocks: Vec<Value>,
    inputs: Vec<String>,
    stop_reason: Option<String>,
}

impl AnthropicMessage {
    pub(crate) fn apply(&mut self, frame: &str) {
        let Some(payload) = frame.lines().find_map(|line| line.strip_prefix("data:")) else {
            return;
        };
        let Ok(value) = serde_json::from_str::<Value>(payload.trim()) else {
            return;
        };
        match value["type"].as_str() {
            Some("content_block_start") => {
                let index = value["index"].as_u64().unwrap_or_default() as usize;
                self.grow(index);
                self.blocks[index] = value["content_block"].clone();
            }
            Some("content_block_delta") => {
                let index = value["index"].as_u64().unwrap_or_default() as usize;
                match value["delta"]["type"].as_str() {
                    Some("text_delta") => self.append(index, "text", &value["delta"]["text"]),
                    Some("thinking_delta") => {
                        self.append(index, "thinking", &value["delta"]["thinking"])
                    }
                    Some("signature_delta") => {
                        self.append(index, "signature", &value["delta"]["signature"])
                    }
                    Some("input_json_delta") => {
                        if let Some(partial) = value["delta"]["partial_json"].as_str() {
                            self.grow(index);
                            self.inputs[index].push_str(partial);
                        }
                    }
                    _ => {}
                }
            }
            Some("message_delta") => {
                self.stop_reason = value["delta"]["stop_reason"].as_str().map(str::to_string);
            }
            _ => {}
        }
    }

    /// The message as it arrived, ready to append to the transcript. `message_start` and
    /// `message_delta` never reserve a block, so slots left empty by a frame we ignored are
    /// dropped instead of sent back as nulls.
    pub(crate) fn content(&mut self) -> Vec<Value> {
        for (index, input) in std::mem::take(&mut self.inputs).into_iter().enumerate() {
            if input.is_empty() || self.blocks.get(index).is_none_or(|block| block.is_null()) {
                continue;
            }
            if self.blocks[index]["type"] == "server_tool_use" {
                if let Ok(parsed) = serde_json::from_str::<Value>(&input) {
                    self.blocks[index]["input"] = parsed;
                }
            }
        }
        self.blocks
            .iter()
            .filter(|block| !block.is_null())
            .cloned()
            .collect()
    }

    /// Whether Anthropic stopped mid-turn: the tool loop it runs for us is still open.
    pub(crate) fn paused(&self) -> bool {
        self.stop_reason.as_deref() == Some("pause_turn")
    }

    fn grow(&mut self, index: usize) {
        while self.blocks.len() <= index {
            self.blocks.push(Value::Null);
            self.inputs.push(String::new());
        }
    }

    /// Deltas arrive as pieces of one string, and `content_block_start` already carries the
    /// empty block, so this only ever extends a field that exists.
    fn append(&mut self, index: usize, field: &str, value: &Value) {
        let Some(more) = value.as_str() else { return };
        let Some(block) = self.blocks.get_mut(index) else {
            return;
        };
        if !block.is_object() {
            return;
        }
        let mut merged = block
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        merged.push_str(more);
        block[field] = Value::String(merged);
    }
}
