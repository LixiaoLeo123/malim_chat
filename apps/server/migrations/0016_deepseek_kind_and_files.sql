-- DeepSeek is its own dialect: its hosted web tools and file references are extensions
-- of Chat Completions that no other OpenAI-compatible gateway speaks. Widening the kind
-- set is a migration because the columns are constrained, not just validated in Rust.
ALTER TABLE providers DROP CONSTRAINT IF EXISTS providers_kind_check;
ALTER TABLE providers ADD CONSTRAINT providers_kind_check CHECK (kind IN ('openai_compatible', 'openai_responses', 'anthropic', 'google_gemini', 'deepseek'));

ALTER TABLE provider_models DROP CONSTRAINT IF EXISTS provider_models_kind_check;
ALTER TABLE provider_models ADD CONSTRAINT provider_models_kind_check CHECK (kind IN ('openai_compatible', 'openai_responses', 'anthropic', 'google_gemini', 'deepseek'));

-- Documents uploaded through the provider's Files API are referenced by id, not inlined,
-- so a message keeps the list of references it was sent with. Images keep using `images`.
ALTER TABLE messages ADD COLUMN files JSONB NOT NULL DEFAULT '[]'::jsonb;