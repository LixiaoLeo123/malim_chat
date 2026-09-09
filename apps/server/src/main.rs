use std::{
    env,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, OsRng},
};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::RngCore},
};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Duration, Utc};
use flate2::read::GzDecoder;
use futures_util::StreamExt;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use reqwest::Client;
use rusqlite::{Connection, params};
use rust_mdict::{KeyWordItem, Mdx};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, postgres::PgPoolOptions};
use tower_http::{
    cors::CorsLayer, limit::RequestBodyLimitLayer,
    sensitive_headers::SetSensitiveRequestHeadersLayer, trace::TraceLayer,
};
use tracing::{error, info, warn};
use uuid::Uuid;

mod agent;
mod agent_events;
mod anthropic_stream;
mod auth;
mod compaction;
mod content;
mod conversation;
mod dictionary;
mod models;
mod provider_config;
mod providers;
mod respond;
mod stream;
mod thinking;
mod web_tools;

use agent::agent_run_events;
use auth::{decrypt, encrypt, login, refresh, signup, user_from_headers};
use compaction::{auto_compact, compact};
use content::{content_part, estimate_image_tokens, estimate_tokens, parse_data_url};
use conversation::{
    create_conversation, create_message, delete_conversation, delete_message, list_conversations,
    list_messages, update_conversation, update_message,
};
use models::{
    Conversation, CreateConversation, CreateMessage, GenerationSettings, Message, Page, PageQuery,
    Provider, ProviderModel, ProviderModelRequest, ProviderRequest, ProviderRow, RespondRequest,
    UpdateConversation, UpdateMessage, UpdateProviderModelRequest, UpdateProviderRequest,
};
use provider_config::{
    create_provider, create_provider_model, configured_model, delete_provider,
    delete_provider_model, list_providers, own_provider, provider_first_model,
    provider_model_bool, provider_model_hosted_tools, provider_model_kind,
    provider_model_supports_images, update_provider, update_provider_model,
};
use respond::{clear_chain, respond};
use stream::{stream_response, stream_web_agent_response};
use thinking::{split_thinking, strip_thinking, ThinkingStream};

const ACCESS_TOKEN_MINUTES: i64 = 15;
const REFRESH_TOKEN_DAYS: i64 = 30;
const DEFAULT_PAGE_SIZE: i64 = 50;
const MAX_PAGE_SIZE: i64 = 100;
const MAX_WEB_SOURCES: usize = 12;
const MIN_PREFERRED_SEARCH_RESULTS: usize = 3;

#[derive(Clone)]
struct AppState {
    db: PgPool,
    http: Client,
    web_reader: Client,
    jwt_secret: Arc<Vec<u8>>,
    encryption_key: Arc<[u8; 32]>,
    searxng_url: Option<String>,
    searxng_preferred_engines: Option<String>,
    dictionary_dir: Arc<PathBuf>,
    russian_dictionary: Arc<Mutex<Mdx>>,
    allow_signup: bool,
}

struct Config {
    database_url: String,
    bind: SocketAddr,
    jwt_secret: String,
    encryption_key: String,
    searxng_url: Option<String>,
    searxng_preferred_engines: Option<String>,
    cors_origins: Vec<HeaderValue>,
    dictionary_dir: PathBuf,
    allow_signup: bool,
}

