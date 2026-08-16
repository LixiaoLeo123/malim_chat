ALTER TABLE provider_models ADD COLUMN kind TEXT NOT NULL DEFAULT 'openai_compatible' CHECK (kind IN ('openai_compatible', 'anthropic'));
UPDATE provider_models SET kind = providers.kind FROM providers WHERE providers.id = provider_models.provider_id;
