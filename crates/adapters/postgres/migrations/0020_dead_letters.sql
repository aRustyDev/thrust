-- Dead letters (ADR-0070, FR-21, issue #53): a queue message the consumer set aside instead of processing, because it can never be
-- processed (an event with no `project_id`; a body that is not an envelope) or because it kept failing until it was delivered as often
-- as the policy allows (`MAX_DELIVERIES`). Every queue adapter writes here through the consumer, so one table holds them all and
-- `GET /internal/dead-letters` reads one place on every target.
--
-- No tenant: a body that does not decode names none, and a dead letter is an operator's concern and not a workspace's. For the same
-- reason there is no foreign key (the event's work item or workspace may be what was missing), and no policy of who may read: the
-- consumer writes it and the test route reads it.
--
-- `id` is the queue's own id for the message when it has one, so a message that is delivered again after it was set aside lands on the
-- same row (`ON CONFLICT DO NOTHING`); otherwise an id the consumer made. `attempts` is how many times the message was delivered, and
-- NULL for a message that Cloudflare's own dead-letter queue moved here after its retries: that queue starts the count again, so the
-- number is not known (research 0004, F4).
--
-- Idempotent like the files before it (`migrate` applies every file on every run).

CREATE TABLE IF NOT EXISTS dead_letters (
    id           text PRIMARY KEY,
    received_at  timestamptz NOT NULL,
    topic        text NOT NULL,
    -- the event's id when the body decoded as an envelope
    event_id     uuid,
    -- the message as it arrived: the envelope's JSON, or the raw text of a body that is not one
    body         text NOT NULL,
    error        text NOT NULL,
    attempts     integer CHECK (attempts IS NULL OR attempts >= 1)
);

-- the listing is newest first
CREATE INDEX IF NOT EXISTS dead_letters_received_at ON dead_letters (received_at DESC);
