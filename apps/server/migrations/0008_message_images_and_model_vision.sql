ALTER TABLE messages
    ADD COLUMN images JSONB NOT NULL DEFAULT '[]'::jsonb;

ALTER TABLE provider_models
    ADD COLUMN supports_images BOOLEAN NOT NULL DEFAULT FALSE;