impl Config {
    fn from_env() -> Result<Self, ApiError> {
        dotenvy::dotenv().ok();
        let get =
            |name: &str| env::var(name).map_err(|_| ApiError::internal(format!("missing {name}")));
        let raw_key = BASE64
            .decode(get("MALIM_ENCRYPTION_KEY")?.as_bytes())
            .map_err(|_| ApiError::internal("MALIM_ENCRYPTION_KEY must be base64"))?;
        if raw_key.len() != 32 {
            return Err(ApiError::internal(
                "MALIM_ENCRYPTION_KEY must decode to 32 bytes",
            ));
        }
        let cors_origins = env::var("MALIM_CORS_ORIGINS")
            .unwrap_or_else(|_| "http://localhost:1420,tauri://localhost".into())
            .split(',')
            .map(|v| {
                v.trim()
                    .parse()
                    .map_err(|_| ApiError::internal("invalid CORS origin"))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            database_url: get("DATABASE_URL")?,
            bind: env::var("MALIM_BIND")
                .unwrap_or_else(|_| "127.0.0.1:3100".into())
                .parse()
                .map_err(|_| ApiError::internal("invalid MALIM_BIND"))?,
            jwt_secret: get("MALIM_JWT_SECRET")?,
            encryption_key: BASE64.encode(raw_key),
            searxng_url: env::var("SEARXNG_URL").ok().filter(|v| !v.is_empty()),
            searxng_preferred_engines: env::var("SEARXNG_PREFERRED_ENGINES")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| Some("yandex".into())),
            cors_origins,
            dictionary_dir: env::var("MALIM_DICTIONARY_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("apps/server/dictionaries")),
            allow_signup: env::var("MALIM_ALLOW_SIGNUP")
                .map(|value| !matches!(value.trim().to_lowercase().as_str(), "0" | "false" | "no"))
                .unwrap_or(true),
        })
    }
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}
impl ApiError {
    fn bad(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_request",
            message: message.into(),
        }
    }
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: "Authentication is required.".into(),
        }
    }
    fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: "The requested resource was not found.".into(),
        }
    }
    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: message.into(),
        }
    }
    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "active_run",
            message: message.into(),
        }
    }
    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: message.into(),
        }
    }

    fn provider_access_denied() -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code: "provider_access_denied",
            message: "The AI provider rejected this request. Check the provider key, endpoint, model access, and account balance or allowlist.".into(),
        }
    }

    fn provider_rate_limited() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "provider_rate_limited",
            message: "The AI provider is rate limiting requests. Wait briefly and try again."
                .into(),
        }
    }

    fn provider_rejected() -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code: "provider_rejected_request",
            message: "The AI provider rejected this request. Check the selected model and provider configuration.".into(),
        }
    }

    fn provider_tool_unsupported() -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code: "provider_tool_unsupported",
            message: "The selected provider or model does not support native tool calling.".into(),
        }
    }

    /// Never surfaced: the caller recovers by resending the whole conversation.
    fn provider_chain_stale() -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code: "provider_chain_stale",
            message: "The provider no longer recognises the stored conversation chain.".into(),
        }
    }

    fn provider_unavailable() -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code: "provider_unavailable",
            message: "The AI provider could not be reached.".into(),
        }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"error": {"code": self.code, "message": self.message}})),
        )
            .into_response()
    }
}
impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        error!(%error, "database failure");
        Self::internal("The server could not complete this request.")
    }
}
impl From<reqwest::Error> for ApiError {
    fn from(error: reqwest::Error) -> Self {
        error!(%error, "upstream provider failure");
        Self::provider_unavailable()
    }
}

#[derive(Deserialize)]
struct CompactRequest {
    through_sequence: Option<i64>,
    force: Option<bool>,
}

fn validate_generation_settings(settings: GenerationSettings) -> Result<Value, ApiError> {
    if !settings.temperature.is_finite() || !(0.0..=2.0).contains(&settings.temperature) {
        return Err(ApiError::bad("Temperature must be between 0 and 2."));
    }
    if !matches!(
        settings.reasoning_effort.as_str(),
        "low" | "medium" | "high"
    ) {
        return Err(ApiError::bad(
            "Reasoning effort must be low, medium, or high.",
        ));
    }
    if let Some(rounds) = settings.context_rounds {
        if rounds > 20 {
            return Err(ApiError::bad(
                "Context rounds must be between 0 and 20, or All.",
            ));
        }
    }
    if let Some(rounds) = settings.tool_rounds {
        if rounds == 0 || rounds > 20 {
            return Err(ApiError::bad(
                "Tool rounds must be between 1 and 20, or Unlimited.",
            ));
        }
    }
    Ok(json!(settings))
}

