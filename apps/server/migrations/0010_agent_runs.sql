CREATE TABLE agent_runs (
    id UUID PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_message_id UUID NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('running', 'failed', 'completed', 'cancelled')),
    transcript JSONB NOT NULL DEFAULT '[]'::jsonb,
    sources JSONB NOT NULL DEFAULT '[]'::jsonb,
    events JSONB NOT NULL DEFAULT '[]'::jsonb,
    round INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX agent_runs_active_message_idx ON agent_runs(user_message_id) WHERE status IN ('running', 'failed');
CREATE INDEX agent_runs_conversation_idx ON agent_runs(conversation_id, updated_at DESC);
CREATE TRIGGER agent_runs_touch BEFORE UPDATE ON agent_runs FOR EACH ROW EXECUTE FUNCTION touch_updated_at();
