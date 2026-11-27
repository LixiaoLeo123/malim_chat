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
    if !meta.starts_with("image/") && !meta.starts_with("video/") || data.is_empty() {
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

pub(crate) fn content_part(kind: &str, supports_images: bool, content: &str, images: &[Value], files: &[Value]) -> Value {
    if images.is_empty() && files.is_empty() {
        return json!(content);
    }
    let can_images = supports_images || kind == "google_gemini";
    // Only DeepSeek takes documents by reference; every other dialect gets a note that
    // the file is there but unavailable to it.
    let can_files = kind == "deepseek";
    if !can_images && !can_files {
        let mut text = plain_text_content(content, images);
        for _ in files {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str("[File attachment omitted: the selected provider does not support file input.]");
        }
        return json!(text);
    }
    let mut parts: Vec<Value> = Vec::new();
    if !content.trim().is_empty() {
        parts.push(text_part(kind, content));
    }
    for image in images {
        if !can_images {
            parts.push(text_part(kind, "[Image attachment omitted: the selected model does not support image input.]"));
            continue;
        }
        let Some((media_type, data)) = parse_data_url(image.as_str().unwrap_or("")) else {
            parts.push(text_part(kind, "[Media attachment omitted: invalid data URL.]"));
            continue;
        };
        let is_video = media_type.starts_with("video/");
        if is_video && kind != "google_gemini" {
            parts.push(text_part(kind, "[Video attachment omitted: the selected provider does not support video input.]"));
            continue;
        }
        match kind {
            "anthropic" => parts.push(json!({"type": "image", "source": {"type": "base64", "media_type": media_type, "data": data}})),
            "openai_responses" => parts.push(json!({"type": "input_image", "image_url": image})),
            "google_gemini" => parts.push(json!({"inlineData": {"mimeType": media_type, "data": data}})),
            _ => parts.push(json!({"type": "image_url", "image_url": {"url": image}})),
        }
    }
    for file in files {
        if !can_files {
            parts.push(text_part(kind, "[File attachment omitted: the selected provider does not support file input.]"));
            continue;
        }
        if let Some(id) = file["id"].as_str() {
            parts.push(json!({"type": "file", "file_id": id}));
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
