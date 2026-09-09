use super::*;

/// Provider and per-model configuration: encrypted credentials, base URLs, the API
/// dialect a model speaks, its context window, vision and context-chaining switches.

pub(crate) async fn list_providers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Provider>>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let rows: Vec<ProviderRow> = sqlx::query_as("SELECT id,name,kind,base_url,default_model,created_at,updated_at FROM providers WHERE user_id=$1 ORDER BY name")
        .bind(user_id).fetch_all(&state.db).await?;
    let mut providers = Vec::with_capacity(rows.len());
    for row in rows {
        providers.push(provider_with_models(&state.db, row).await?);
    }
    Ok(Json(providers))
}

pub(crate) async fn create_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ProviderRequest>,
) -> Result<Json<Provider>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let kind = request.kind.unwrap_or_else(|| "openai_compatible".into());
    if !matches!(
        kind.as_str(),
        "openai_compatible" | "openai_responses" | "anthropic"
    ) || !request.base_url.starts_with("https://")
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
    let default_model = request
        .default_model
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let mut tx = state.db.begin().await?;
    let row:ProviderRow=sqlx::query_as("INSERT INTO providers (id,user_id,name,kind,base_url,encrypted_api_key,key_nonce,default_model) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id,name,kind,base_url,default_model,created_at,updated_at").bind(Uuid::new_v4()).bind(user_id).bind(request.name.trim()).bind(kind).bind(request.base_url.trim_end_matches('/')).bind(ciphertext).bind(nonce).bind(default_model).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    let provider = provider_with_models(&state.db, row).await?;
    event(&state.db, user_id, "provider", provider.id, "created", 1).await?;
    Ok(Json(provider))
}

pub(crate) async fn update_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateProviderRequest>,
) -> Result<Json<Provider>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    let current = own_provider(&state.db, user_id, id).await?;
    let name = request
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or(current.name);
    let kind = request.kind.unwrap_or(current.kind);
    if !matches!(
        kind.as_str(),
        "openai_compatible" | "openai_responses" | "anthropic"
    ) {
        return Err(ApiError::bad(
            "Provider API format must be OpenAI-compatible or Anthropic.",
        ));
    }
    let base_url = request
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_end_matches('/').to_string())
        .unwrap_or(current.base_url);
    if !base_url.starts_with("https://") {
        return Err(ApiError::bad("Provider base URL must use HTTPS."));
    }
    let (ciphertext, nonce): (Option<Vec<u8>>, Option<Vec<u8>>) = match request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(key) => {
            let (ciphertext, nonce) = encrypt(&state, key)?;
            (Some(ciphertext), Some(nonce))
        }
        None => (None, None),
    };
    let row: ProviderRow = if let (Some(ciphertext), Some(nonce)) = (ciphertext, nonce) {
        sqlx::query_as("UPDATE providers SET name=$3, kind=$4, base_url=$5, encrypted_api_key=$6, key_nonce=$7 WHERE id=$1 AND user_id=$2 RETURNING id,name,kind,base_url,default_model,created_at,updated_at")
            .bind(id).bind(user_id).bind(name).bind(kind).bind(base_url).bind(ciphertext).bind(nonce).fetch_one(&state.db).await?
    } else {
        sqlx::query_as("UPDATE providers SET name=$3, kind=$4, base_url=$5 WHERE id=$1 AND user_id=$2 RETURNING id,name,kind,base_url,default_model,created_at,updated_at")
            .bind(id).bind(user_id).bind(name).bind(kind).bind(base_url).fetch_one(&state.db).await?
    };
    let provider = provider_with_models(&state.db, row).await?;
    event(&state.db, user_id, "provider", provider.id, "updated", 1).await?;
    Ok(Json(provider))
}

pub(crate) async fn provider_with_models(pool: &PgPool, row: ProviderRow) -> Result<Provider, ApiError> {
    let models = sqlx::query_as("SELECT id,provider_id,group_name,model,kind,sort_order,context_window,supports_images,chain_context,hosted_tools,created_at,updated_at FROM provider_models WHERE provider_id=$1 ORDER BY group_name,sort_order,model")
        .bind(row.id).fetch_all(pool).await?;
    Ok(Provider {
        id: row.id,
        name: row.name,
        kind: row.kind,
        base_url: row.base_url,
        default_model: row.default_model,
        created_at: row.created_at,
        updated_at: row.updated_at,
        models,
    })
}

