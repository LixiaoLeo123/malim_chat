ALTER TABLE providers DROP CONSTRAINT IF EXISTS providers_kind_check;
ALTER TABLE providers ADD CONSTRAINT providers_kind_check CHECK (kind IN ('openai_compatible', 'openai_responses', 'anthropic'));
ALTER TABLE provider_models DROP CONSTRAINT IF EXISTS provider_models_kind_check;
ALTER TABLE provider_models ADD CONSTRAINT provider_models_kind_check CHECK (kind IN ('openai_compatible', 'openai_responses', 'anthropic'));
CREATE UNIQUE INDEX agent_runs_active_conversation_idx ON agent_runs(conversation_id) WHERE status = 'running';
