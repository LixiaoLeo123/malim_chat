-- Whether a Responses model is sent OpenAI's hosted tools (web search, code interpreter).
-- A gateway whose channels do not implement them fails the whole turn, so this has to be
-- switchable per model rather than assumed for the dialect.
ALTER TABLE provider_models
    ADD COLUMN hosted_tools BOOLEAN NOT NULL DEFAULT TRUE;