pub(crate) async fn own_provider(pool: &PgPool, user_id: Uuid, id: Uuid) -> Result<ProviderRow, ApiError> {
    sqlx::query_as("SELECT id,name,kind,base_url,default_model,created_at,updated_at FROM providers WHERE id=$1 AND user_id=$2")
        .bind(id).bind(user_id).fetch_optional(pool).await?.ok_or_else(ApiError::not_found)
}

pub(crate) async fn configured_model(pool: &PgPool, provider_id: Uuid, model: &str) -> Result<bool, ApiError> {
    Ok(sqlx::query_as::<_, (bool,)>(
        "SELECT EXISTS(SELECT 1 FROM provider_models WHERE provider_id=$1 AND model=$2)",
    )
    .bind(provider_id)
    .bind(model)
    .fetch_one(pool)
    .await?
    .0)
}

pub(crate) async fn provider_first_model(
    pool: &PgPool,
    provider_id: Uuid,
) -> Result<Option<String>, ApiError> {
    Ok(sqlx::query_as::<_, (String,)>(
        "SELECT model FROM provider_models WHERE provider_id=$1 ORDER BY sort_order,model LIMIT 1",
    )
    .bind(provider_id)
    .fetch_optional(pool)
    .await?
    .map(|value| value.0))
}

pub(crate) async fn provider_model_kind(
    pool: &PgPool,
    provider_id: Uuid,
    model: &str,
) -> Result<Option<String>, ApiError> {
    Ok(sqlx::query_as::<_, (String,)>(
        "SELECT kind FROM provider_models WHERE provider_id=$1 AND model=$2 LIMIT 1",
    )
    .bind(provider_id)
    .bind(model)
    .fetch_optional(pool)
    .await?
    .map(|value| value.0))
}

/// A per-model boolean switch. Both Responses switches live as columns because a gateway
/// may implement any subset of the dialect, and only the user knows which one they picked.
pub(crate) async fn provider_model_bool(
    pool: &PgPool,
    provider_id: Uuid,
    model: &str,
    column: &'static str,
    fallback: bool,
) -> Result<bool, ApiError> {
    Ok(
        sqlx::query_as::<_, (bool,)>(&format!(
            "SELECT {column} FROM provider_models WHERE provider_id=$1 AND model=$2 LIMIT 1"
        ))
        .bind(provider_id)
        .bind(model)
        .fetch_optional(pool)
        .await?
        .map(|value| value.0)
        .unwrap_or(fallback),
    )
}

/// Whether this model should be sent the provider's own hosted tools. Per model because a
/// gateway implements any subset of a dialect, and a rejected tool fails the whole turn.
pub(crate) async fn provider_model_hosted_tools(
    pool: &PgPool,
    provider_id: Uuid,
    model: &str,
) -> Result<bool, ApiError> {
    provider_model_bool(pool, provider_id, model, "hosted_tools", true).await
}

pub(crate) async fn provider_model_supports_images(
    pool: &PgPool,
    provider_id: Uuid,
    model: &str,
) -> Result<Option<bool>, ApiError> {
    Ok(sqlx::query_as::<_, (bool,)>(
        "SELECT supports_images FROM provider_models WHERE provider_id=$1 AND model=$2 LIMIT 1",
    )
    .bind(provider_id)
    .bind(model)
    .fetch_optional(pool)
    .await?
    .map(|value| value.0))
}

