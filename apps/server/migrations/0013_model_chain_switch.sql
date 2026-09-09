-- Whether a Responses model is driven with `previous_response_id` instead of resending
-- the transcript. Off means the model always gets the full conversation every turn.
ALTER TABLE provider_models
    ADD COLUMN chain_context BOOLEAN NOT NULL DEFAULT TRUE;
