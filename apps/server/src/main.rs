use std::{env, net::SocketAddr, path::PathBuf, sync::{Arc, Mutex}};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, OsRng},
};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::RngCore},
};
use axum::{
    body::{Body, Bytes},
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
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

const ACCESS_TOKEN_MINUTES: i64 = 15;
const REFRESH_TOKEN_DAYS: i64 = 30;
const DEFAULT_PAGE_SIZE: i64 = 50;
const MAX_PAGE_SIZE: i64 = 100;

#[derive(Clone)]
struct AppState {
    db: PgPool,
    http: Client,
    jwt_secret: Arc<Vec<u8>>,
    encryption_key: Arc<[u8; 32]>,
    searxng_url: Option<String>,
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
    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: message.into(),
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
        Self {
            status: StatusCode::BAD_GATEWAY,
            code: "provider_unavailable",
            message: "The AI provider could not be reached.".into(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: Uuid,
    exp: usize,
    iat: usize,
    typ: String,
}
#[derive(Serialize)]
struct AuthResponse {
    access_token: String,
    refresh_token: String,
    user: User,
}
#[derive(Deserialize)]
struct SignupRequest {
    email: String,
    password: String,
    display_name: Option<String>,
}
#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}
#[derive(Deserialize)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Debug, Serialize, FromRow)]
struct User {
    id: Uuid,
    email: String,
    display_name: String,
    created_at: DateTime<Utc>,
}
#[derive(FromRow)]
struct AuthUser {
    id: Uuid,
    email: String,
    display_name: String,
    created_at: DateTime<Utc>,
    password_hash: String,
    disabled_at: Option<DateTime<Utc>>,
}
impl From<AuthUser> for User {
    fn from(value: AuthUser) -> Self {
        Self {
            id: value.id,
            email: value.email,
            display_name: value.display_name,
            created_at: value.created_at,
        }
    }
}
#[derive(FromRow)]
struct RefreshUser {
    id: Uuid,
    email: String,
    display_name: String,
    created_at: DateTime<Utc>,
    refresh_id: Uuid,
}
#[derive(Debug, Serialize)]
struct Provider {
    id: Uuid,
    name: String,
    kind: String,
    base_url: String,
    default_model: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    models: Vec<ProviderModel>,
}
#[derive(Debug, FromRow)]
struct ProviderRow {
    id: Uuid,
    name: String,
    kind: String,
    base_url: String,
    default_model: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
#[derive(Debug, Serialize, FromRow)]
struct ProviderModel {
    id: Uuid,
    provider_id: Uuid,
    group_name: String,
    model: String,
    kind: String,
    sort_order: i32,
    context_window: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
#[derive(Debug, Serialize, FromRow)]
struct Conversation {
    id: Uuid,
    title: String,
    model_provider_id: Option<Uuid>,
    model: Option<String>,
    context_window: i32,
    context_tokens: i32,
    generation_settings: Value,
    revision: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
#[derive(Debug, Serialize, FromRow)]
struct Message {
    id: Uuid,
    conversation_id: Uuid,
    sequence: i64,
    client_mutation_id: Option<Uuid>,
    role: String,
    content: String,
    reasoning_content: String,
    content_format: String,
    status: String,
    model: Option<String>,
    token_count: i32,
    search_sources: Value,
    edited_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct PageQuery {
    cursor: Option<String>,
    limit: Option<i64>,
}
#[derive(Serialize)]
struct Page<T> {
    items: Vec<T>,
    next_cursor: Option<String>,
}
#[derive(Deserialize)]
struct CreateConversation {
    title: Option<String>,
    provider_id: Option<Uuid>,
    model: Option<String>,
}
#[derive(Deserialize)]
struct UpdateConversation {
    title: Option<String>,
    archived: Option<bool>,
    provider_id: Option<Uuid>,
    model: Option<String>,
    generation_settings: Option<GenerationSettings>,
}
#[derive(Deserialize, Serialize)]
struct GenerationSettings {
    temperature: f32,
    reasoning_effort: String,
    enable_markdown: bool,
    stream: bool,
}
#[derive(Deserialize)]
struct ProviderRequest {
    name: String,
    kind: Option<String>,
    base_url: String,
    api_key: String,
    default_model: Option<String>,
}
#[derive(Deserialize)]
struct ProviderModelRequest {
    group_name: String,
    model: String,
    kind: String,
    sort_order: Option<i32>,
    context_window: Option<i32>,
}
#[derive(Deserialize)]
struct UpdateProviderModelRequest {
    group_name: Option<String>,
    model: Option<String>,
    kind: Option<String>,
    sort_order: Option<i32>,
    context_window: Option<i32>,
}
#[derive(Deserialize)]
struct CreateMessage {
    content: String,
    client_mutation_id: Uuid,
    search: Option<bool>,
}
#[derive(Deserialize)]
struct UpdateMessage {
    content: String,
}
#[derive(Deserialize)]
struct RespondRequest {
    message_id: Uuid,
    search: Option<bool>,
    temperature: Option<f32>,
    reasoning_effort: Option<String>,
    enable_markdown: Option<bool>,
    stream: Option<bool>,
}
#[derive(Deserialize)]
struct CompactRequest {
    through_sequence: Option<i64>,
    force: Option<bool>,
}
#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}
#[derive(Deserialize)]
struct SyncQuery {
    cursor: Option<i64>,
    limit: Option<i64>,
}
#[derive(Serialize, FromRow)]
struct SyncEvent {
    cursor: i64,
    entity_type: String,
    entity_id: Uuid,
    operation: String,
    revision: i64,
    occurred_at: DateTime<Utc>,
}
#[derive(Deserialize)]
struct DictionaryQuery {
    word: String,
    dictionary: String,
}
#[derive(Serialize)]
struct DictionaryEntryResponse {
    headword: String,
    lemma: String,
    pronunciation: String,
    definitions: Vec<String>,
    translations: Vec<String>,
    forms: Vec<String>,
    labels: Vec<String>,
    examples: Vec<String>,
    detail: Value,
    definition_html: String,
    matched_terms: Vec<String>,
}
#[derive(Serialize)]
struct DictionaryResponse {
    word: String,
    dictionary: String,
    entries: Vec<DictionaryEntryResponse>,
}

fn token_for(state: &AppState, user_id: Uuid) -> Result<String, ApiError> {
    let now = Utc::now();
    encode(
        &Header::default(),
        &Claims {
            sub: user_id,
            iat: now.timestamp() as usize,
            exp: (now + Duration::minutes(ACCESS_TOKEN_MINUTES)).timestamp() as usize,
            typ: "access".into(),
        },
        &EncodingKey::from_secret(&state.jwt_secret),
    )
    .map_err(|_| ApiError::internal("could not issue access token"))
}
fn user_from_headers(state: &AppState, headers: &HeaderMap) -> Result<Uuid, ApiError> {
    let raw = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(ApiError::unauthorized)?;
    let token = decode::<Claims>(
        raw,
        &DecodingKey::from_secret(&state.jwt_secret),
        &Validation::default(),
    )
    .map_err(|_| ApiError::unauthorized())?;
    if token.claims.typ != "access" {
        return Err(ApiError::unauthorized());
    }
    Ok(token.claims.sub)
}
fn digest(input: &str) -> String {
    format!("{:x}", Sha256::digest(input.as_bytes()))
}
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    BASE64.encode(bytes)
}
fn encrypt(state: &AppState, plaintext: &str) -> Result<(Vec<u8>, Vec<u8>), ApiError> {
    let cipher = Aes256Gcm::new_from_slice(state.encryption_key.as_ref())
        .map_err(|_| ApiError::internal("encryption setup failed"))?;
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let encrypted = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|_| ApiError::internal("credential encryption failed"))?;
    Ok((encrypted, nonce.to_vec()))
}
fn decrypt(state: &AppState, encrypted: &[u8], nonce: &[u8]) -> Result<String, ApiError> {
    let cipher = Aes256Gcm::new_from_slice(state.encryption_key.as_ref())
        .map_err(|_| ApiError::internal("encryption setup failed"))?;
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), encrypted)
        .map_err(|_| ApiError::internal("stored credential could not be decrypted"))?;
    String::from_utf8(plain).map_err(|_| ApiError::internal("stored credential is invalid"))
}
fn estimate_tokens(content: &str) -> i32 {
    ((content.chars().count() as f64 / 3.5).ceil() as i32).max(1)
}
fn validate_generation_settings(settings: GenerationSettings) -> Result<Value, ApiError> {
    if !settings.temperature.is_finite() || !(0.0..=2.0).contains(&settings.temperature) {
        return Err(ApiError::bad("Temperature must be between 0 and 2."));
    }
    if !matches!(settings.reasoning_effort.as_str(), "low" | "medium" | "high") {
        return Err(ApiError::bad("Reasoning effort must be low, medium, or high."));
    }
    Ok(json!(settings))
}
async fn own_conversation(
    pool: &PgPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Conversation, ApiError> {
    sqlx::query_as::<_, Conversation>("SELECT id,title,model_provider_id,model,context_window,context_tokens,generation_settings,revision,created_at,updated_at FROM conversations WHERE id=$1 AND user_id=$2 AND archived_at IS NULL").bind(id).bind(user_id).fetch_optional(pool).await?.ok_or_else(ApiError::not_found)
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
    sqlx::migrate!().run(&db).await?;
    let russian_dictionary = Mdx::new(config.dictionary_dir.join("OpenRussian.mdx"))
        .map_err(|_| anyhow::anyhow!("Russian dictionary could not be opened"))?;
    let state = AppState {
        db,
        http: Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?,
        jwt_secret: Arc::new(config.jwt_secret.into_bytes()),
        encryption_key: Arc::new(key),
        searxng_url: config.searxng_url,
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
        .route("/v1/me", get(me))
        .route("/v1/providers", get(list_providers).post(create_provider))
        .route("/v1/providers/{id}", delete(delete_provider))
        .route("/v1/providers/{id}/models", post(create_provider_model))
        .route("/v1/providers/{id}/models/{model_id}", patch(update_provider_model).delete(delete_provider_model))
        .route(
            "/v1/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route(
            "/v1/conversations/{id}",
            get(get_conversation)
                .patch(update_conversation)
                .delete(delete_conversation),
        )
        .route(
            "/v1/conversations/{id}/messages",
            get(list_messages).post(create_message),
        )
        .route(
            "/v1/conversations/{id}/messages/{message_id}",
            patch(update_message).delete(delete_message),
        )
        .route("/v1/conversations/{id}/respond", post(respond))
        .route("/v1/conversations/{id}/compact", post(compact))
        .route("/v1/search", get(search))
        .route("/v1/dictionary", get(dictionary_lookup))
        .route("/v1/sync", get(sync))
        .layer(RequestBodyLimitLayer::new(1_048_576))
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

async fn signup(
    State(state): State<AppState>,
    Json(request): Json<SignupRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    if !state.allow_signup {
        return Err(ApiError::forbidden(
            "New user registration is temporarily disabled.",
        ));
    }
    let email = request.email.trim().to_lowercase();
    if !email.contains('@') || request.password.len() < 12 {
        return Err(ApiError::bad(
            "Use a valid email and a password of at least 12 characters.",
        ));
    }
    let name = request
        .display_name
        .unwrap_or_else(|| email.split('@').next().unwrap_or("User").to_string())
        .trim()
        .to_string();
    if name.is_empty() || name.len() > 80 {
        return Err(ApiError::bad(
            "Display name must contain 1 to 80 characters.",
        ));
    }
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    let password_hash = Argon2::default()
        .hash_password(request.password.as_bytes(), &salt)
        .map_err(|_| ApiError::internal("could not protect password"))?
        .to_string();
    let user = User {
        id: Uuid::new_v4(),
        email,
        display_name: name,
        created_at: Utc::now(),
    };
    let inserted = sqlx::query("INSERT INTO users (id,email,password_hash,display_name,created_at) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (email) DO NOTHING").bind(user.id).bind(&user.email).bind(password_hash).bind(&user.display_name).bind(user.created_at).execute(&state.db).await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::bad("An account with that email already exists."));
    }
    issue_session(&state, user).await.map(Json)
}
async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    let row: AuthUser = sqlx::query_as("SELECT id,email,display_name,created_at,password_hash,disabled_at FROM users WHERE email=$1").bind(request.email.trim().to_lowercase()).fetch_optional(&state.db).await?.ok_or_else(ApiError::unauthorized)?;
    if row.disabled_at.is_some()
        || Argon2::default()
            .verify_password(
                request.password.as_bytes(),
                &PasswordHash::new(&row.password_hash).map_err(|_| ApiError::unauthorized())?,
            )
            .is_err()
    {
        return Err(ApiError::unauthorized());
    }
    issue_session(&state, row.into()).await.map(Json)
}
async fn refresh(
    State(state): State<AppState>,
    Json(request): Json<RefreshRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    let raw_hash = digest(&request.refresh_token);
    let row: RefreshUser = sqlx::query_as("SELECT u.id,u.email,u.display_name,u.created_at,rt.id AS refresh_id FROM refresh_tokens rt JOIN users u ON u.id=rt.user_id WHERE rt.token_hash=$1 AND rt.revoked_at IS NULL AND rt.expires_at > now() AND u.disabled_at IS NULL").bind(raw_hash).fetch_optional(&state.db).await?.ok_or_else(ApiError::unauthorized)?;
    sqlx::query("UPDATE refresh_tokens SET revoked_at=now() WHERE id=$1")
        .bind(row.refresh_id)
        .execute(&state.db)
        .await?;
    issue_session(
        &state,
        User {
            id: row.id,
            email: row.email,
            display_name: row.display_name,
            created_at: row.created_at,
        },
    )
    .await
    .map(Json)
}
async fn issue_session(state: &AppState, user: User) -> Result<AuthResponse, ApiError> {
    let refresh_token = random_token();
    sqlx::query(
        "INSERT INTO refresh_tokens (id,user_id,token_hash,expires_at) VALUES ($1,$2,$3,$4)",
    )
    .bind(Uuid::new_v4())
    .bind(user.id)
    .bind(digest(&refresh_token))
    .bind(Utc::now() + Duration::days(REFRESH_TOKEN_DAYS))
    .execute(&state.db)
    .await?;
    Ok(AuthResponse {
        access_token: token_for(state, user.id)?,
        refresh_token,
        user,
    })
}
async fn me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<User>, ApiError> {
    let id = user_from_headers(&state, &headers)?;
    Ok(Json(sqlx::query_as("SELECT id,email,display_name,created_at FROM users WHERE id=$1 AND disabled_at IS NULL").bind(id).fetch_optional(&state.db).await?.ok_or_else(ApiError::unauthorized)?))
}

async fn list_providers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Provider>>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let rows: Vec<ProviderRow> = sqlx::query_as("SELECT id,name,kind,base_url,default_model,created_at,updated_at FROM providers WHERE user_id=$1 ORDER BY name")
        .bind(user_id).fetch_all(&state.db).await?;
    let mut providers = Vec::with_capacity(rows.len());
    for row in rows { providers.push(provider_with_models(&state.db, row).await?); }
    Ok(Json(providers))
}
async fn create_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ProviderRequest>,
) -> Result<Json<Provider>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let kind = request.kind.unwrap_or_else(|| "openai_compatible".into());
    if !matches!(kind.as_str(), "openai_compatible" | "anthropic")
        || !request.base_url.starts_with("https://")
        || request.api_key.trim().is_empty()
    {
        return Err(ApiError::bad(
            "Provider type, HTTPS base URL, and API key are required.",
        ));
    }
    if request.name.trim().is_empty() {
        return Err(ApiError::bad("Provider name is required."));
    }
    let (ciphertext, nonce) = encrypt(&state, request.api_key.trim())?;
    let default_model = request.default_model.as_deref().map(str::trim).unwrap_or("").to_string();
    let mut tx = state.db.begin().await?;
    let row:ProviderRow=sqlx::query_as("INSERT INTO providers (id,user_id,name,kind,base_url,encrypted_api_key,key_nonce,default_model) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id,name,kind,base_url,default_model,created_at,updated_at").bind(Uuid::new_v4()).bind(user_id).bind(request.name.trim()).bind(kind).bind(request.base_url.trim_end_matches('/')).bind(ciphertext).bind(nonce).bind(default_model).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    let provider = provider_with_models(&state.db, row).await?;
    event(&state.db, user_id, "provider", provider.id, "created", 1).await?;
    Ok(Json(provider))
}

