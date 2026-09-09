use super::*;
use crate::web_tools::web_tool_definitions;

/// Which wire dialect a provider endpoint speaks. Parsed once per call so the request
/// builder has a single place to branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestKind {
    OpenAiChat,
    OpenAiResponses,
    Anthropic,
}

impl RequestKind {
    pub(crate) fn parse(kind: &str) -> Self {
        match kind {
            "anthropic" => Self::Anthropic,
            "openai_responses" => Self::OpenAiResponses,
            _ => Self::OpenAiChat,
        }
    }
}

/// Who does the searching: the ReAct loop hands the model our SearXNG functions, while
/// Responses providers run OpenAI's hosted search on their side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tools {
    None,
    Retrieved,
    Hosted,
}

/// A completed non-streaming turn. Reasoning is carried separately instead of being
/// smuggled through the answer inside `<think>` markers.
#[derive(Debug, Default)]
pub(crate) struct Reply {
    pub(crate) text: String,
    pub(crate) reasoning: String,
    /// Responses ids can be referenced by the next turn; other dialects return none.
    pub(crate) response_id: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ToolCall {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) input: Value,
}

pub(crate) struct ToolTurn {
    pub(crate) reply: Reply,
    pub(crate) assistant_message: Value,
    pub(crate) tool_calls: Vec<ToolCall>,
}

pub(crate) async fn call_provider_with_tools(
    http: &Client,
    kind: &str,
    base: &str,
    key: &str,
    model: &str,
    messages: &[Value],
    temperature: Option<f32>,
    reasoning_effort: Option<&str>,
) -> Result<ToolTurn, ApiError> {
    let request = build_request(
        kind,
        base,
        model,
        messages,
        temperature,
        reasoning_effort,
        Tools::Retrieved,
        false,
        None,
    );
    let body: Value = send(http, &request, key, true, false)
        .await?
        .json()
        .await?;
    if request.anthropic {
        let blocks = body["content"]
            .as_array()
            .cloned()
            .ok_or_else(|| ApiError::bad("The provider returned an unexpected tool response."))?;
        let tool_calls = blocks
            .iter()
            .filter_map(|block| {
                Some(ToolCall {
                    id: block["id"].as_str()?.to_string(),
                    name: block["name"].as_str()?.to_string(),
                    input: block["input"].clone(),
                })
            })
            .collect();
        let (text, reasoning) = anthropic_parts(&body);
        return Ok(ToolTurn {
            reply: Reply {
                text,
                reasoning,
                response_id: None,
            },
            assistant_message: json!({ "role": "assistant", "content": blocks }),
            tool_calls,
        });
    }
    let message = body["choices"]
        .as_array()
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .cloned()
        .filter(|message| message.is_object())
        .unwrap_or_else(|| json!({ "role": "assistant" }));
    let tool_calls = message["tool_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|call| {
            let arguments = call["function"]["arguments"].as_str().unwrap_or("{}");
            Some(ToolCall {
                id: call["id"].as_str()?.to_string(),
                name: call["function"]["name"].as_str()?.to_string(),
                input: serde_json::from_str(arguments).unwrap_or_else(|_| json!({})),
            })
        })
        .collect();
    let (text, reasoning) = openai_message_parts(&message);
    Ok(ToolTurn {
        reply: Reply {
            text,
            reasoning,
            response_id: None,
        },
        assistant_message: message,
        tool_calls,
    })
}

