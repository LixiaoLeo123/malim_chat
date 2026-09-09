use super::*;

/// Wire types: what the API accepts and what it returns.  structs mirror the
/// tables they are read from,  shapes are what clients see.

#[derive(Debug, Serialize)]
pub(crate) struct Provider {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) base_url: String,
    pub(crate) default_model: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) models: Vec<ProviderModel>,
}

#[derive(Debug, FromRow)]
pub(crate) struct ProviderRow {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) base_url: String,
    pub(crate) default_model: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, FromRow)]
pub(crate) struct ProviderModel {
    pub(crate) id: Uuid,
    pub(crate) provider_id: Uuid,
    pub(crate) group_name: String,
    pub(crate) model: String,
    pub(crate) kind: String,
    pub(crate) sort_order: i32,
    pub(crate) context_window: i32,
    pub(crate) supports_images: bool,
    pub(crate) chain_context: bool,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, FromRow)]
pub(crate) struct Conversation {
    pub(crate) id: Uuid,
    pub(crate) title: String,
    pub(crate) model_provider_id: Option<Uuid>,
    pub(crate) model: Option<String>,
    pub(crate) context_window: i32,
    pub(crate) context_tokens: i32,
    pub(crate) is_favorite: bool,
    pub(crate) generation_settings: Value,
    pub(crate) revision: i64,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, FromRow)]
pub(crate) struct Message {
    pub(crate) id: Uuid,
    pub(crate) conversation_id: Uuid,
    pub(crate) sequence: i64,
    pub(crate) client_mutation_id: Option<Uuid>,
    pub(crate) role: String,
    pub(crate) content: String,
    pub(crate) reasoning_content: String,
    pub(crate) content_format: String,
    pub(crate) status: String,
    pub(crate) model: Option<String>,
    pub(crate) token_count: i32,
    pub(crate) search_sources: Value,
    pub(crate) images: Value,
    pub(crate) edited_at: Option<DateTime<Utc>>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub(crate) struct PageQuery {
    pub(crate) cursor: Option<String>,
    pub(crate) limit: Option<i64>,
}

#[derive(Serialize)]
pub(crate) struct Page<T> {
    pub(crate) items: Vec<T>,
    pub(crate) next_cursor: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CreateConversation {
    pub(crate) title: Option<String>,
    pub(crate) provider_id: Option<Uuid>,
    pub(crate) model: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct UpdateConversation {
    pub(crate) title: Option<String>,
    pub(crate) archived: Option<bool>,
    pub(crate) provider_id: Option<Uuid>,
    pub(crate) model: Option<String>,
    pub(crate) generation_settings: Option<GenerationSettings>,
    pub(crate) is_favorite: Option<bool>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct GenerationSettings {
    pub(crate) temperature: f32,
    pub(crate) reasoning_effort: String,
    pub(crate) enable_markdown: bool,
    pub(crate) stream: bool,
    #[serde(default = "default_context_rounds")]
    pub(crate) context_rounds: Option<u8>,
    #[serde(default = "default_tool_rounds")]
    pub(crate) tool_rounds: Option<u8>,
}

pub(crate) fn default_context_rounds() -> Option<u8> {
    Some(8)
}

pub(crate) fn default_tool_rounds() -> Option<u8> {
    Some(DEFAULT_WEB_TOOL_ROUNDS as u8)
}

#[derive(Deserialize)]
pub(crate) struct ProviderRequest {
    pub(crate) name: String,
    pub(crate) kind: Option<String>,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) default_model: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct UpdateProviderRequest {
    pub(crate) name: Option<String>,
    pub(crate) kind: Option<String>,
    pub(crate) base_url: Option<String>,
    pub(crate) api_key: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ProviderModelRequest {
    pub(crate) group_name: String,
    pub(crate) model: String,
    pub(crate) kind: String,
    pub(crate) sort_order: Option<i32>,
    pub(crate) context_window: Option<i32>,
    pub(crate) supports_images: Option<bool>,
    pub(crate) chain_context: Option<bool>,
}

#[derive(Deserialize)]
pub(crate) struct UpdateProviderModelRequest {
    pub(crate) group_name: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) kind: Option<String>,
    pub(crate) sort_order: Option<i32>,
    pub(crate) context_window: Option<i32>,
    pub(crate) supports_images: Option<bool>,
    pub(crate) chain_context: Option<bool>,
}

#[derive(Deserialize)]
pub(crate) struct CreateMessage {
    pub(crate) content: String,
    pub(crate) client_mutation_id: Uuid,
    pub(crate) search: Option<bool>,
    pub(crate) images: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub(crate) struct UpdateMessage {
    pub(crate) content: String,
}

#[derive(Deserialize)]
pub(crate) struct RespondRequest {
    pub(crate) message_id: Uuid,
    pub(crate) search: Option<bool>,
    pub(crate) temperature: Option<f32>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) enable_markdown: Option<bool>,
    pub(crate) stream: Option<bool>,
    pub(crate) context_rounds: Option<Option<u8>>,
    pub(crate) tool_rounds: Option<Option<u8>>,
}