pub(crate) async fn create_provider_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(request): Json<ProviderModelRequest>,
) -> Result<Json<ProviderModel>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    own_provider(&state.db, user_id, id).await?;
    if request.group_name.trim().is_empty() || request.model.trim().is_empty() {
        return Err(ApiError::bad("Model group and model name are required."));
    }
    if !matches!(
        request.kind.as_str(),
        "openai_compatible" | "openai_responses" | "anthropic"
    ) {
        return Err(ApiError::bad(
            "Model API format must be OpenAI-compatible or Anthropic.",
        ));
    }
    let item = sqlx::query_as("INSERT INTO provider_models (id,provider_id,group_name,model,kind,sort_order,context_window,supports_images,chain_context,hosted_tools) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING id,provider_id,group_name,model,kind,sort_order,context_window,supports_images,chain_context,hosted_tools,created_at,updated_at")
        .bind(Uuid::new_v4()).bind(id).bind(request.group_name.trim()).bind(request.model.trim()).bind(request.kind).bind(request.sort_order.unwrap_or(0)).bind(request.context_window.unwrap_or(128_000).clamp(4096, 2_000_000)).bind(request.supports_images.unwrap_or(false)).bind(request.chain_context.unwrap_or(true)).bind(request.hosted_tools.unwrap_or(true)).fetch_one(&state.db).await?;
    event(&state.db, user_id, "provider", id, "updated", 1).await?;
    Ok(Json(item))
}

pub(crate) async fn update_provider_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, model_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<UpdateProviderModelRequest>,
) -> Result<Json<ProviderModel>, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    own_provider(&state.db, user_id, id).await?;
    if request
        .group_name
        .as_ref()
        .is_some_and(|v| v.trim().is_empty())
        || request.model.as_ref().is_some_and(|v| v.trim().is_empty())
    {
        return Err(ApiError::bad("Model group and model name cannot be empty."));
    }
    if request
        .kind
        .as_deref()
        .is_some_and(|v| !matches!(v, "openai_compatible" | "openai_responses" | "anthropic"))
    {
        return Err(ApiError::bad(
            "Model API format must be OpenAI-compatible or Anthropic.",
        ));
    }
    let previous: (String,) =
        sqlx::query_as("SELECT model FROM provider_models WHERE id=$1 AND provider_id=$2")
            .bind(model_id)
            .bind(id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(ApiError::not_found)?;
    let item: ProviderModel = sqlx::query_as("UPDATE provider_models SET group_name=COALESCE($3,group_name),model=COALESCE($4,model),kind=COALESCE($5,kind),sort_order=COALESCE($6,sort_order),context_window=COALESCE($7,context_window),supports_images=COALESCE($8,supports_images),chain_context=COALESCE($9,chain_context),hosted_tools=COALESCE($10,hosted_tools) WHERE id=$1 AND provider_id=$2 RETURNING id,provider_id,group_name,model,kind,sort_order,context_window,supports_images,chain_context,hosted_tools,created_at,updated_at")
        .bind(model_id).bind(id).bind(request.group_name.map(|v| v.trim().to_string())).bind(request.model.map(|v| v.trim().to_string())).bind(request.kind).bind(request.sort_order).bind(request.context_window.map(|value| value.clamp(4096, 2_000_000))).bind(request.supports_images).bind(request.chain_context).bind(request.hosted_tools).fetch_optional(&state.db).await?.ok_or_else(ApiError::not_found)?;
    sqlx::query("UPDATE providers SET default_model=$3 WHERE id=$1 AND default_model=$2")
        .bind(id)
        .bind(previous.0)
        .bind(&item.model)
        .execute(&state.db)
        .await?;
    event(&state.db, user_id, "provider", id, "updated", 1).await?;
    Ok(Json(item))
}

pub(crate) async fn delete_provider_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, model_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let user_id = user_from_headers(&state, &headers)?;
    own_provider(&state.db, user_id, id).await?;
    let removed: Option<(String,)> = sqlx::query_as(
        "DELETE FROM provider_models WHERE id=$1 AND provider_id=$2 RETURNING model",
    )
    .bind(model_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let removed = removed.ok_or_else(ApiError::not_found)?;
    sqlx::query("UPDATE providers SET default_model=(SELECT model FROM provider_models WHERE provider_id=$1 ORDER BY sort_order,model LIMIT 1) WHERE id=$1 AND default_model=$2").bind(id).bind(removed.0).execute(&state.db).await?;
    event(&state.db, user_id, "provider", id, "updated", 1).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn delete_provider(
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