async fn provider_with_models(pool: &PgPool, row: ProviderRow) -> Result<Provider, ApiError> {
    let models = sqlx::query_as("SELECT id,provider_id,group_name,model,kind,sort_order,context_window,created_at,updated_at FROM provider_models WHERE provider_id=$1 ORDER BY group_name,sort_order,model")
        .bind(row.id).fetch_all(pool).await?;
    Ok(Provider { id: row.id, name: row.name, kind: row.kind, base_url: row.base_url, default_model: row.default_model, created_at: row.created_at, updated_at: row.updated_at, models })
}

async fn own_provider(pool: &PgPool, user_id: Uuid, id: Uuid) -> Result<ProviderRow, ApiError> {
    sqlx::query_as("SELECT id,name,kind,base_url,default_model,created_at,updated_at FROM providers WHERE id=$1 AND user_id=$2")
        .bind(id).bind(user_id).fetch_optional(pool).await?.ok_or_else(ApiError::not_found)
}

async fn configured_model(pool: &PgPool, provider_id: Uuid, model: &str) -> Result<bool, ApiError> {
    Ok(sqlx::query_as::<_, (bool,)>("SELECT EXISTS(SELECT 1 FROM provider_models WHERE provider_id=$1 AND model=$2)")
        .bind(provider_id).bind(model).fetch_one(pool).await?.0)
}