pub(crate) async fn call_provider(
    http: &Client,
    kind: &str,
    base: &str,
    key: &str,
    model: &str,
    messages: &[Value],
    temperature: Option<f32>,
    reasoning_effort: Option<&str>,
    tools: Tools,
    chain: Option<&str>,
) -> Result<Reply, ApiError> {
    let request = build_request(
        kind,
        base,
        model,
        messages,
        temperature,
        reasoning_effort,
        tools,
        false,
        chain,
    );
    let body: Value = send(http, &request, key, false, chain.is_some())
        .await?
        .json()
        .await?;
    Ok(match RequestKind::parse(kind) {
        RequestKind::OpenAiResponses => Reply {
            text: responses_content(&body).ok_or_else(|| {
                ApiError::bad("The Responses provider returned no text output.")
            })?,
            reasoning: responses_reasoning(&body).unwrap_or_default(),
            response_id: body["id"].as_str().map(str::to_string),
        },
        RequestKind::Anthropic => {
            let (text, reasoning) = anthropic_parts(&body);
            if text.is_empty() && reasoning.is_empty() {
                return Err(invalid_provider_response());
            }
            Reply {
                text,
                reasoning,
                response_id: None,
            }
        }
        RequestKind::OpenAiChat => {
            let (text, reasoning) = openai_message_parts(
                body["choices"]
                    .as_array()
                    .and_then(|choices| choices.first())
                    .and_then(|choice| choice.get("message"))
                    .unwrap_or(&Value::Null),
            );
            if text.is_empty() && reasoning.is_empty() {
                return Err(invalid_provider_response());
            }
            Reply {
                text,
                reasoning,
                response_id: None,
            }
        }
    })
}

pub(crate) async fn call_provider_stream(
    http: &Client,
    kind: &str,
    base: &str,
    key: &str,
    model: &str,
    messages: &[Value],
    temperature: Option<f32>,
    reasoning_effort: Option<&str>,
    tools: Tools,
    chain: Option<&str>,
) -> Result<reqwest::Response, ApiError> {
    let request = build_request(
        kind,
        base,
        model,
        messages,
        temperature,
        reasoning_effort,
        tools,
        true,
        chain,
    );
    send(http, &request, key, false, chain.is_some()).await
}

/// A provider request, already shaped for its dialect.
struct Outbound {
    url: String,
    body: Value,
    anthropic: bool,
}

fn build_request(
    kind: &str,
    base: &str,
    model: &str,
    messages: &[Value],
    temperature: Option<f32>,
    reasoning_effort: Option<&str>,
    tools: Tools,
    stream: bool,
    chain: Option<&str>,
) -> Outbound {
    let kind = RequestKind::parse(kind);
    let url = provider_url(kind, base);
    let mut body = json!({ "model": model });
    if let Some(value) = temperature {
        body["temperature"] = json!(value.clamp(0.0, 2.0));
    }
    match kind {
        RequestKind::OpenAiResponses => {
            body["input"] = json!(responses_input(messages));
            if let Some(id) = chain {
                body["previous_response_id"] = json!(id);
            }
            if tools == Tools::Hosted {
                body["tools"] = json!(responses_tools());
            }
            if let Some(value) = reasoning_effort {
                // Responses exposes no raw chain of thought; the summary is all there is.
                body["reasoning"] = json!({"effort": value, "summary": "auto"});
            }
            if stream {
                body["stream"] = json!(true);
            }
            // Logged per turn: when the same model answers over Chat but fails here, the
            // difference between an accepted and a rejected request is exactly this line.
            info!(model, shape = %responses_shape(&body), "outbound Responses request");
            Outbound {
                url,
                body,
                anthropic: false,
            }
        }
        RequestKind::Anthropic => {
            let system = messages
                .iter()
                .filter(|message| message["role"] == "system")
                .filter_map(|message| message["content"].as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            body["max_tokens"] = json!(8192);
            body["messages"] = json!(messages
                .iter()
                .filter(|message| message["role"] != "system")
                .collect::<Vec<_>>());
            if !system.is_empty() {
                body["system"] = json!(system);
            }
            if tools == Tools::Retrieved {
                body["tools"] = json!(web_tool_definitions("anthropic"));
            }
            if stream {
                body["stream"] = json!(true);
            }
            Outbound {
                url,
                body,
                anthropic: true,
            }
        }
        RequestKind::OpenAiChat => {
            body["messages"] = json!(messages);
            body["stream"] = json!(stream);
            if tools == Tools::Retrieved {
                body["tools"] = json!(web_tool_definitions("openai_compatible"));
            }
            if let Some(value) = reasoning_effort {
                body["reasoning_effort"] = json!(value);
            }
            Outbound {
                url,
                body,
                anthropic: false,
            }
        }
    }
}

async fn send(
    http: &Client,
    request: &Outbound,
    key: &str,
    tool_request: bool,
    chained: bool,
) -> Result<reqwest::Response, ApiError> {
    let response = if request.anthropic {
        http.post(&request.url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .json(&request.body)
            .send()
            .await?
    } else {
        http.post(&request.url)
            .bearer_auth(key)
            .json(&request.body)
            .send()
            .await?
    };
    ensure_provider_success(response, tool_request, chained).await
}

fn invalid_provider_response() -> ApiError {
    ApiError {
        status: StatusCode::BAD_GATEWAY,
        code: "invalid_provider_response",
        message: "The AI provider returned an unexpected response.".into(),
    }
}

async fn ensure_provider_success(
    response: reqwest::Response,
    tool_request: bool,
    chained: bool,
) -> Result<reqwest::Response, ApiError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let body_hint = body.chars().take(400).collect::<String>();
    let excerpt = body_hint.clone();
    warn!(upstream_status=%status, tool_request, chained, error_body=%excerpt, "AI provider returned an error response");
    Err(provider_error_from_response(
        status,
        &body_hint,
        tool_request,
        chained,
    ))
}

/// The provider's own words, so a failure says what actually happened instead of
/// "could not be reached". OpenAI-compatible gateways all report under `error.message`.
fn upstream_detail(body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        return String::new();
    }
    let candidate = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            let error = value.get("error");
            error
                .and_then(|error| error.get("message"))
                .or_else(|| error.and_then(|error| error.get("err_msg")))
                .or_else(|| error.filter(|error| error.is_string()))
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.to_string());
    let flattened = candidate.chars().filter(|c| !c.is_control()).collect::<String>();
    let mut detail = flattened.trim().chars().take(240).collect::<String>();
    if !detail.is_empty() {
        if let Some(last) = detail.pop() {
            if last == ',' || last == ':' {
                detail.push(':');
            } else {
                detail.push(last);
            }
        }
    }
    detail
}

