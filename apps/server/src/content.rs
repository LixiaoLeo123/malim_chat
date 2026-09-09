use super::*;

/// Turns stored messages into the multimodal shape each provider dialect expects, and
/// keeps the token bookkeeping the context meter reads.

pub(crate) fn estimate_image_tokens(images: &[String]) -> i32 {
    images
        .iter()
        .map(|image| {
            let bytes = image.split(',').last().map(|part| part.len()).unwrap_or(0);
            (bytes / 1024).clamp(300, 2400) as i32
        })
        .sum()
}

pub(crate) fn parse_data_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    if !meta.starts_with("image/") || data.is_empty() {
        return None;
    }
    Some((meta.split(';').next()?.to_string(), data.to_string()))
}

pub(crate) fn plain_text_content(content: &str, images: &[Value]) -> String {
    if images.is_empty() {
        return content.to_string();
    }
    let mut text = content.to_string();
    for _ in images {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(
            "[Image attachment omitted: the selected model does not support image input.]",
        );
    }
    text
}

pub(crate) fn content_part(kind: &str, supports_images: bool, content: &str, images: &[Value]) -> Value {
    if images.is_empty() {
        return json!(content);
    }
    if !supports_images {
        return json!(plain_text_content(content, images));
    }
    let mut parts: Vec<Value> = Vec::new();
    if !content.trim().is_empty() {
        parts.push(text_part(kind, content));
    }
    for image in images {
        if let Some((media_type, data)) = parse_data_url(image.as_str().unwrap_or("")) {
            match kind {
                "anthropic" => parts.push(json!({"type": "image", "source": {"type": "base64", "media_type": media_type, "data": data}})),
                // Responses names its parts after the input item, not the Chat shape.
                "openai_responses" => parts.push(json!({"type": "input_image", "image_url": image})),
                _ => parts.push(json!({"type": "image_url", "image_url": {"url": image}})),
            }
        } else {
            parts.push(text_part(
                kind,
                "[Image attachment omitted: invalid image data.]",
            ));
        }
    }
    if parts.is_empty() {
        return json!(content);
    }
    if kind == "anthropic" && !parts.iter().any(|part| part["type"] == "text") {
        parts.insert(0, json!({"type": "text", "text": ""}));
    }
    json!(parts)
}

/// A text part in the dialect's own spelling: Responses names input parts after the item
/// (`input_text`), and a `text` part there is rejected as an invalid value.
fn text_part(kind: &str, content: &str) -> Value {
    json!({"type": if kind == "openai_responses" { "input_text" } else { "text" }, "text": content})
}

pub(crate) fn estimate_tokens(content: &str) -> i32 {
    ((content.chars().count() as f64 / 3.5).ceil() as i32).max(1)
}
