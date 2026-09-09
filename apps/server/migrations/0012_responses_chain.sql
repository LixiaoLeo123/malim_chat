-- Responses context chaining: the provider stores the conversation, so the server
-- only remembers where the stored chain ends instead of resending the transcript.
ALTER TABLE conversations
    ADD COLUMN chain_response_id TEXT,
    ADD COLUMN chain_sequence BIGINT NOT NULL DEFAULT 0;