async fn provider_first_model(pool: &PgPool, provider_id: Uuid) -> Result<Option<String>, ApiError> {
    Ok(sqlx::query_as::<_, (String,)>("SELECT model FROM provider_models WHERE provider_id=$1 ORDER BY sort_order,model LIMIT 1")
        .bind(provider_id).fetch_optional(pool).await?.map(|value| value.0))
}

async fn provider_model_kind(pool: &PgPool, provider_id: Uuid, model: &str) -> Result<Option<String>, ApiError> {
    Ok(sqlx::query_as::<_, (String,)>("SELECT kind FROM provider_models WHERE provider_id=$1 AND model=$2 LIMIT 1")
        .bind(provider_id).bind(model).fetch_optional(pool).await?.map(|value| value.0))
}

async fn create_provider_model(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>, Json(request): Json<ProviderModelRequest>) -> Result<Json<ProviderModel>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    own_provider(&state.db, user_id, id).await?;
    if request.group_name.trim().is_empty() || request.model.trim().is_empty() { return Err(ApiError::bad("Model group and model name are required.")); }
    if !matches!(request.kind.as_str(), "openai_compatible" | "anthropic") { return Err(ApiError::bad("Model API format must be OpenAI-compatible or Anthropic.")); }
    let item = sqlx::query_as("INSERT INTO provider_models (id,provider_id,group_name,model,kind,sort_order,context_window) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id,provider_id,group_name,model,kind,sort_order,context_window,created_at,updated_at")
        .bind(Uuid::new_v4()).bind(id).bind(request.group_name.trim()).bind(request.model.trim()).bind(request.kind).bind(request.sort_order.unwrap_or(0)).bind(request.context_window.unwrap_or(128_000).clamp(4096, 2_000_000)).fetch_one(&state.db).await?;
    event(&state.db, user_id, "provider", id, "updated", 1).await?;
    Ok(Json(item))
}

async fn update_provider_model(State(state): State<AppState>, headers: HeaderMap, Path((id, model_id)): Path<(Uuid, Uuid)>, Json(request): Json<UpdateProviderModelRequest>) -> Result<Json<ProviderModel>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    own_provider(&state.db, user_id, id).await?;
    if request.group_name.as_ref().is_some_and(|v| v.trim().is_empty()) || request.model.as_ref().is_some_and(|v| v.trim().is_empty()) { return Err(ApiError::bad("Model group and model name cannot be empty.")); }
    if request.kind.as_deref().is_some_and(|v| !matches!(v, "openai_compatible" | "anthropic")) { return Err(ApiError::bad("Model API format must be OpenAI-compatible or Anthropic.")); }
    let previous: (String,) = sqlx::query_as("SELECT model FROM provider_models WHERE id=$1 AND provider_id=$2").bind(model_id).bind(id).fetch_optional(&state.db).await?.ok_or_else(ApiError::not_found)?;
    let item: ProviderModel = sqlx::query_as("UPDATE provider_models SET group_name=COALESCE($3,group_name),model=COALESCE($4,model),kind=COALESCE($5,kind),sort_order=COALESCE($6,sort_order),context_window=COALESCE($7,context_window) WHERE id=$1 AND provider_id=$2 RETURNING id,provider_id,group_name,model,kind,sort_order,context_window,created_at,updated_at")
        .bind(model_id).bind(id).bind(request.group_name.map(|v| v.trim().to_string())).bind(request.model.map(|v| v.trim().to_string())).bind(request.kind).bind(request.sort_order).bind(request.context_window.map(|value| value.clamp(4096, 2_000_000))).fetch_optional(&state.db).await?.ok_or_else(ApiError::not_found)?;
    sqlx::query("UPDATE providers SET default_model=$3 WHERE id=$1 AND default_model=$2").bind(id).bind(previous.0).bind(&item.model).execute(&state.db).await?;
    event(&state.db, user_id, "provider", id, "updated", 1).await?;
    Ok(Json(item))
}

