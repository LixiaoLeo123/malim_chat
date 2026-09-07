use super::*;
use crate::web_tools::web_tool_definitions;

#[derive(Debug)]
pub(crate) struct ToolCall {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) input: Value,
}

pub(crate) struct ProviderToolTurn {
    pub(crate) content: String,
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
    forced_tool: Option<&str>,
) -> Result<ProviderToolTurn, ApiError> {
    let url = provider_url(kind, base);
    let response = if kind == "anthropic" {
        let system = messages
            .iter()
            .filter(|message| message["role"] == "system")
            .filter_map(|message| message["content"].as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut body = json!({"model":model,"max_tokens":8192,"messages":messages.iter().filter(|message|message["role"] != "system").collect::<Vec<_>>(),"tools":web_tool_definitions(kind)});
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if let Some(value) = temperature {
            body["temperature"] = json!(value.clamp(0.0, 2.0));
        }
        if let Some(name) = forced_tool {
            body["tool_choice"] = json!({"type":"tool","name":name});
        }
        http.post(url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?
    } else {
        let mut body = json!({"model":model,"messages":messages,"stream":false,"tools":web_tool_definitions(kind)});
        if let Some(value) = temperature {
            body["temperature"] = json!(value.clamp(0.0, 2.0));
        }
        if let Some(value) = reasoning_effort {
            body["reasoning_effort"] = json!(value);
        }
        if let Some(name) = forced_tool {
            body["tool_choice"] = json!({"type":"function","function":{"name":name}});
        }
        http.post(url).bearer_auth(key).json(&body).send().await?
    };
    let body: Value = ensure_provider_success(response, true)
        .await?
        .json()
        .await?;
    if kind == "anthropic" {
        let blocks = body["content"]
            .as_array()
            .cloned()
            .ok_or_else(|| ApiError::bad("The provider returned an unexpected tool response."))?;
        let tool_calls = blocks
            .iter()
            .filter(|block| block["type"] == "tool_use")
            .filter_map(|block| {
                Some(ToolCall {
                    id: block["id"].as_str()?.to_string(),
                    name: block["name"].as_str()?.to_string(),
                    input: block["input"].clone(),
                })
            })
            .collect();
        Ok(ProviderToolTurn {
            content: anthropic_content(&body).unwrap_or_default(),
            assistant_message: json!({"role":"assistant","content":blocks}),
            tool_calls,
        })
    } else {
        let mut message = body["choices"]
            .as_array()
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .cloned()
            .ok_or_else(|| ApiError::bad("The provider returned an unexpected tool response."))?;
        if message["role"].is_null() {
            message["role"] = json!("assistant");
        }
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
        Ok(ProviderToolTurn {
            content: openai_content(&json!({"choices":[{"message":message.clone()}]}))
                .unwrap_or_default(),
            assistant_message: message,
            tool_calls,
        })
    }
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
) -> Result<String, ApiError> {
    let url = provider_url(kind, base);
    let response = if kind == "anthropic" {
        let system = messages
            .iter()
            .filter(|m| m["role"] == "system")
            .filter_map(|m| m["content"].as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut body = json!({"model":model,"max_tokens":8192,"messages":messages.iter().filter(|m|m["role"] != "system").collect::<Vec<_>>()});
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if let Some(value) = temperature {
            body["temperature"] = json!(value.clamp(0.0, 2.0));
        }
        http.post(url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?
    } else {
        let mut body = json!({"model":model,"messages":messages,"stream":false});
        if let Some(value) = temperature {
            body["temperature"] = json!(value.clamp(0.0, 2.0));
        }
        if let Some(value) = reasoning_effort {
            body["reasoning_effort"] = json!(value);
        }
        http.post(url).bearer_auth(key).json(&body).send().await?
    };
    let body: Value = ensure_provider_success(response, false)
        .await?
        .json()
        .await?;
    let answer = if kind == "anthropic" {
        anthropic_content(&body)
    } else {
        openai_content(&body)
    }
    .ok_or_else(|| ApiError {
        status: StatusCode::BAD_GATEWAY,
        code: "invalid_provider_response",
        message: "The AI provider returned an unexpected response.".into(),
    })?;
    Ok(answer)
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
) -> Result<reqwest::Response, ApiError> {
    let url = provider_url(kind, base);
    let response = if kind == "anthropic" {
        let system = messages
            .iter()
            .filter(|m| m["role"] == "system")
            .filter_map(|m| m["content"].as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut body = json!({"model":model,"max_tokens":8192,"stream":true,"messages":messages.iter().filter(|m|m["role"] != "system").collect::<Vec<_>>()});
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if let Some(value) = temperature {
            body["temperature"] = json!(value.clamp(0.0, 2.0));
        }
        http.post(url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?
    } else {
        let mut body = json!({"model":model,"messages":messages,"stream":true});
        if let Some(value) = temperature {
            body["temperature"] = json!(value.clamp(0.0, 2.0));
        }
        if let Some(value) = reasoning_effort {
            body["reasoning_effort"] = json!(value);
        }
        http.post(url).bearer_auth(key).json(&body).send().await?
    };
    ensure_provider_success(response, false).await
}

async fn ensure_provider_success(
    response: reqwest::Response,
    tool_request: bool,
) -> Result<reqwest::Response, ApiError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let body_hint = body.chars().take(400).collect::<String>();
    warn!(upstream_status=%status, tool_request, body_length=body.len(), "AI provider returned an error response");
    Err(provider_error_from_response(
        status,
        &body_hint,
        tool_request,
    ))
}

pub(crate) fn provider_error_from_response(
    status: StatusCode,
    body: &str,
    tool_request: bool,
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
    if status.is_client_error() {
        return ApiError::provider_rejected();
    }
    ApiError::provider_unavailable()
}

pub(crate) fn provider_url(kind: &str, base: &str) -> String {
    let base = base.trim_end_matches('/');
    match kind {
        "anthropic" if base.ends_with("/v1/messages") => base.to_string(),
        "anthropic" if base.ends_with("/v1") => format!("{base}/messages"),
        "anthropic" => format!("{base}/v1/messages"),
        _ if base.ends_with("/chat/completions") => base.to_string(),
        _ if base.ends_with("/v1") => format!("{base}/chat/completions"),
        _ => format!("{base}/v1/chat/completions"),
    }
}

fn anthropic_content(body: &Value) -> Option<String> {
    let content = body["content"].as_array()?;
    let output = content
        .iter()
        .filter_map(|part| match part["type"].as_str() {
            Some("text") => part["text"].as_str().map(str::to_string),
            Some("thinking") => part["thinking"]
                .as_str()
                .map(|text| format!("<think>{text}</think>")),
            _ => None,
        })
        .collect::<String>();
    (!output.is_empty()).then_some(output)
}

fn openai_content(body: &Value) -> Option<String> {
    let message = body["choices"].as_array()?.first()?.get("message")?;
    let mut output = String::new();
    for field in ["reasoning_content", "reasoning"] {
        if let Some(value) = message[field].as_str() {
            output.push_str("<think>");
            output.push_str(value);
            output.push_str("</think>");
        }
    }
    if let Some(value) = message["content"].as_str() {
        output.push_str(value);
    }
    (!output.is_empty()).then_some(output)
}

pub(crate) fn provider_stream_delta(kind: &str, frame: &str) -> Option<(bool, String)> {
    let payload = frame
        .lines()
        .find_map(|line| line.strip_prefix("data:"))?
        .trim();
    if payload == "[DONE]" {
        return None;
    }
    let value: Value = serde_json::from_str(payload).ok()?;
    if kind == "anthropic" {
        value["delta"]["text"]
            .as_str()
            .map(|text| (false, text.to_string()))
            .or_else(|| {
                value["delta"]["thinking"]
                    .as_str()
                    .map(|text| (true, text.to_string()))
            })
    } else {
        let delta = &value["choices"].as_array()?.first()?["delta"];
        delta["content"]
            .as_str()
            .map(|text| (false, text.to_string()))
            .or_else(|| {
                delta["reasoning_content"]
                    .as_str()
                    .map(|text| (true, text.to_string()))
            })
            .or_else(|| {
                delta["reasoning"]
                    .as_str()
                    .map(|text| (true, text.to_string()))
            })
    }
}
