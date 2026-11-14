-- Email recovery (F-A03, D-041): the tokens of the password-reset emails. A token is a random 256-bit value that is mailed once and
-- shown nowhere else; only its SHA-256 digest is stored, so a copy of this table is not a way into any account (as `sessions` and
-- `oidc_logins` are stored). A token is single use: the reset takes the row, so a second post of the same link finds nothing.
-- A user has at most one that works: a new one replaces the older ones in the statement that inserts it (a request while the last
-- one is younger than the cooldown stores nothing, and sends nothing, in the same statement).
--
-- No foreign key to `users`: like `sessions` (0004) and `oidc_logins` (0005) this is a directory table that a token is looked
-- up in before anyone is known, and a row whose user is gone is refused by the reset (it looks the user up) and expires by itself.
-- Rows that nobody came back for are dropped by the next insert.
--
-- Idempotent like the files before it (`migrate` applies every file on every run).

CREATE TABLE IF NOT EXISTS password_resets (
    token_hash  bytea PRIMARY KEY CHECK (octet_length(token_hash) = 32),
    user_id     uuid NOT NULL,
    created_at  timestamptz NOT NULL,
    expires_at  timestamptz NOT NULL
);

-- the cooldown asks for the newest token of a user, and an insert replaces that user's older ones
CREATE INDEX IF NOT EXISTS password_resets_user ON password_resets (user_id, created_at);

CREATE INDEX IF NOT EXISTS password_resets_expiry ON password_resets (expires_at);