async fn delete_provider_model(State(state): State<AppState>, headers: HeaderMap, Path((id, model_id)): Path<(Uuid, Uuid)>) -> Result<StatusCode, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    own_provider(&state.db, user_id, id).await?;
    let removed: Option<(String,)> = sqlx::query_as("DELETE FROM provider_models WHERE id=$1 AND provider_id=$2 RETURNING model").bind(model_id).bind(id).fetch_optional(&state.db).await?;
    let removed = removed.ok_or_else(ApiError::not_found)?;
    sqlx::query("UPDATE providers SET default_model=(SELECT model FROM provider_models WHERE provider_id=$1 ORDER BY sort_order,model LIMIT 1) WHERE id=$1 AND default_model=$2").bind(id).bind(removed.0).execute(&state.db).await?;
    event(&state.db, user_id, "provider", id, "updated", 1).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let result = sqlx::query("DELETE FROM providers WHERE id=$1 AND user_id=$2")
        .bind(id)
        .bind(user_id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    };
    event(&state.db, user_id, "provider", id, "deleted", 1).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_conversations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<Conversation>>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let limit = page
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let cursor = page
        .cursor
        .as_deref()
        .and_then(|s| s.parse::<DateTime<Utc>>().ok());
    let rows=sqlx::query_as::<_,Conversation>("SELECT id,title,model_provider_id,model,context_window,context_tokens,generation_settings,revision,created_at,updated_at FROM conversations WHERE user_id=$1 AND archived_at IS NULL AND ($2::timestamptz IS NULL OR updated_at < $2) ORDER BY updated_at DESC,id DESC LIMIT $3").bind(user_id).bind(cursor).bind(limit+1).fetch_all(&state.db).await?;
    let next = rows.get(limit as usize).map(|c| c.updated_at.to_rfc3339());
    Ok(Json(Page {
        items: rows.into_iter().take(limit as usize).collect(),
        next_cursor: next,
    }))
}
async fn create_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateConversation>,
) -> Result<Json<Conversation>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let (model, context_window) = if let Some(pid) = request.provider_id {
        own_provider(&state.db, user_id, pid).await.map_err(|_| ApiError::bad("Selected provider does not belong to this account."))?;
        let model = match request.model {
            Some(value) => value,
            None => provider_first_model(&state.db, pid).await?.ok_or_else(|| ApiError::bad("Add a configured model to this provider before creating a chat."))?,
        };
        if !configured_model(&state.db, pid, &model).await? { return Err(ApiError::bad("Choose a configured model for this provider.")); }
        let context_window: (i32,) = sqlx::query_as("SELECT context_window FROM provider_models WHERE provider_id=$1 AND model=$2").bind(pid).bind(&model).fetch_one(&state.db).await?;
        (Some(model), context_window.0)
    } else { (request.model, 128_000) };
    let conversation:Conversation=sqlx::query_as("INSERT INTO conversations (id,user_id,title,model_provider_id,model,context_window) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id,title,model_provider_id,model,context_window,context_tokens,generation_settings,revision,created_at,updated_at").bind(Uuid::new_v4()).bind(user_id).bind(request.title.unwrap_or_else(||"New chat".into()).trim()).bind(request.provider_id).bind(model).bind(context_window).fetch_one(&state.db).await?;
    event(
        &state.db,
        user_id,
        "conversation",
        conversation.id,
        "created",
        conversation.revision,
    )
    .await?;
    Ok(Json(conversation))
}
async fn get_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Conversation>, ApiError> {
    Ok(Json(
        own_conversation(&state.db, user_from_headers(&state, &headers)?, id).await?,
    ))
}
async fn update_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateConversation>,
) -> Result<Json<Conversation>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let existing = own_conversation(&state.db, user_id, id).await?;
    let generation_settings = request.generation_settings.map(validate_generation_settings).transpose()?;
    let model_changed = request.model.is_some();
    let model = request.model.map(|value| value.trim().to_string());
    if model.as_ref().is_some_and(String::is_empty) {
        return Err(ApiError::bad("Model name cannot be empty."));
    }
    let provider_id = request.provider_id.or(existing.model_provider_id);
    let selected_model = model.as_deref().or(existing.model.as_deref());
    let mut selected_context_window = None;
    if let (Some(provider_id), Some(model)) = (provider_id, selected_model) {
        own_provider(&state.db, user_id, provider_id).await.map_err(|_| ApiError::bad("Selected provider does not belong to this account."))?;
        if !configured_model(&state.db, provider_id, model).await? { return Err(ApiError::bad("Choose a configured model for this provider.")); }
        if request.provider_id.is_some() || model_changed {
            selected_context_window = Some(sqlx::query_as::<_, (i32,)>("SELECT context_window FROM provider_models WHERE provider_id=$1 AND model=$2").bind(provider_id).bind(model).fetch_one(&state.db).await?.0);
        }
    }
    let c:Conversation=sqlx::query_as("UPDATE conversations SET title=COALESCE($3,title),archived_at=CASE WHEN $4::boolean IS TRUE THEN now() WHEN $4::boolean IS FALSE THEN NULL ELSE archived_at END,model_provider_id=COALESCE($5,model_provider_id),model=COALESCE($6,model),context_window=COALESCE($7,context_window),generation_settings=COALESCE($8,generation_settings),revision=revision+1 WHERE id=$1 AND user_id=$2 RETURNING id,title,model_provider_id,model,context_window,context_tokens,generation_settings,revision,created_at,updated_at").bind(id).bind(user_id).bind(request.title.map(|v|v.trim().to_string())).bind(request.archived).bind(request.provider_id).bind(model).bind(selected_context_window).bind(generation_settings).fetch_one(&state.db).await?;
    event(
        &state.db,
        user_id,
        "conversation",
        id,
        "updated",
        c.revision,
    )
    .await?;
    Ok(Json(c))
}
async fn delete_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let r = sqlx::query("DELETE FROM conversations WHERE id=$1 AND user_id=$2")
        .bind(id)
        .bind(user_id)
        .execute(&state.db)
        .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    };
    event(&state.db, user_id, "conversation", id, "deleted", 0).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<Message>>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    own_conversation(&state.db, user_id, id).await?;
    let limit = page
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let cursor = page.cursor.as_deref().and_then(|s| s.parse::<i64>().ok());
    let mut rows=sqlx::query_as::<_,Message>("SELECT id,conversation_id,sequence,client_mutation_id,role,content,reasoning_content,content_format,status,model,token_count,search_sources,edited_at,created_at,updated_at FROM messages WHERE conversation_id=$1 AND deleted_at IS NULL AND ($2::bigint IS NULL OR sequence < $2) ORDER BY sequence DESC LIMIT $3").bind(id).bind(cursor).bind(limit+1).fetch_all(&state.db).await?;
    let next = rows.get(limit as usize).map(|m| m.sequence.to_string());
    rows.truncate(limit as usize);
    rows.reverse();
    Ok(Json(Page {
        items: rows,
        next_cursor: next,
    }))
}
async fn create_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(request): Json<CreateMessage>,
) -> Result<Json<Message>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let c = own_conversation(&state.db, user_id, id).await?;
    let content = request.content.trim();
    if content.is_empty() || content.len() > 200_000 {
        return Err(ApiError::bad(
            "Message must contain 1 to 200,000 characters.",
        ));
    };
    if let Some(existing)=sqlx::query_as::<_,Message>("SELECT id,conversation_id,sequence,client_mutation_id,role,content,reasoning_content,content_format,status,model,token_count,search_sources,edited_at,created_at,updated_at FROM messages WHERE conversation_id=$1 AND client_mutation_id=$2").bind(id).bind(request.client_mutation_id).fetch_optional(&state.db).await? { return Ok(Json(existing)); }
    let mut tx = state.db.begin().await?;
    let sequence:(i64,)=sqlx::query_as("UPDATE conversations SET next_sequence=next_sequence+1,context_tokens=context_tokens+$3,revision=revision+1 WHERE id=$1 AND user_id=$2 RETURNING next_sequence-1").bind(id).bind(user_id).bind(estimate_tokens(content)).fetch_one(&mut *tx).await?;
    let m:Message=sqlx::query_as("INSERT INTO messages (id,conversation_id,sequence,client_mutation_id,role,content,token_count,search_sources) VALUES ($1,$2,$3,$4,'user',$5,$6,$7) RETURNING id,conversation_id,sequence,client_mutation_id,role,content,reasoning_content,content_format,status,model,token_count,search_sources,edited_at,created_at,updated_at").bind(Uuid::new_v4()).bind(id).bind(sequence.0).bind(request.client_mutation_id).bind(content).bind(estimate_tokens(content)).bind(if request.search.unwrap_or(false){json!([])}else{json!([])}).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    event(
        &state.db,
        user_id,
        "message",
        m.id,
        "created",
        c.revision + 1,
    )
    .await?;
    Ok(Json(m))
}
async fn update_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, message_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<UpdateMessage>,
) -> Result<Json<Message>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    own_conversation(&state.db, user_id, id).await?;
    let content = request.content.trim();
    if content.is_empty() {
        return Err(ApiError::bad("Message cannot be empty."));
    }
    let m:Message=sqlx::query_as("UPDATE messages SET content=$3,token_count=$4,edited_at=now() WHERE id=$1 AND conversation_id=$2 AND deleted_at IS NULL RETURNING id,conversation_id,sequence,client_mutation_id,role,content,reasoning_content,content_format,status,model,token_count,search_sources,edited_at,created_at,updated_at").bind(message_id).bind(id).bind(content).bind(estimate_tokens(content)).fetch_optional(&state.db).await?.ok_or_else(ApiError::not_found)?;
    event(
        &state.db, user_id, "message", message_id, "updated", m.sequence,
    )
    .await?;
    Ok(Json(m))
}
async fn delete_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, message_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    own_conversation(&state.db, user_id, id).await?;
    let r=sqlx::query("UPDATE messages SET deleted_at=now() WHERE id=$1 AND conversation_id=$2 AND deleted_at IS NULL").bind(message_id).bind(id).execute(&state.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    };
    event(&state.db, user_id, "message", message_id, "deleted", 0).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn respond(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(request): Json<RespondRequest>,
) -> Result<Response, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let conversation = own_conversation(&state.db, user_id, id).await?;
    let input:Message=sqlx::query_as("SELECT id,conversation_id,sequence,client_mutation_id,role,content,reasoning_content,content_format,status,model,token_count,search_sources,edited_at,created_at,updated_at FROM messages WHERE id=$1 AND conversation_id=$2 AND role='user' AND deleted_at IS NULL").bind(request.message_id).bind(id).fetch_optional(&state.db).await?.ok_or_else(ApiError::not_found)?;
    let provider_id = conversation
        .model_provider_id
        .ok_or_else(|| ApiError::bad("Choose a provider before requesting a response."))?;
    let p:(String,String,String,Vec<u8>,Vec<u8>)=sqlx::query_as("SELECT kind,base_url,default_model,encrypted_api_key,key_nonce FROM providers WHERE id=$1 AND user_id=$2").bind(provider_id).bind(user_id).fetch_optional(&state.db).await?.ok_or_else(||ApiError::bad("The selected provider was removed."))?;
    let prior_summary: Option<(String, i64)> = sqlx::query_as("SELECT content,ends_at_sequence FROM conversation_summaries WHERE conversation_id=$1 ORDER BY ends_at_sequence DESC LIMIT 1").bind(id).fetch_optional(&state.db).await?;
    let messages:Vec<(String,String)>=sqlx::query_as("SELECT role,content FROM messages WHERE conversation_id=$1 AND sequence > $2 AND deleted_at IS NULL AND status='complete' AND role <> 'summary' ORDER BY sequence DESC LIMIT 80").bind(id).bind(prior_summary.as_ref().map(|summary| summary.1).unwrap_or(0)).fetch_all(&state.db).await?;
    let mut transcript: Vec<Value> = messages
        .into_iter()
        .rev()
        .map(|(role, content)| json!({"role":role,"content":strip_thinking(&content)}))
        .collect();
    let api_key = decrypt(&state, &p.3, &p.4)?;
    let model = match conversation.model.as_deref().filter(|value| !value.trim().is_empty()) {
        Some(value) => value.to_string(),
        None => provider_first_model(&state.db, provider_id).await?.unwrap_or_else(|| p.2.clone()),
    };
    let kind = provider_model_kind(&state.db, provider_id, &model).await?.unwrap_or_else(|| p.0.clone());
    let explicit_search = content_requests_web_search(&input.content);
    let search_requested = request.search.unwrap_or(false) || explicit_search;
    info!(conversation_id=%id, message_id=%input.id, search_toggle=request.search.unwrap_or(false), explicit_search, stream=request.stream.unwrap_or(false), "response request received");
    let (sources, planner_returned_no_queries) = if search_requested && should_search_web(&input.content) {
        // Search is deliberately a two-call workflow: plan queries, collect sources,
        // then make the answering call below with those sources as context.
        let queries = generate_search_queries(&state.http, &kind, &p.1, &api_key, &model, &transcript, &input.content).await?;
        info!(conversation_id=%id, planned_queries=queries.len(), "search planner completed");
        let planner_returned_no_queries = queries.is_empty();
        let mut merged = Vec::new();
        for query in queries.into_iter().take(4) {
            match fetch_search(&state, &query).await {
                Ok(results) => for mut result in results {
                result["query"] = json!(query);
                if !merged.iter().any(|existing: &Value| existing["url"] == result["url"]) { merged.push(result); }
                },
                Err(cause) => warn!(conversation_id=%id, error=%cause.message, "search provider request failed"),
            }
        }
        merged.truncate(8);
        info!(conversation_id=%id, source_count=merged.len(), "search collection completed");
        (merged, planner_returned_no_queries)
    } else { (vec![], false) };
    if let Some((summary, _)) = prior_summary {
        transcript.insert(0, json!({"role":"system","content":format!("Previous conversation context, compressed by malim_chat:\n{summary}")}));
    }
    if search_requested && !sources.is_empty() {
        transcript.insert(0,json!({"role":"system","content":format!("Web search is enabled. Answer the latest user question using only relevant sources below. Cite supporting claims with their source URLs. Do not claim that web sources are unavailable when they are listed. Sources: {}",serde_json::to_string(&sources).unwrap_or_default())}));
    } else if search_requested && planner_returned_no_queries {
        transcript.insert(0,json!({"role":"system","content":"Web search was enabled. The search planner determined that no external lookup was needed for this request. Do not claim to have searched or cite web sources."}));
    } else if search_requested {
        transcript.insert(0,json!({"role":"system","content":"Web search was requested and completed, but the search engine returned no usable sources. State that limitation plainly; do not invent sources or claim the search was not attempted."}));
    }
    let enable_markdown = request.enable_markdown.unwrap_or(true);
    transcript.insert(0, json!({"role":"system","content": if enable_markdown { "Format the answer with GitHub-flavored Markdown when it improves readability. Use fenced code blocks with a language label for code." } else { "Respond in plain text only. Do not use Markdown syntax, headings, lists, tables, fenced code blocks, or inline formatting markers." }}));
    if request.stream.unwrap_or(false) {
        let upstream = call_provider_stream(&state.http, &kind, &p.1, &api_key, &model, &transcript, request.temperature, request.reasoning_effort.as_deref()).await?;
        return Ok(stream_response(state, upstream, id, user_id, model, sources, enable_markdown));
    }
    let (answer, reasoning) = split_thinking(&call_provider(&state.http, &kind, &p.1, &api_key, &model, &transcript, request.temperature, request.reasoning_effort.as_deref()).await?);
    let m = persist_assistant_message(&state, id, user_id, &model, answer, reasoning, &sources, enable_markdown).await?;
    Ok(Json(m).into_response())
}