async fn own_conversation(
    pool: &PgPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Conversation, ApiError> {
    sqlx::query_as::<_, Conversation>("SELECT id,title,model_provider_id,model,context_window,context_tokens,is_favorite,generation_settings,revision,created_at,updated_at FROM conversations WHERE id=$1 AND user_id=$2 AND archived_at IS NULL").bind(id).bind(user_id).fetch_optional(pool).await?.ok_or_else(ApiError::not_found)
}

async fn event(
    pool: &PgPool,
    user_id: Uuid,
    entity_type: &str,
    entity_id: Uuid,
    operation: &str,
    revision: i64,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO sync_events (user_id,entity_type,entity_id,operation,revision) VALUES ($1,$2,$3,$4,$5)").bind(user_id).bind(entity_type).bind(entity_id).bind(operation).bind(revision).execute(pool).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();
    let config = Config::from_env().map_err(|e| anyhow::anyhow!(e.message))?;
    let decoded = BASE64.decode(config.encryption_key.as_bytes())?;
    let key: [u8; 32] = decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid encryption key length"))?;
    let db = PgPoolOptions::new()
        .max_connections(20)
        .connect(&config.database_url)
        .await?;
    // Apply embedded schema migrations before serving requests.
    sqlx::migrate!().run(&db).await?;
    let russian_dictionary = Mdx::new(config.dictionary_dir.join("OpenRussian.mdx"))
        .map_err(|_| anyhow::anyhow!("Russian dictionary could not be opened"))?;
    let state = AppState {
        db,
        http: Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?,
        web_reader: Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        jwt_secret: Arc::new(config.jwt_secret.into_bytes()),
        encryption_key: Arc::new(key),
        searxng_url: config.searxng_url,
        searxng_preferred_engines: config.searxng_preferred_engines,
        dictionary_dir: Arc::new(config.dictionary_dir),
        russian_dictionary: Arc::new(Mutex::new(russian_dictionary)),
        allow_signup: config.allow_signup,
    };
    let cors = CorsLayer::new()
        .allow_origin(config.cors_origins)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers([http::header::AUTHORIZATION, http::header::CONTENT_TYPE]);
    let app = Router::new()
        .route("/healthz", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/v1/auth/signup", post(signup))
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/refresh", post(refresh))
        .route("/v1/providers", get(list_providers).post(create_provider))
        .route(
            "/v1/providers/{id}",
            patch(update_provider).delete(delete_provider),
        )
        .route("/v1/providers/{id}/models", post(create_provider_model))
        .route(
            "/v1/providers/{id}/models/{model_id}",
            patch(update_provider_model).delete(delete_provider_model),
        )
        .route(
            "/v1/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route(
            "/v1/conversations/{id}",
            patch(update_conversation)
                .delete(delete_conversation),
        )
        .route(
            "/v1/conversations/{id}/messages",
            get(list_messages).post(create_message),
        )
        .route(
            "/v1/conversations/{conversation_id}/agent-runs/{message_id}",
            get(agent_run_events),
        )
        .route(
            "/v1/conversations/{id}/messages/{message_id}",
            patch(update_message).delete(delete_message),
        )
        .route("/v1/conversations/{id}/respond", post(respond))
        .route("/v1/conversations/{id}/compact", post(compact))
        .route("/v1/dictionary", get(dictionary::dictionary_lookup))
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .layer(RequestBodyLimitLayer::new(64 * 1024 * 1024))
        .layer(SetSensitiveRequestHeadersLayer::new(std::iter::once(
            http::header::AUTHORIZATION,
        )))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state);
    info!(bind=%config.bind, "malim_chat server started");
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use crate::content::{content_part, parse_data_url, plain_text_content};
    use crate::thinking::{split_thinking, strip_thinking, ThinkingStream};
    use crate::providers::{
        provider_error_from_response, provider_stream_delta, provider_url, RequestKind, StreamFragment,
    };
    use crate::web_tools::{is_safe_public_url, is_valid_search_query, web_tool_definitions};
    use axum::http::StatusCode;

    #[test]
    fn accepts_generic_planner_queries_without_topic_rules() {
        assert!(is_valid_search_query("ab"));
        assert!(is_valid_search_query("a specific search query"));
        assert!(!is_valid_search_query("x"));
    }

    #[test]
    fn repeats_the_provider_own_error_words() {
        let error = provider_error_from_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "{\"error\":{\"message\":\"no channel for model gpt-5.6-luna\",\"type\":\"new_api_error\"}}",
            false,
            false,
        );
        assert_eq!(error.code, "provider_unavailable");
        assert!(
            error.message.contains("no channel for model gpt-5.6-luna"),
            "{}",
            error.message
        );
    }

    #[test]
    fn reports_failures_nested_inside_a_response_stream() {
        // A gateway can answer 200 and still fail the turn inside the stream body.
        let failed = provider_stream_delta(
            "openai_responses",
            "data: {\"type\":\"response.failed\",\"response\":{\"id\":\"resp_1\",\"status\":\"failed\",\"error\":{\"message\":\"no channel for model\"}}}",
        )
        .expect("nested failure should surface");
        assert_eq!(failed.error.as_deref(), Some("no channel for model"));

        let incomplete = provider_stream_delta(
            "openai_responses",
            "data: {\"type\":\"response.incomplete\",\"response\":{\"id\":\"resp_2\",\"status\":\"incomplete\",\"incomplete_details\":{\"reason\":\"max_output_tokens\"}}}",
        )
        .expect("incomplete should surface");
        assert_eq!(incomplete.error, None);
        assert_eq!(incomplete.incomplete.as_deref(), Some("max_output_tokens"));

        // Lifecycle frames serialize their empty fields as JSON null, so `"error": null`
        // must read as "no error" — reading it as one kills the turn before any text.
        let created = provider_stream_delta(
            "openai_responses",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_3\",\"status\":\"in_progress\",\"error\":null,\"incomplete_details\":null}}",
        )
        .expect("the stored response id is still worth keeping");
        assert_eq!(created.error, None);
        assert_eq!(created.incomplete, None);
        assert_eq!(created.response_id.as_deref(), Some("resp_3"));
    }

    #[test]
    fn shows_anthropic_hosted_tools_as_timeline_activity() {
        let call = provider_stream_delta(
            "anthropic",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"server_tool_use\",\"id\":\"srvtoolu_1\",\"name\":\"web_search\",\"input\":{}}}",
        )
        .expect("a hosted call is worth showing");
        assert_eq!(call.activity.as_ref().unwrap()["id"], "srvtoolu_1");
        assert_eq!(call.activity.as_ref().unwrap()["status"], "running");
        assert!(call.sources.is_empty());

        let result = provider_stream_delta(
            "anthropic",
            "data: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"web_search_tool_result\",\"tool_use_id\":\"srvtoolu_1\",\"content\":[{\"type\":\"web_search_result\",\"title\":\"The Router\",\"url\":\"https://example.com/a\",\"page_age\":\"3 days ago\"}]}}",
        )
        .expect("a hosted result is worth showing");
        // The same id updates the row the call opened instead of adding a second one.
        assert_eq!(result.activity.as_ref().unwrap()["id"], "srvtoolu_1");
        assert_eq!(result.activity.as_ref().unwrap()["status"], "completed");
        assert_eq!(result.sources.len(), 1);
        assert_eq!(result.sources[0]["url"], "https://example.com/a");

        let paused = provider_stream_delta(
            "anthropic",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"pause_turn\"}}",
        )
        .expect("a pause is not an end");
        assert!(paused.paused);
        assert!(paused.text.is_empty());

        let stopped = provider_stream_delta(
            "anthropic",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}",
        );
        assert!(stopped.is_none(), "a finished turn has nothing to relay");
    }

    #[test]
    fn sends_the_provider_the_tools_its_dialect_actually_defines() {
        let hosted = crate::providers::build_request(
            "anthropic",
            "https://api.anthropic.com",
            "claude-sonnet-5",
            &[crate::respond::instruction_message(true)],
            None,
            None,
            crate::providers::Tools::Hosted,
            false,
            None,
        );
        let types = hosted.body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["type"].as_str().unwrap())
            .collect::<Vec<_>>();
        // These exact strings are the API's, and the versions are the ones that need no
        // beta header and no paired code-execution version.
        assert_eq!(
            types,
            [
                "web_search_20250305",
                "web_fetch_20250910",
                "code_execution_20250825"
            ]
        );
        assert_eq!(hosted.body["messages"], serde_json::json!([]));
        assert!(hosted.anthropic);

        // A Chat endpoint has no hosted tools at all, so it must never be sent any.
        let plain = crate::providers::build_request(
            "openai_compatible",
            "https://example.com/v1",
            "glm-5.3",
            &[crate::respond::instruction_message(true)],
            None,
            None,
            crate::providers::Tools::Hosted,
            false,
            None,
        );
        assert!(plain.body.get("tools").is_none());
    }

    #[test]
    fn rebuilds_a_paused_anthropic_message_to_send_back_unchanged() {
        use crate::anthropic_stream::AnthropicMessage;
        let mut message = AnthropicMessage::default();
        for frame in [
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Searching\"}}",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" now\"}}",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"server_tool_use\",\"id\":\"srvtoolu_1\",\"name\":\"web_search\",\"input\":{}}}",
        ] {
            message.apply(frame);
        }
        // A hosted call's input arrives in fragments and is only whole at the end.
        let partial = serde_json::json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"routers\"}"}});
        message.apply(&format!("data: {partial}"));
        for frame in [
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\"}}",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"pause_turn\"}}",
        ] {
            message.apply(frame);
        }
        assert!(message.paused());
        let content = message.content();
        assert_eq!(content[0]["text"], "Searching now");
        assert_eq!(content[1]["input"]["query"], "routers");
        assert_eq!(content[1]["name"], "web_search");
        // `message_start` describes the envelope, not a block, so it must not become one.
        assert_eq!(content.len(), 2);
    }

    #[test]
    fn keeps_responses_summary_parts_apart() {
        assert_eq!(
            provider_stream_delta(
                "openai_responses",
                "data: {\"type\":\"response.reasoning_summary_text.delta\",\"output_index\":1,\"summary_index\":2,\"delta\":\"**Modeling**\"}"
            ),
            Some(StreamFragment {
                reasoning: true,
                text: "**Modeling**".into(),
                part: Some("1:2".into()),
                response_id: None,
                error: None,
                ..Default::default()
            })
        );
        assert_eq!(
            provider_stream_delta(
                "openai_responses",
                "data: {\"type\":\"response.reasoning_summary_text.delta\",\"output_index\":2,\"summary_index\":2,\"delta\":\"**Again**\"}"
            ),
            Some(StreamFragment {
                reasoning: true,
                text: "**Again**".into(),
                part: Some("2:2".into()),
                response_id: None,
                error: None,
                ..Default::default()
            })
        );
        assert_eq!(
            provider_stream_delta(
                "openai_responses",
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}"
            ),
            Some(StreamFragment {
                reasoning: false,
                text: "hello".into(),
                part: None,
                response_id: None,
                error: None,
                ..Default::default()
            })
        );
    }

    #[test]
    fn parses_openai_stream_delta() {
        assert_eq!(
            provider_stream_delta(
                "openai_compatible",
                "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}"
            ),
            Some(StreamFragment {
                reasoning: false,
                text: "hello".into(),
                part: None,
                response_id: None,
                error: None,
                ..Default::default()
            })
        );
        assert_eq!(
            provider_stream_delta(
                "openai_compatible",
                "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"plan\"}}]}"
            ),
            Some(StreamFragment {
                reasoning: true,
                text: "plan".into(),
                part: None,
                response_id: None,
                error: None,
                ..Default::default()
            })
        );
        assert_eq!(
            provider_stream_delta("anthropic", "data: {\"delta\":{\"thinking\":\"plan\"}}"),
            Some(StreamFragment {
                reasoning: true,
                text: "plan".into(),
                part: None,
                response_id: None,
                error: None,
                ..Default::default()
            })
        );
    }

    #[test]
    fn accepts_standard_provider_base_urls() {
        assert_eq!(
            provider_url(RequestKind::OpenAiChat, "https://api.openai.com"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            provider_url(RequestKind::OpenAiChat, "https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            provider_url(RequestKind::Anthropic, "https://api.anthropic.com"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            provider_url(RequestKind::Anthropic, "https://api.anthropic.com/v1"),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn exposes_provider_native_web_tools_and_rejects_private_urls() {
        assert!(web_tool_definitions("openai_compatible")[0]["function"]["parameters"]["properties"]["query"].is_object());
        assert!(
            web_tool_definitions("anthropic")[0]["input_schema"]["properties"]["query"].is_object()
        );
        assert!(is_safe_public_url("https://www.example.com/article"));
        assert!(!is_safe_public_url("http://127.0.0.1:3100/healthz"));
        assert!(!is_safe_public_url("http://10.0.0.8/admin"));
        assert!(!is_safe_public_url("http://[::1]/"));
    }

    #[test]
    fn parses_image_data_urls() {
        assert_eq!(
            parse_data_url("data:image/png;base64,AAAA"),
            Some(("image/png".into(), "AAAA".into()))
        );
        assert_eq!(
            parse_data_url("data:image/jpeg;base64,BBBB"),
            Some(("image/jpeg".into(), "BBBB".into()))
        );
        assert_eq!(parse_data_url("data:text/plain;base64,AAAA"), None);
        assert_eq!(parse_data_url("data:image/png;base64,"), None);
        assert_eq!(parse_data_url("https://example.com/a.png"), None);
    }

    #[test]
    fn degrades_images_to_placeholder_when_model_has_no_vision() {
        let images = vec![serde_json::json!("data:image/png;base64,AAAA")];
        let content = content_part("openai_compatible", false, "describe this", &images);
        assert!(
            content
                .as_str()
                .unwrap()
                .contains("does not support image input")
        );
        let text = plain_text_content("describe this", &images);
        assert!(text.contains("does not support image input"));
        assert!(text.starts_with("describe this"));
    }

    #[test]
    fn builds_multimodal_content_for_vision_models() {
        let images = vec![serde_json::json!("data:image/png;base64,AAAA")];
        let openai = content_part("openai_compatible", true, "describe this", &images);
        assert_eq!(openai[0]["type"], "text");
        assert_eq!(openai[1]["type"], "image_url");
        assert_eq!(openai[1]["image_url"]["url"], "data:image/png;base64,AAAA");
        let anthropic = content_part("anthropic", true, "describe this", &images);
        assert_eq!(anthropic[1]["type"], "image");
        assert_eq!(anthropic[1]["source"]["media_type"], "image/png");
        assert_eq!(anthropic[1]["source"]["data"], "AAAA");
        let responses = content_part("openai_responses", true, "describe this", &images);
        assert_eq!(responses[0]["type"], "input_text");
        assert_eq!(responses[1]["type"], "input_image");
        assert_eq!(responses[1]["image_url"], "data:image/png;base64,AAAA");
    }

    #[test]
    fn separates_thinking_from_visible_answer_in_complete_and_streamed_text() {
        assert_eq!(
            strip_thinking("Before<think>private</think>After"),
            "BeforeAfter"
        );
        assert_eq!(
            split_thinking("Before<think>private</think>After"),
            ("BeforeAfter".into(), "private".into())
        );
        assert_eq!(
            split_thinking("Before<THINKING>private</Thinking>After"),
            ("BeforeAfter".into(), "private".into())
        );
        let mut stream = ThinkingStream::new();
        assert_eq!(stream.push("Before<thi"), vec![(false, "Before".into())]);
        assert_eq!(
            stream.push("nk>private</think>After"),
            vec![(true, "private".into()), (false, "After".into())]
        );
        let mut long_tag_stream = ThinkingStream::new();
        assert_eq!(
            long_tag_stream.push("Before<thinkin"),
            vec![(false, "Before".into())]
        );
        assert_eq!(
            long_tag_stream.push("g>private</thinking>After"),
            vec![(true, "private".into()), (false, "After".into())]
        );
    }

    #[test]
    fn distinguishes_provider_access_and_tool_schema_failures() {
        assert_eq!(
            provider_error_from_response(
                StatusCode::FORBIDDEN,
                "tool calling is unsupported",
                true, false
            )
            .code,
            "provider_access_denied"
        );
        assert_eq!(
            provider_error_from_response(
                StatusCode::BAD_REQUEST,
                "tool_choice is unsupported",
                true, false
            )
            .code,
            "provider_tool_unsupported"
        );
        assert_eq!(
            provider_error_from_response(StatusCode::TOO_MANY_REQUESTS, "", false, false).code,
            "provider_rate_limited"
        );
    }
}
