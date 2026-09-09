use super::*;

/// Accounts, session tokens and the AES-GCM envelope used for provider credentials.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Claims {
    sub: Uuid,
    exp: usize,
    iat: usize,
    typ: String,
}

#[derive(Serialize)]
pub(crate) struct AuthResponse {
    access_token: String,
    refresh_token: String,
    user: User,
}

#[derive(Deserialize)]
pub(crate) struct SignupRequest {
    email: String,
    password: String,
    display_name: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Deserialize)]
pub(crate) struct RefreshRequest {
    refresh_token: String,
}

#[derive(Debug, Serialize, FromRow)]
pub(crate) struct User {
    id: Uuid,
    email: String,
    display_name: String,
    created_at: DateTime<Utc>,
}

#[derive(FromRow)]
pub(crate) struct AuthUser {
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
pub(crate) struct RefreshUser {
    id: Uuid,
    email: String,
    display_name: String,
    created_at: DateTime<Utc>,
    refresh_id: Uuid,
}

pub(crate) fn token_for(state: &AppState, user_id: Uuid) -> Result<String, ApiError> {
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

pub(crate) fn user_from_headers(state: &AppState, headers: &HeaderMap) -> Result<Uuid, ApiError> {
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

pub(crate) fn digest(input: &str) -> String {
    format!("{:x}", Sha256::digest(input.as_bytes()))
}

pub(crate) fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    BASE64.encode(bytes)
}

pub(crate) fn encrypt(state: &AppState, plaintext: &str) -> Result<(Vec<u8>, Vec<u8>), ApiError> {
    let cipher = Aes256Gcm::new_from_slice(state.encryption_key.as_ref())
        .map_err(|_| ApiError::internal("encryption setup failed"))?;
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let encrypted = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|_| ApiError::internal("credential encryption failed"))?;
    Ok((encrypted, nonce.to_vec()))
}

pub(crate) fn decrypt(state: &AppState, encrypted: &[u8], nonce: &[u8]) -> Result<String, ApiError> {
    let cipher = Aes256Gcm::new_from_slice(state.encryption_key.as_ref())
        .map_err(|_| ApiError::internal("encryption setup failed"))?;
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), encrypted)
        .map_err(|_| ApiError::internal("stored credential could not be decrypted"))?;
    String::from_utf8(plain).map_err(|_| ApiError::internal("stored credential is invalid"))
}

pub(crate) async fn signup(
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

pub(crate) async fn login(
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

pub(crate) async fn refresh(
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

pub(crate) async fn issue_session(state: &AppState, user: User) -> Result<AuthResponse, ApiError> {
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