async fn persist_assistant_message(state: &AppState, conversation_id: Uuid, user_id: Uuid, model: &str, answer: String, reasoning: String, sources: &[Value], enable_markdown: bool) -> Result<Message, ApiError> {
    let tokens = estimate_tokens(&answer);
    let mut tx = state.db.begin().await?;
    let seq:(i64,)=sqlx::query_as("UPDATE conversations SET next_sequence=next_sequence+1,context_tokens=context_tokens+$3,revision=revision+1 WHERE id=$1 AND user_id=$2 RETURNING next_sequence-1").bind(conversation_id).bind(user_id).bind(tokens).fetch_one(&mut *tx).await?;
    let m:Message=sqlx::query_as("INSERT INTO messages (id,conversation_id,sequence,role,content,reasoning_content,content_format,model,token_count,search_sources) VALUES ($1,$2,$3,'assistant',$4,$5,$6,$7,$8,$9) RETURNING id,conversation_id,sequence,client_mutation_id,role,content,reasoning_content,content_format,status,model,token_count,search_sources,edited_at,created_at,updated_at").bind(Uuid::new_v4()).bind(conversation_id).bind(seq.0).bind(answer).bind(reasoning).bind(if enable_markdown { "markdown" } else { "plain" }).bind(model).bind(tokens).bind(json!(sources)).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    event(&state.db, user_id, "message", m.id, "created", seq.0).await?;
    let _ = auto_compact(&state.db, conversation_id).await;
    Ok(m)
}

async fn auto_compact(pool: &PgPool, conversation_id: Uuid) -> Result<(), ApiError> {
    let values: (i32, i32, i64) = sqlx::query_as(
        "SELECT context_tokens,context_window,next_sequence FROM conversations WHERE id=$1",
    )
    .bind(conversation_id)
    .fetch_one(pool)
    .await?;
    if values.0 < (values.1 as f32 * 0.70) as i32 {
        return Ok(());
    }
    let previous: Option<(String, i64)> = sqlx::query_as("SELECT content,ends_at_sequence FROM conversation_summaries WHERE conversation_id=$1 ORDER BY ends_at_sequence DESC LIMIT 1").bind(conversation_id).fetch_optional(pool).await?;
    let existing_end = previous.as_ref().map(|summary| summary.1).unwrap_or(0);
    let end = values.2 - 1;
    if end <= existing_end {
        return Ok(());
    }
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT role,content FROM messages WHERE conversation_id=$1 AND sequence > $2 AND sequence <= $3 AND deleted_at IS NULL ORDER BY sequence").bind(conversation_id).bind(existing_end).bind(end).fetch_all(pool).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let compacted = rows
        .iter()
        .map(|(role, content)| format!("{role}: {}", strip_thinking(content)))
        .collect::<Vec<_>>()
        .join("\n");
    let content = format!(
        "Conversation summary through message {end}:\n{}\n{}",
        previous.map(|summary| summary.0).unwrap_or_default(),
        compacted
    )
    .chars()
    .take(30_000)
    .collect::<String>();
    sqlx::query("INSERT INTO conversation_summaries (id,conversation_id,starts_at_sequence,ends_at_sequence,content,token_count) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (conversation_id,starts_at_sequence,ends_at_sequence) DO UPDATE SET content=EXCLUDED.content,token_count=EXCLUDED.token_count").bind(Uuid::new_v4()).bind(conversation_id).bind(existing_end + 1).bind(end).bind(&content).bind(estimate_tokens(&content)).execute(pool).await?;
    sqlx::query("UPDATE conversations SET context_tokens=$2 WHERE id=$1")
        .bind(conversation_id)
        .bind(estimate_tokens(&content))
        .execute(pool)
        .await?;
    Ok(())
}
async fn compact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(request): Json<CompactRequest>,
) -> Result<Json<Value>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let c = own_conversation(&state.db, user_id, id).await?;
    if !request.force.unwrap_or(false) && c.context_tokens < (c.context_window as f32 * 0.70) as i32
    {
        return Ok(Json(
            json!({"compacted":false,"reason":"below_threshold","context_tokens":c.context_tokens,"context_window":c.context_window}),
        ));
    }
    let last:(i64,)=sqlx::query_as("SELECT COALESCE(MAX(sequence),0) FROM messages WHERE conversation_id=$1 AND deleted_at IS NULL").bind(id).fetch_one(&state.db).await?;
    let end = request.through_sequence.unwrap_or(last.0);
    let rows:Vec<(String,String)>=sqlx::query_as("SELECT role,content FROM messages WHERE conversation_id=$1 AND sequence <= $2 AND deleted_at IS NULL ORDER BY sequence").bind(id).bind(end).fetch_all(&state.db).await?;
    if rows.is_empty() {
        return Err(ApiError::bad("There are no messages to compact."));
    }
    let source = rows
        .iter()
        .map(|(role, content)| format!("{role}: {}", strip_thinking(content)))
        .collect::<Vec<_>>()
        .join("\n");
    let prior: Option<(String, i64)> = sqlx::query_as("SELECT content,ends_at_sequence FROM conversation_summaries WHERE conversation_id=$1 ORDER BY ends_at_sequence DESC LIMIT 1").bind(id).fetch_optional(&state.db).await?;
    let model_summary = if let Some(provider_id) = c.model_provider_id {
        let p:(String,String,String,Vec<u8>,Vec<u8>)=sqlx::query_as("SELECT kind,base_url,default_model,encrypted_api_key,key_nonce FROM providers WHERE id=$1 AND user_id=$2").bind(provider_id).bind(user_id).fetch_optional(&state.db).await?.ok_or_else(|| ApiError::bad("The selected provider was removed."))?;
        let api_key = decrypt(&state, &p.3, &p.4)?;
        let model = match c.model.as_deref().filter(|value| !value.trim().is_empty()) {
            Some(value) => value.to_string(),
            None => provider_first_model(&state.db, provider_id).await?.unwrap_or_else(|| p.2.clone()),
        };
        let kind = provider_model_kind(&state.db, provider_id, &model).await?.unwrap_or_else(|| p.0.clone());
        info!(conversation_id=%id, through_sequence=end, source_characters=source.len(), "manual compaction started");
        call_provider(&state.http, &kind, &p.1, &api_key, &model, &[json!({"role":"system","content":"Create a concise, factual memory for continuing this conversation. Preserve decisions, constraints, user preferences, unresolved tasks, and important technical details. Do not use Markdown."}), json!({"role":"user","content":format!("Previous memory:\n{}\n\nConversation to compact:\n{}", prior.as_ref().map(|item| item.0.as_str()).unwrap_or(""), source.chars().take(120_000).collect::<String>())})], Some(0.2), None).await?
    } else {
        source.chars().take(30_000).collect()
    };
    let concise = format!("Conversation summary through message {end}:\n{model_summary}");
    let summary_id = Uuid::new_v4();
    let summary_tokens = estimate_tokens(&concise);
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO conversation_summaries (id,conversation_id,starts_at_sequence,ends_at_sequence,content,token_count) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (conversation_id,starts_at_sequence,ends_at_sequence) DO UPDATE SET content=EXCLUDED.content,token_count=EXCLUDED.token_count").bind(summary_id).bind(id).bind(prior.as_ref().map(|item| item.1 + 1).unwrap_or(1)).bind(end).bind(&concise).bind(summary_tokens).execute(&mut *tx).await?;
    let sequence:(i64,)=sqlx::query_as("UPDATE conversations SET next_sequence=next_sequence+1,context_tokens=$3,revision=revision+1 WHERE id=$1 AND user_id=$2 RETURNING next_sequence-1").bind(id).bind(user_id).bind(summary_tokens).fetch_one(&mut *tx).await?;
    let marker = format!("Context compacted through message {end}. The conversation now uses a durable summary ({summary_tokens} estimated tokens).");
    let message:Message=sqlx::query_as("INSERT INTO messages (id,conversation_id,sequence,role,content,content_format,token_count) VALUES ($1,$2,$3,'summary',$4,'plain',0) RETURNING id,conversation_id,sequence,client_mutation_id,role,content,reasoning_content,content_format,status,model,token_count,search_sources,edited_at,created_at,updated_at").bind(Uuid::new_v4()).bind(id).bind(sequence.0).bind(marker).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    event(&state.db, user_id, "message", message.id, "created", sequence.0).await?;
    info!(conversation_id=%id, through_sequence=end, summary_tokens, "manual compaction completed");
    Ok(Json(
        json!({"compacted":true,"summary_id":summary_id,"through_sequence":end,"estimated_tokens":summary_tokens,"message":message,"context_tokens":summary_tokens}),
    ))
}
async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<Value>>, ApiError> {
    user_from_headers(&state, &headers)?;
    if query.q.trim().is_empty() {
        return Err(ApiError::bad("Search query cannot be empty."));
    };
    Ok(Json(fetch_search(&state, &query.q).await?))
}

