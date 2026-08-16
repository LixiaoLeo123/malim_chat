ALTER TABLE conversations
    ADD COLUMN generation_settings JSONB NOT NULL DEFAULT '{"temperature":0.7,"reasoning_effort":"medium","enable_markdown":true,"stream":false}'::jsonb;
