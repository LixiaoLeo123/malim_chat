-- The provider-format columns are constrained to a closed set; widening the set is a
-- migration, not just a validation change, or saving a Google model fails at the database.
ALTER TABLE providers DROP CONSTRAINT IF EXISTS providers_kind_check;
ALTER TABLE providers ADD CONSTRAINT providers_kind_check CHECK (kind IN ('openai_compatible', 'openai_responses', 'anthropic', 'google_gemini'));

ALTER TABLE provider_models DROP CONSTRAINT IF EXISTS provider_models_kind_check;
ALTER TABLE provider_models ADD CONSTRAINT provider_models_kind_check CHECK (kind IN ('openai_compatible', 'openai_responses', 'anthropic', 'google_gemini'));