async fn dictionary_lookup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DictionaryQuery>,
) -> Result<Json<DictionaryResponse>, ApiError> {
    user_from_headers(&state, &headers)?;
    let word = query.word.trim().to_string();
    if word.is_empty() || word.chars().count() > 120 {
        return Err(ApiError::bad(
            "Select a word or short phrase of up to 120 characters.",
        ));
    }
    let dictionary = query.dictionary;
    if !matches!(
        dictionary.as_str(),
        "russian_en" | "german_en" | "english_zh"
    ) {
        return Err(ApiError::bad("Unsupported dictionary."));
    }
    let directory = (*state.dictionary_dir).clone();
    let russian_dictionary = Arc::clone(&state.russian_dictionary);
    let response =
        tokio::task::spawn_blocking(move || lookup_dictionary(&directory, &russian_dictionary, &dictionary, &word))
            .await
            .map_err(|_| ApiError::internal("Dictionary worker did not complete."))??;
    Ok(Json(response))
}

fn lookup_dictionary(
    directory: &std::path::Path,
    russian_dictionary: &Mutex<Mdx>,
    dictionary: &str,
    word: &str,
) -> Result<DictionaryResponse, ApiError> {
    if !directory.is_dir() {
        return Err(ApiError::internal(
            "Local dictionaries are not installed on this server.",
        ));
    }
    let entries = match dictionary {
        "russian_en" => lookup_russian_dictionary(russian_dictionary, word)?,
        "german_en" => lookup_german_dictionary(directory, word)?,
        "english_zh" => lookup_english_chinese_dictionary(directory, word)?,
        _ => unreachable!(),
    };
    Ok(DictionaryResponse {
        word: word.to_string(),
        dictionary: dictionary.to_string(),
        entries,
    })
}

fn normalize_dictionary_key(value: &str) -> String {
    value
        .trim()
        .replace('\u{0301}', "")
        .to_lowercase()
        .chars()
        .filter(|character| character.is_alphanumeric() || *character == 'ё')
        .collect()
}

fn split_lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn lookup_russian_dictionary(
    dictionary: &Mutex<Mdx>,
    word: &str,
) -> Result<Vec<DictionaryEntryResponse>, ApiError> {
    let mut mdx = dictionary.lock().map_err(|_| ApiError::internal("Russian dictionary is unavailable."))?;
    let mut terms = vec![word.trim().to_lowercase()];
    let alternate = terms[0].replace('ё', "е");
    if alternate != terms[0] { terms.push(alternate); }
    for term in terms {
        if let Some(entry) = russian_entry(&mut mdx, &term, None) { return Ok(vec![entry]); }
    }
    Ok(Vec::new())
}

fn russian_entry(mdx: &mut Mdx, term: &str, linked_from: Option<&str>) -> Option<DictionaryEntryResponse> {
    let normalized = normalize_dictionary_key(term);
    let entries = russian_exact_entries(mdx, &normalized);
    for item in &entries {
        let lookup = mdx.fetch(item)?;
        if let Some(target) = russian_link_target(&lookup.definition) {
            if normalize_dictionary_key(&target) != normalized {
                return russian_entry(mdx, &target, Some(&lookup.key_text));
            }
        }
        return russian_response(lookup.key_text, &lookup.definition, linked_from);
    }
    let lookup = mdx.lookup(term)?;
    if let Some(target) = russian_link_target(&lookup.definition) {
        if normalize_dictionary_key(&target) != normalized { return russian_entry(mdx, &target, Some(&lookup.key_text)); }
    }
    russian_response(lookup.key_text, &lookup.definition, linked_from)
}

fn russian_exact_entries(mdx: &Mdx, target: &str) -> Vec<KeyWordItem> {
    let list = mdx.keyword_list();
    let mut lo = 0;
    let mut hi = list.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if normalize_dictionary_key(&list[mid].key_text).as_str() < target { lo = mid + 1; } else { hi = mid; }
    }
    let start = lo;
    while lo < list.len() && normalize_dictionary_key(&list[lo].key_text) == target { lo += 1; }
    list[start..lo].to_vec()
}

fn russian_response(headword: String, definition: &str, linked_from: Option<&str>) -> Option<DictionaryEntryResponse> {
    let mut labels = vec!["OpenRussian".into()];
    if let Some(source) = linked_from { labels.push(format!("Lemma for {source}")); }
    Some(DictionaryEntryResponse {
        lemma: headword.clone(),
        matched_terms: linked_from.into_iter().map(str::to_string).collect(),
        headword,
        pronunciation: String::new(),
        definitions: Vec::new(),
        translations: Vec::new(),
        forms: Vec::new(),
        labels,
        examples: Vec::new(),
        detail: Value::Null,
        definition_html: definition.to_string(),
    })
}

fn russian_link_target(definition: &str) -> Option<String> {
    definition
        .split("@@@LINK=")
        .nth(1)?
        .split("@@@")
        .next()?
        .split_whitespace()
        .next()
        .map(|value| value.trim_matches(|ch: char| matches!(ch, '"' | '\'' | '<' | '>')).to_lowercase())
}

fn german_database_path(directory: &std::path::Path) -> Result<PathBuf, ApiError> {
    let source = directory.join("german_en.sqlite.gz");
    let target = std::env::temp_dir().join("malim_chat_german_en.sqlite");
    if !target.exists() {
        let source_file = std::fs::File::open(source)
            .map_err(|_| ApiError::internal("German dictionary is not installed."))?;
        let mut decoder = GzDecoder::new(source_file);
        let mut output = std::fs::File::create(&target)
            .map_err(|_| ApiError::internal("German dictionary cache could not be created."))?;
        std::io::copy(&mut decoder, &mut output)
            .map_err(|_| ApiError::internal("German dictionary could not be unpacked."))?;
    }
    Ok(target)
}

