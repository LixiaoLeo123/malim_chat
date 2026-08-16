CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS citext;

CREATE TABLE users (
    id UUID PRIMARY KEY,
    email CITEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    display_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    disabled_at TIMESTAMPTZ
);

CREATE TABLE refresh_tokens (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT UNIQUE NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ,
    user_agent TEXT
);
CREATE INDEX refresh_tokens_active_idx ON refresh_tokens(user_id, expires_at) WHERE revoked_at IS NULL;

CREATE TABLE providers (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('openai_compatible', 'anthropic')),
    base_url TEXT NOT NULL,
    encrypted_api_key BYTEA NOT NULL,
    key_nonce BYTEA NOT NULL,
    default_model TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, name)
);
CREATE INDEX providers_user_idx ON providers(user_id);

CREATE TABLE conversations (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL DEFAULT 'New chat',
    model_provider_id UUID REFERENCES providers(id) ON DELETE SET NULL,
    model TEXT,
    context_window INTEGER NOT NULL DEFAULT 128000 CHECK (context_window > 0),
    context_tokens INTEGER NOT NULL DEFAULT 0 CHECK (context_tokens >= 0),
    next_sequence BIGINT NOT NULL DEFAULT 1,
    revision BIGINT NOT NULL DEFAULT 1,
    archived_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX conversations_user_updated_idx ON conversations(user_id, updated_at DESC, id DESC) WHERE archived_at IS NULL;

CREATE TABLE messages (
    id UUID PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    sequence BIGINT NOT NULL,
    client_mutation_id UUID,
    role TEXT NOT NULL CHECK (role IN ('system', 'user', 'assistant', 'tool', 'summary')),
    content TEXT NOT NULL,
    content_format TEXT NOT NULL DEFAULT 'markdown',
    status TEXT NOT NULL DEFAULT 'complete' CHECK (status IN ('pending', 'streaming', 'complete', 'error')),
    model TEXT,
    token_count INTEGER NOT NULL DEFAULT 0 CHECK (token_count >= 0),
    search_sources JSONB NOT NULL DEFAULT '[]'::jsonb,
    edited_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (conversation_id, sequence),
    UNIQUE (conversation_id, client_mutation_id)
);
CREATE INDEX messages_timeline_idx ON messages(conversation_id, sequence DESC) WHERE deleted_at IS NULL;

CREATE TABLE conversation_summaries (
    id UUID PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    starts_at_sequence BIGINT NOT NULL,
    ends_at_sequence BIGINT NOT NULL,
    content TEXT NOT NULL,
    token_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (conversation_id, starts_at_sequence, ends_at_sequence)
);

CREATE TABLE sync_events (
    cursor BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    entity_type TEXT NOT NULL,
    entity_id UUID NOT NULL,
    operation TEXT NOT NULL CHECK (operation IN ('created', 'updated', 'deleted')),
    revision BIGINT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX sync_events_user_cursor_idx ON sync_events(user_id, cursor);

CREATE OR REPLACE FUNCTION touch_updated_at() RETURNS trigger AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER users_touch BEFORE UPDATE ON users FOR EACH ROW EXECUTE FUNCTION touch_updated_at();
CREATE TRIGGER providers_touch BEFORE UPDATE ON providers FOR EACH ROW EXECUTE FUNCTION touch_updated_at();
CREATE TRIGGER conversations_touch BEFORE UPDATE ON conversations FOR EACH ROW EXECUTE FUNCTION touch_updated_at();
CREATE TRIGGER messages_touch BEFORE UPDATE ON messages FOR EACH ROW EXECUTE FUNCTION touch_updated_at();
