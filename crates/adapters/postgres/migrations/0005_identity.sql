-- Issue #38 (FR-16): the identity tables behind sign-in, API keys and the OIDC login. Directory tables in the sense of
-- ADR-0041 (`scope: directory`): a user belongs to the installation, not to a workspace, so none of them has a
-- `workspace_id`. Idempotent like the files before it (`migrate` applies every file on every run).
--
-- Nothing here stores a secret in a form that can be used: a password is an argon2id PHC string, an API key and a
-- pending login's `state` are SHA-256 digests (like `sessions.token_hash`). The only readable values are the ones a
-- provider round trip needs back: the nonce and the PKCE verifier, which live for minutes and are deleted when taken.
--
-- Kept portable on purpose (issue #37 carried it to SQLite, on a branch archived at the tag `spike/0002-worker-do`): the column types are the ones 0001-0004
-- already use (`uuid` keys, `bytea` digests, `timestamptz`, `boolean`), and there is no partial index, no `jsonb` and
-- no `DO` block here.

-- scope: directory
CREATE TABLE IF NOT EXISTS users (
    id             uuid PRIMARY KEY,
    -- trimmed and lower-cased by the core; NULL for a user created by id only. UNIQUE ignores NULLs.
    email          text UNIQUE,
    -- a PHC string ('$argon2id$v=19$m=...,t=...,p=...$salt$hash'); NULL: no password, the user signs in through a provider
    password_hash  text,
    -- a deactivated user's sessions and keys stop working at once (ADR-0052 Tests); deactivation proper is ADR-0053's
    is_active      boolean NOT NULL DEFAULT true,
    created_at     timestamptz NOT NULL DEFAULT now()
);

-- scope: directory. Plane's `Account(provider, provider_account_id)` shape (ADR-0050): `provider` is the issuer URL and
-- `provider_account_id` the `sub` claim, so two providers cannot collide.
CREATE TABLE IF NOT EXISTS accounts (
    provider             text NOT NULL,
    provider_account_id  text NOT NULL,
    user_id              uuid NOT NULL REFERENCES users (id),
    email                text,
    created_at           timestamptz NOT NULL,
    PRIMARY KEY (provider, provider_account_id)
);

CREATE INDEX IF NOT EXISTS accounts_user ON accounts (user_id);

-- scope: directory. ADR-0052 K1: a high-entropy token with a visible prefix; only its digest is stored, so there is no
-- plaintext column and a key cannot be shown twice. The digest is the lookup key (UNIQUE gives the index).
CREATE TABLE IF NOT EXISTS api_keys (
    id            uuid PRIMARY KEY,
    user_id       uuid NOT NULL REFERENCES users (id),
    token_hash    bytea NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    prefix        text NOT NULL,
    label         text NOT NULL,
    created_at    timestamptz NOT NULL,
    expires_at    timestamptz,
    revoked_at    timestamptz,
    last_used_at  timestamptz
);

CREATE INDEX IF NOT EXISTS api_keys_user ON api_keys (user_id, created_at);

-- scope: directory. A login in flight: written when the browser is sent to the provider, deleted by the callback that
-- takes it, so a replayed callback finds nothing. Rows that nobody came back for are dropped by the next insert.
CREATE TABLE IF NOT EXISTS oidc_logins (
    state_hash     bytea PRIMARY KEY CHECK (octet_length(state_hash) = 32),
    nonce          text NOT NULL,
    code_verifier  text NOT NULL,
    created_at     timestamptz NOT NULL,
    expires_at     timestamptz NOT NULL
);

CREATE INDEX IF NOT EXISTS oidc_logins_expiry ON oidc_logins (expires_at);

-- Sessions already carry the user id (0004). They are not given a foreign key: the lookup joins `users` and requires
-- an active row, which fails closed for a session whose user does not exist, and adding a constraint to a table that
-- may hold rows from a stage-2 run would need a backfill this spike does not have a reason to write.