fn lookup_german_dictionary(
    directory: &std::path::Path,
    word: &str,
) -> Result<Vec<DictionaryEntryResponse>, ApiError> {
    let connection = Connection::open(german_database_path(directory)?)
        .map_err(|_| ApiError::internal("German dictionary could not be opened."))?;
    let key = normalize_dictionary_key(word).replace('ß', "ss");
    let mut statement = connection.prepare("SELECT e.headword,e.lemma,e.forms_json,e.definition_html FROM german_lookup l JOIN german_entries e ON e.id=l.entry_id WHERE l.form_key=?1 LIMIT 12")
        .map_err(|_| ApiError::internal("German dictionary schema is invalid."))?;
    let rows = statement
        .query_map([key.clone()], |row| {
            let forms: String = row.get(2)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                forms,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|_| ApiError::internal("German dictionary query failed."))?;
    let mut entries = rows
        .filter_map(Result::ok)
        .map(
            |(headword, lemma, forms, definition)| DictionaryEntryResponse {
                lemma: lemma.clone(),
                matched_terms: vec![word.to_string()],
                headword,
                pronunciation: String::new(),
                definitions: Vec::new(),
                translations: Vec::new(),
                forms: serde_json::from_str::<Vec<String>>(&forms).unwrap_or_else(|_| vec![lemma]),
                labels: vec!["Kaikki German".into()],
                examples: Vec::new(),
                detail: Value::Null,
                definition_html: definition,
            },
        )
        .collect::<Vec<_>>();
    if entries.is_empty() {
        let mut fallback = connection.prepare("SELECT headword,lemma,forms_json,definition_html FROM german_entries WHERE headword_key LIKE ?1 LIMIT 12").map_err(|_| ApiError::internal("German dictionary query failed."))?;
        let rows = fallback
            .query_map([format!("{key}%")], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|_| ApiError::internal("German dictionary query failed."))?;
        entries = rows
            .filter_map(Result::ok)
            .map(
                |(headword, lemma, forms, definition)| DictionaryEntryResponse {
                    lemma: lemma.clone(),
                    matched_terms: vec![word.to_string()],
                    headword,
                    pronunciation: String::new(),
                    definitions: Vec::new(),
                    translations: Vec::new(),
                    forms: serde_json::from_str(&forms).unwrap_or_else(|_| vec![lemma]),
                    labels: vec!["Kaikki German".into()],
                    examples: Vec::new(),
                    detail: Value::Null,
                    definition_html: definition,
                },
            )
            .collect();
    }
    Ok(entries)
}

fn lookup_english_chinese_dictionary(
    directory: &std::path::Path,
    word: &str,
) -> Result<Vec<DictionaryEntryResponse>, ApiError> {
    let connection = Connection::open(directory.join("ecdict_en_zh.sqlite"))
        .map_err(|_| ApiError::internal("English-Chinese dictionary could not be opened."))?;
    let normalized = word.trim().to_lowercase();
    let mut statement = connection.prepare("SELECT word,phonetic,definition,translation,pos,collins,oxford,tags,bnc,frequency,exchange,detail FROM entries WHERE word = ?1 COLLATE NOCASE UNION ALL SELECT word,phonetic,definition,translation,pos,collins,oxford,tags,bnc,frequency,exchange,detail FROM entries WHERE word LIKE ?2 COLLATE NOCASE LIMIT 12")
        .map_err(|_| ApiError::internal("English-Chinese dictionary schema is invalid."))?;
    let rows = statement
        .query_map(params![normalized, format!("{normalized}%")], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i32>(5)?,
                row.get::<_, i32>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i32>(8)?,
                row.get::<_, i32>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
            ))
        })
        .map_err(|_| ApiError::internal("English-Chinese dictionary query failed."))?;
    Ok(rows
        .filter_map(Result::ok)
        .map(
            |(
                headword,
                pronunciation,
                definition,
                translation,
                pos,
                collins,
                oxford,
                tags,
                bnc,
                frequency,
                exchange,
                detail,
            )| {
                let mut labels = split_lines(&tags);
                if !pos.is_empty() {
                    labels.push(format!("POS: {pos}"));
                }
                if collins > 0 {
                    labels.push(format!("Collins {collins}"));
                }
                if oxford > 0 {
                    labels.push("Oxford 3000".into());
                }
                if bnc > 0 {
                    labels.push(format!("BNC #{bnc}"));
                }
                if frequency > 0 {
                    labels.push(format!("Modern frequency #{frequency}"));
                }
                let forms = exchange
                    .split('/')
                    .filter_map(|item| item.split_once(':'))
                    .map(|(kind, value)| {
                        format!(
                            "{}: {}",
                            match kind {
                                "p" => "past",
                                "d" => "past participle",
                                "i" => "present participle",
                                "3" => "third-person singular",
                                "r" => "comparative",
                                "t" => "superlative",
                                "s" => "plural",
                                "0" => "lemma",
                                _ => kind,
                            },
                            value
                        )
                    })
                    .collect();
                let detail_value = serde_json::from_str(&detail).unwrap_or_else(|_| {
                    if detail.is_empty() {
                        Value::Null
                    } else {
                        Value::String(detail)
                    }
                });
                DictionaryEntryResponse {
                    lemma: headword.clone(),
                    matched_terms: vec![word.to_string()],
                    headword,
                    pronunciation,
                    definitions: split_lines(&definition),
                    translations: split_lines(&translation),
                    forms,
                    labels,
                    examples: Vec::new(),
                    detail: detail_value,
                    definition_html: String::new(),
                }
            },
        )
        .collect())
}

async fn generate_search_queries(
    http: &Client,
    kind: &str,
    base: &str,
    key: &str,
    model: &str,
    history: &[Value],
    question: &str,
) -> Result<Vec<String>, ApiError> {
    let mut lines: Vec<String> = history
        .iter()
        .filter_map(|m| {
            let role = m["role"].as_str()?;
            let content = m["content"].as_str()?;
            Some(format!("{role}: {content}"))
        })
        .collect();
    let question_last = history
        .last()
        .is_some_and(|m| m["role"].as_str() == Some("user") && m["content"].as_str() == Some(question));
    if !question_last {
        lines.push(format!("user: {question}"));
    }
    let messages = vec![
        json!({"role":"system","content":"You plan web searches for the LATEST user message in a conversation. Use the earlier conversation history for context when choosing queries. Return ONLY a JSON array containing zero to four search-engine queries. Return [] only when an external lookup is not useful. Otherwise, each query must contain a specific named entity or key phrase from the latest question; include dates or versions when relevant. Do not answer the question."}),
        json!({"role":"user","content":format!("Conversation history (most recent message last):\n{}", lines.join("\n"))}),
    ];
    let raw = call_provider(http, kind, base, key, model, &messages, Some(0.0), None).await?;
    let candidate = raw.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    let parsed: Value = serde_json::from_str(candidate).or_else(|_| {
        let start = candidate.find('[').ok_or(())?;
        let end = candidate.rfind(']').ok_or(())?;
        serde_json::from_str(&candidate[start..=end]).map_err(|_| ())
    }).map_err(|_| ApiError::bad("The provider returned invalid search queries."))?;
    let queries = parsed.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::trim).filter(|query| is_valid_search_query(query)).take(4).map(ToOwned::to_owned).collect::<Vec<_>>();
    Ok(queries)
}

fn is_valid_search_query(query: &str) -> bool {
    let normalized = query.trim();
    normalized.chars().count() >= 2 && normalized.chars().count() <= 180
}

fn should_search_web(question: &str) -> bool {
    let normalized = question.trim().to_lowercase();
    !normalized.is_empty()
        && !matches!(normalized.as_str(), "use web search" | "search the web" | "online search" | "使用网络搜索" | "使用网络")
        && !normalized.starts_with("answer in ")
        && !normalized.starts_with("reply in ")
        && !normalized.starts_with("respond in ")
}

fn content_requests_web_search(question: &str) -> bool {
    let normalized = question.to_lowercase();
    ["web search", "online search", "search the internet", "联网搜索", "网络搜索", "使用网络搜索", "上网搜索", "可以上网", "帮我搜索", "搜索一下"]
        .iter()
        .any(|phrase| normalized.contains(phrase))
}

async fn fetch_search(state: &AppState, query: &str) -> Result<Vec<Value>, ApiError> {
    let base = state
        .searxng_url
        .as_deref()
        .ok_or_else(|| ApiError::bad("Online search is not configured."))?;
    let search_query = query.trim();
    let language = if search_query.chars().any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch)) { "zh-CN" } else { "en-US" };
    let response: Value = state
        .http
        .get(format!("{}/search", base.trim_end_matches('/')))
        .query(&[("q", search_query), ("format", "json"), ("language", language)])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let mut results = response.get("results").and_then(Value::as_array).cloned().unwrap_or_default().into_iter()
        .filter(|v| v.get("title").and_then(Value::as_str).is_some_and(|x| !x.trim().is_empty()) && v.get("url").and_then(Value::as_str).is_some_and(|x| x.starts_with("http")))
        .map(|v| json!({"title":v["title"],"url":v["url"],"content":v["content"],"engine":v["engine"]}))
        .collect::<Vec<_>>();
    results.truncate(8);
    info!(query_length=query.chars().count(), result_count=results.len(), "search query completed");
    Ok(results)
}
async fn sync(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SyncQuery>,
) -> Result<Json<Page<SyncEvent>>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let rows=sqlx::query_as::<_,SyncEvent>("SELECT cursor,entity_type,entity_id,operation,revision,occurred_at FROM sync_events WHERE user_id=$1 AND cursor > $2 ORDER BY cursor LIMIT $3").bind(user_id).bind(query.cursor.unwrap_or(0)).bind(limit+1).fetch_all(&state.db).await?;
    let next = rows.get(limit as usize).map(|e| e.cursor.to_string());
    Ok(Json(Page {
        items: rows.into_iter().take(limit as usize).collect(),
        next_cursor: next,
    }))
}
async fn call_provider(
    http: &Client,
    kind: &str,
    base: &str,
    key: &str,
    model: &str,
    messages: &[Value],
    temperature: Option<f32>,
    reasoning_effort: Option<&str>,
) -> Result<String, ApiError> {
    let url = match kind {
        "anthropic" => format!("{}/v1/messages", base.trim_end_matches('/')),
        _ => format!("{}/v1/chat/completions", base.trim_end_matches('/')),
    };
    let response = if kind == "anthropic" {
        // Anthropic uses a top-level system field. Dropping it made the search planner
        // and the final search-grounding instructions ineffective for Claude providers.
        let system = messages.iter().filter(|m| m["role"] == "system").filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n\n");
        let mut body = json!({"model":model,"max_tokens":4096,"messages":messages.iter().filter(|m|m["role"] != "system").collect::<Vec<_>>()});
        if !system.is_empty() { body["system"] = json!(system); }
        if let Some(value) = temperature { body["temperature"] = json!(value.clamp(0.0, 2.0)); }
        http.post(url).header("x-api-key",key).header("anthropic-version","2023-06-01").json(&body).send().await?
    } else {
        let mut body = json!({"model":model,"messages":messages,"stream":false});
        if let Some(value) = temperature { body["temperature"] = json!(value.clamp(0.0, 2.0)); }
        if let Some(value) = reasoning_effort { body["reasoning_effort"] = json!(value); }
        http.post(url).bearer_auth(key).json(&body).send().await?
    };
    let body: Value = response.error_for_status()?.json().await?;
    let answer = if kind == "anthropic" {
        body["content"][0]["text"].as_str()
    } else {
        body["choices"][0]["message"]["content"].as_str()
    }
    .ok_or_else(|| ApiError {
        status: StatusCode::BAD_GATEWAY,
        code: "invalid_provider_response",
        message: "The AI provider returned an unexpected response.".into(),
    })?;
    Ok(answer.to_string())
}

