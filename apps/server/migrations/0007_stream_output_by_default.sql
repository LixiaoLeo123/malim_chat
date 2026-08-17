ALTER TABLE conversations
    ALTER COLUMN generation_settings SET DEFAULT '{"temperature":0.7,"reasoning_effort":"medium","enable_markdown":true,"stream":true}'::jsonb;
