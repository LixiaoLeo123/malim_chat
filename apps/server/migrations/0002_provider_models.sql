CREATE TABLE provider_models (
    id UUID PRIMARY KEY,
    provider_id UUID NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    group_name TEXT NOT NULL DEFAULT 'General',
    model TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (provider_id, model)
);
CREATE INDEX provider_models_provider_group_idx ON provider_models(provider_id, group_name, sort_order, model);
CREATE TRIGGER provider_models_touch BEFORE UPDATE ON provider_models FOR EACH ROW EXECUTE FUNCTION touch_updated_at();

INSERT INTO provider_models (id, provider_id, group_name, model)
SELECT gen_random_uuid(), id, 'General', default_model
FROM providers
ON CONFLICT (provider_id, model) DO NOTHING;

INSERT INTO provider_models (id, provider_id, group_name, model)
SELECT gen_random_uuid(), model_provider_id, 'Imported', model
FROM conversations
WHERE model_provider_id IS NOT NULL AND model IS NOT NULL AND btrim(model) <> ''
ON CONFLICT (provider_id, model) DO NOTHING;