pub(crate) fn provider_error_from_response(
    status: StatusCode,
    body: &str,
    tool_request: bool,
    chained: bool,
) -> ApiError {
    let detail = upstream_detail(body);
    let mut error = provider_error_kind(status, &detail, tool_request, chained);
    if detail.is_empty() {
        return error;
    }
    error.message = format!("{} {}", error.message, detail);
    error
}

fn provider_error_kind(
    status: StatusCode,
    body: &str,
    tool_request: bool,
    chained: bool,
) -> ApiError {
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return ApiError::provider_access_denied();
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return ApiError::provider_rate_limited();
    }
    let normalized = body.to_ascii_lowercase();
    if tool_request
        && [
            "tool_choice",
            "tool calling",
            "function calling",
            "function_call",
            "functions are not supported",
            "tools are not supported",
            "does not support tools",
            "does not support function",
        ]
        .iter()
        .any(|hint| normalized.contains(hint))
    {
        return ApiError::provider_tool_unsupported();
    }
    // A rejected `previous_response_id` is recoverable: the caller rebuilds the context
    // from the transcript and starts a new chain. Providers word this differently, so
    // any client error on a chained request is treated as a broken chain.
    if chained && status.is_client_error() {
        return ApiError::provider_chain_stale();
    }
    if status.is_client_error() {
        return ApiError::provider_rejected();
    }
    ApiError::provider_unavailable()
}

pub(crate) fn provider_url(kind: RequestKind, base: &str) -> String {
    let base = base.trim_end_matches('/');
    match kind {
        RequestKind::Anthropic if base.ends_with("/v1/messages") => base.to_string(),
        RequestKind::Anthropic if base.ends_with("/v1") => format!("{base}/messages"),
        RequestKind::Anthropic => format!("{base}/v1/messages"),
        RequestKind::OpenAiResponses if base.ends_with("/responses") => base.to_string(),
        RequestKind::OpenAiResponses if base.ends_with("/v1") => format!("{base}/responses"),
        RequestKind::OpenAiResponses => format!("{base}/v1/responses"),
        RequestKind::OpenAiChat if base.ends_with("/chat/completions") => base.to_string(),
        RequestKind::OpenAiChat if base.ends_with("/v1") => format!("{base}/chat/completions"),
        RequestKind::OpenAiChat => format!("{base}/v1/chat/completions"),
    }
}