async fn call_provider_stream(http: &Client, kind: &str, base: &str, key: &str, model: &str, messages: &[Value], temperature: Option<f32>, reasoning_effort: Option<&str>) -> Result<reqwest::Response, ApiError> {
    let url = match kind { "anthropic" => format!("{}/v1/messages", base.trim_end_matches('/')), _ => format!("{}/v1/chat/completions", base.trim_end_matches('/')) };
    let response = if kind == "anthropic" {
        let system = messages.iter().filter(|m| m["role"] == "system").filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n\n");
        let mut body = json!({"model":model,"max_tokens":4096,"stream":true,"messages":messages.iter().filter(|m|m["role"] != "system").collect::<Vec<_>>()});
        if !system.is_empty() { body["system"] = json!(system); }
        if let Some(value) = temperature { body["temperature"] = json!(value.clamp(0.0, 2.0)); }
        http.post(url).header("x-api-key", key).header("anthropic-version", "2023-06-01").json(&body).send().await?
    } else {
        let mut body = json!({"model":model,"messages":messages,"stream":true});
        if let Some(value) = temperature { body["temperature"] = json!(value.clamp(0.0, 2.0)); }
        if let Some(value) = reasoning_effort { body["reasoning_effort"] = json!(value); }
        http.post(url).bearer_auth(key).json(&body).send().await?
    };
    response.error_for_status().map_err(ApiError::from)
}

fn provider_stream_delta(kind: &str, frame: &str) -> Option<String> {
    let payload = frame.lines().find_map(|line| line.strip_prefix("data:"))?.trim();
    if payload == "[DONE]" { return None; }
    let value: Value = serde_json::from_str(payload).ok()?;
    if kind == "anthropic" { value["delta"]["text"].as_str().map(str::to_string) } else { value["choices"].as_array()?.first()?["delta"]["content"].as_str().map(str::to_string) }
}

fn split_thinking(input: &str) -> (String, String) {
    let mut answer = String::new();
    let mut reasoning = String::new();
    let mut remaining = input;
    let mut in_think = false;
    while !remaining.is_empty() {
        let marker = if in_think { "</think>" } else { "<think>" };
        match remaining.find(marker) {
            Some(index) => {
                if in_think { reasoning.push_str(&remaining[..index]); } else { answer.push_str(&remaining[..index]); }
                remaining = &remaining[index + marker.len()..];
                in_think = !in_think;
            }
            None => {
                if in_think { reasoning.push_str(remaining); } else { answer.push_str(remaining); }
                break;
            }
        }
    }
    (answer, reasoning)
}

fn strip_thinking(input: &str) -> String { split_thinking(input).0 }

struct ThinkingStream {
    in_think: bool,
    pending: String,
}
impl ThinkingStream {
    fn new() -> Self { Self { in_think: false, pending: String::new() } }
    fn push(&mut self, input: &str) -> Vec<(bool, String)> {
        self.pending.push_str(input);
        let mut output = Vec::new();
        loop {
            let marker = if self.in_think { "</think>" } else { "<think>" };
            if let Some(index) = self.pending.find(marker) {
                if index > 0 { output.push((self.in_think, self.pending[..index].to_string())); }
                self.pending.drain(..index + marker.len());
                self.in_think = !self.in_think;
                continue;
            }
            let keep = (1..marker.len()).rev().find(|size| self.pending.ends_with(&marker[..*size])).unwrap_or(0);
            let safe = self.pending.len().saturating_sub(keep);
            if safe > 0 {
                output.push((self.in_think, self.pending[..safe].to_string()));
                self.pending.drain(..safe);
            }
            return output;
        }
    }
    fn finish(&mut self) -> Vec<(bool, String)> {
        if self.pending.is_empty() { vec![] } else { vec![(self.in_think, std::mem::take(&mut self.pending))] }
    }
}

fn stream_response(state: AppState, upstream: reqwest::Response, conversation_id: Uuid, user_id: Uuid, model: String, sources: Vec<Value>, enable_markdown: bool) -> Response {
    let kind = if upstream.url().path().ends_with("/v1/messages") { "anthropic".to_string() } else { "openai_compatible".to_string() };
    let output = async_stream::stream! {
        let mut answer = String::new(); let mut reasoning = String::new(); let mut buffer = String::new(); let mut upstream = upstream.bytes_stream(); let mut thinking = ThinkingStream::new();
        while let Some(chunk) = upstream.next().await {
            match chunk {
                Ok(chunk) => { buffer.push_str(&String::from_utf8_lossy(&chunk)); buffer = buffer.replace("\r\n", "\n"); while let Some(boundary) = buffer.find("\n\n") { let frame = buffer[..boundary].to_string(); buffer.drain(..boundary + 2); if let Some(delta) = provider_stream_delta(&kind, &frame) { for (is_reasoning, text) in thinking.push(&delta) { if text.is_empty() { continue; } if is_reasoning { reasoning.push_str(&text); } else { answer.push_str(&text); } let payload = serde_json::to_string(&json!({"type":if is_reasoning { "reasoning" } else { "delta" },"delta":text})).unwrap_or_default(); yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(format!("data: {payload}\n\n"))); } } } }
                Err(error) => { warn!(conversation_id=%conversation_id, %error, "upstream stream interrupted"); let payload = serde_json::to_string(&json!({"type":"error","message":"The provider stream was interrupted."})).unwrap_or_default(); yield Ok(Bytes::from(format!("data: {payload}\n\n"))); return; }
            }
        }
        for (is_reasoning, text) in thinking.finish() { if is_reasoning { reasoning.push_str(&text); } else { answer.push_str(&text); } let payload = serde_json::to_string(&json!({"type":if is_reasoning { "reasoning" } else { "delta" },"delta":text})).unwrap_or_default(); yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(format!("data: {payload}\n\n"))); }
        if answer.trim().is_empty() { let payload = serde_json::to_string(&json!({"type":"error","message":"The provider returned no final answer."})).unwrap_or_default(); yield Ok(Bytes::from(format!("data: {payload}\n\n"))); return; }
        match persist_assistant_message(&state, conversation_id, user_id, &model, answer, reasoning, &sources, enable_markdown).await {
            Ok(message) => { let payload = serde_json::to_string(&json!({"type":"done","message":message})).unwrap_or_default(); yield Ok(Bytes::from(format!("data: {payload}\n\n"))); }
            Err(error) => { error!(conversation_id=%conversation_id, error=%error.message, "could not persist streamed response"); let payload = serde_json::to_string(&json!({"type":"error","message":"The streamed response could not be saved."})).unwrap_or_default(); yield Ok(Bytes::from(format!("data: {payload}\n\n"))); }
        }
    };
    let mut response = Body::from_stream(output).into_response();
    response.headers_mut().insert(http::header::CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    response.headers_mut().insert(http::header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

#[cfg(test)]
mod tests {
    use super::{is_valid_search_query, provider_stream_delta, split_thinking, strip_thinking, ThinkingStream};

    #[test]
    fn accepts_generic_planner_queries_without_topic_rules() {
        assert!(is_valid_search_query("ab"));
        assert!(is_valid_search_query("a specific search query"));
        assert!(!is_valid_search_query("x"));
    }

    #[test]
    fn parses_openai_stream_delta() {
        assert_eq!(provider_stream_delta("openai_compatible", "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}"), Some("hello".into()));
    }

    #[test]
    fn separates_thinking_from_visible_answer_in_complete_and_streamed_text() {
        assert_eq!(strip_thinking("Before<think>private</think>After"), "BeforeAfter");
        assert_eq!(split_thinking("Before<think>private</think>After"), ("BeforeAfter".into(), "private".into()));
        let mut stream = ThinkingStream::new();
        assert_eq!(stream.push("Before<thi"), vec![(false, "Before".into())]);
        assert_eq!(stream.push("nk>private</think>After"), vec![(true, "private".into()), (false, "After".into())]);
    }
}