/// Responses never runs the ReAct loop (it searches on its own side), so the transcript it
/// receives is plain role/content turns.
/// A description of a Responses body that keeps the transcript out of the log: field names,
/// hosted tool types, the reasoning knob, and the role and shape of each input item.
fn responses_shape(body: &Value) -> String {
    let mut fields: Vec<&str> = body
        .as_object()
        .map(|object| object.keys().map(String::as_str).collect())
        .unwrap_or_default();
    fields.sort_unstable();
    let tools = body["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool["type"].as_str())
                .collect::<Vec<_>>()
                .join("+")
        })
        .unwrap_or_else(|| "-".into());
    let input = body["input"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    format!(
                        "{}:{}",
                        item["role"].as_str().unwrap_or("?"),
                        if item["content"].is_string() {
                            "str"
                        } else {
                            "parts"
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let reasoning = if body["reasoning"].is_null() {
        "-".to_string()
    } else {
        body["reasoning"].to_string()
    };
    format!(
        "fields=[{}] tools=[{tools}] reasoning={reasoning} chain={} input=[{input}]",
        fields.join(","),
        body["previous_response_id"].as_str().unwrap_or("-"),
    )
}

fn responses_input(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .filter_map(|message| {
            let role = message["role"].as_str()?;
            Some(json!({ "role": role, "content": message["content"].clone() }))
        })
        .collect()
}

/// Hosted tools the Responses API runs on its own side: no call ever comes back to us, so
/// these need no executor. Chat Completions and Anthropic providers have no equivalent,
/// which is why those go through the ReAct loop with our SearXNG functions instead.
///
/// `web_search_preview` rather than the newer `web_search` spelling: that keeps the tool
/// name the gateway is known to accept, so a failure here points at `code_interpreter`.
fn responses_tools() -> Vec<Value> {
    vec![
        json!({ "type": "web_search_preview" }),
        json!({ "type": "code_interpreter", "container": { "type": "auto" } }),
    ]
}

fn responses_content(body: &Value) -> Option<String> {
    if let Some(text) = body["output_text"].as_str() {
        return Some(text.to_string());
    }
    body["output"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|item| item["type"] != "reasoning")
                .filter_map(|item| item["content"].as_array())
                .flatten()
                .filter_map(|part| part["text"].as_str())
                .collect::<String>()
        })
        .filter(|text: &String| !text.is_empty())
}

/// Reasoning summaries sit in `output[]` items of type `reasoning`, one entry per part.
fn responses_reasoning(body: &Value) -> Option<String> {
    let parts = body["output"].as_array()?.iter().filter(|item| item["type"] == "reasoning").flat_map(|item| item["summary"].as_array().cloned().unwrap_or_default()).filter_map(|part| part["text"].as_str().map(str::to_string)).filter(|text| !text.trim().is_empty()).collect::<Vec<String>>();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// Splits an Anthropic reply into its answer and any thinking blocks.
fn anthropic_parts(body: &Value) -> (String, String) {
    let blocks = body["content"].as_array().cloned().unwrap_or_default();
    let text = blocks.iter().filter(|block| block["type"] == "text").filter_map(|block| block["text"].as_str()).collect::<String>();
    let reasoning = blocks.iter().filter(|block| block["type"] == "thinking").filter_map(|block| block["thinking"].as_str()).collect::<Vec<_>>().join("\n\n");
    (text, reasoning)
}

/// Chat Completions reasoning models report their thinking in `reasoning_content`
/// (or `reasoning`), separate from the answer.
fn openai_message_parts(message: &Value) -> (String, String) {
    let mut reasoning = String::new();
    for field in ["reasoning_content", "reasoning"] {
        if let Some(value) = message[field].as_str() {
            if !reasoning.is_empty() {
                reasoning.push('\n');
            }
            reasoning.push_str(value);
        }
    }
    (
        message["content"].as_str().unwrap_or_default().to_string(),
        reasoning,
    )
}

/// One decoded frame from a provider stream.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct StreamFragment {
    pub(crate) reasoning: bool,
    pub(crate) text: String,
    /// Responses returns reasoning as summary parts, so each part can be rendered as
    /// its own timeline row instead of being glued to its neighbours.
    pub(crate) part: Option<String>,
    /// Responses ids arrive on the lifecycle frames, not the text deltas.
    pub(crate) response_id: Option<String>,
    /// Gateways report failures inside the stream body while still answering 200.
    pub(crate) error: Option<String>,
    /// Why a response ended without producing text: token limit, filtering, and so on.
    pub(crate) incomplete: Option<String>,
}

pub(crate) fn provider_stream_delta(kind: &str, frame: &str) -> Option<StreamFragment> {
    let payload = frame
        .lines()
        .find_map(|line| line.strip_prefix("data:"))?
        .trim();
    if payload == "[DONE]" {
        return None;
    }
    let value: Value = serde_json::from_str(payload).ok()?;
    let fragment = |error: Option<String>, incomplete: Option<String>| StreamFragment {
        reasoning: false,
        text: String::new(),
        part: None,
        response_id: None,
        error,
        incomplete,
    };
    // A gateway can fail the turn inside the stream while still answering 200: a bare
    // `error`, a nested `response.error`, or a `response` whose status is `failed`. A JSON
    // `null` counts as absent, because every lifecycle frame carries `"error": null` and
    // reading that as a failure aborts the stream before a single token arrives.
    let failure = value
        .get("error")
        .or_else(|| value.get("response").and_then(|response| response.get("error")))
        .filter(|error| !error.is_null());
    if failure.is_some() || value["response"]["status"].as_str() == Some("failed") {
        let detail = failure
            .map(|error| match error.as_str() {
                Some(text) => text.to_string(),
                None => upstream_detail(&error.to_string()),
            })
            .filter(|detail| !detail.is_empty())
            .unwrap_or_else(|| "the provider marked the response as failed".into());
        return Some(fragment(Some(detail), None));
    }
    if let Some(reason) = value["response"]["incomplete_details"]["reason"]
        .as_str()
        .or_else(|| value["incomplete_details"]["reason"].as_str())
    {
        return Some(fragment(None, Some(reason.to_string())));
    }
    let kind = RequestKind::parse(kind);
    if kind == RequestKind::OpenAiResponses {
        let response_id = value["response"]["id"]
            .as_str()
            .or_else(|| value["id"].as_str().filter(|_| value["type"].as_str() == Some("response.created")))
            .map(str::to_string);
        let event_type = value["type"].as_str()?;
        let fragment = match event_type {
            "response.output_text.delta" => StreamFragment {
                reasoning: false,
                text: value["delta"].as_str()?.to_string(),
                part: None,
                response_id,
                error: None,                incomplete: None,
            },
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                StreamFragment {
                    reasoning: true,
                    text: value["delta"].as_str()?.to_string(),
                    // `summary_index` restarts at zero for every reasoning item, so a
                    // response with several items would otherwise collide into one row.
                    part: value["output_index"].as_u64().map(|output_index| {
                        let index = value["summary_index"]
                            .as_u64()
                            .or_else(|| value["content_index"].as_u64())
                            .unwrap_or_default();
                        format!("{output_index}:{index}")
                    }),
                    response_id,
                    error: None,                incomplete: None,
                }
            }
            _ => StreamFragment {
                reasoning: false,
                text: String::new(),
                part: None,
                response_id,
                error: None,                incomplete: None,
            },
        };
        if fragment.text.is_empty() && fragment.response_id.is_none() {
            return None;
        }
        return Some(fragment);
    }
    if kind == RequestKind::Anthropic {
        return value["delta"]["text"].as_str().map(|text| StreamFragment {
            reasoning: false,
            text: text.to_string(),
            part: None,
            response_id: None,
            error: None,                incomplete: None,
        }).or_else(|| {
            value["delta"]["thinking"].as_str().map(|text| StreamFragment {
                reasoning: true,
                text: text.to_string(),
                part: None,
                response_id: None,
                error: None,                incomplete: None,
            })
        });
    }
    let delta = &value["choices"].as_array()?.first()?["delta"];
    let text = |reasoning: bool, value: &str| StreamFragment {
        reasoning,
        text: value.to_string(),
        part: None,
        response_id: None,
        error: None,                incomplete: None,
    };
    delta["content"]
        .as_str()
        .map(|value| text(false, value))
        .or_else(|| delta["reasoning_content"].as_str().map(|value| text(true, value)))
        .or_else(|| delta["reasoning"].as_str().map(|value| text(true, value)))
}
