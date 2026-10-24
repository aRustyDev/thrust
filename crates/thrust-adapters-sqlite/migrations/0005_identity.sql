-- The SQLite translation of Postgres migration 0005 (identity, issue #38, crates/thrust-adapters-postgres/migrations): the
-- tables behind sign-in, API keys and the OIDC login. Directory tables in the sense of ADR-0041 (`scope: directory`): a user
-- belongs to the installation, not to a workspace, so none of them has a `workspace_id`. Same names, same columns, same
-- constraints; the dialect is the one 0001 describes (uuid -> TEXT, timestamptz -> INTEGER microseconds, bytea -> BLOB,
-- boolean -> INTEGER 0/1). The Postgres file is idempotent by `IF NOT EXISTS`; this one runs once, as one transaction,
-- with the row that records it (`migrate`).
--
-- Nothing here stores a secret in a form that can be used: a password is an argon2id PHC string, an API key and a pending
-- login's `state` are SHA-256 digests (like `sessions.token_hash`). The only readable values are the ones a provider round
-- trip needs back: the nonce and the PKCE verifier, which live for minutes and are deleted when taken.

-- scope: directory
CREATE TABLE users (
    id             TEXT PRIMARY KEY,
    -- trimmed and lower-cased by the core; NULL for a user created by id only. UNIQUE ignores NULLs.
    email          TEXT UNIQUE,
    -- a PHC string ('$argon2id$v=19$m=...,t=...,p=...$salt$hash'); NULL: no password, the user signs in through a provider
    password_hash  TEXT,
    -- a deactivated user's sessions and keys stop working at once (ADR-0052 Tests); deactivation proper is ADR-0053's
    is_active      INTEGER NOT NULL DEFAULT 1,
    -- `now()` in Postgres; the same `strftime` the migration ledger uses here, in microseconds
    created_at     INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000000)
);

-- scope: directory. Plane's `Account(provider, provider_account_id)` shape (ADR-0050): `provider` is the issuer URL and
-- `provider_account_id` the `sub` claim, so two providers cannot collide.
CREATE TABLE accounts (
    provider             TEXT NOT NULL,
    provider_account_id  TEXT NOT NULL,
    user_id              TEXT NOT NULL REFERENCES users (id),
    email                TEXT,
    created_at           INTEGER NOT NULL,
    PRIMARY KEY (provider, provider_account_id)
);

CREATE INDEX accounts_user ON accounts (user_id);

-- scope: directory. ADR-0052 K1: a high-entropy token with a visible prefix; only its digest is stored, so there is no
-- plaintext column and a key cannot be shown twice. The digest is the lookup key (UNIQUE gives the index).
CREATE TABLE api_keys (
    id            TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL REFERENCES users (id),
    token_hash    BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    prefix        TEXT NOT NULL,
    label         TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    expires_at    INTEGER,
    revoked_at    INTEGER,
    last_used_at  INTEGER
);

CREATE INDEX api_keys_user ON api_keys (user_id, created_at);

-- scope: directory. A login in flight: written when the browser is sent to the provider, deleted by the callback that
-- takes it, so a replayed callback finds nothing. Rows that nobody came back for are dropped by the next insert.
CREATE TABLE oidc_logins (
    state_hash     BLOB PRIMARY KEY CHECK (length(state_hash) = 32),
    nonce          TEXT NOT NULL,
    code_verifier  TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    expires_at     INTEGER NOT NULL
);

CREATE INDEX oidc_logins_expiry ON oidc_logins (expires_at);

-- Sessions already carry the user id (0001). They are not given a foreign key, as in Postgres: the lookup joins `users` and
-- requires an active row, which fails closed for a session whose user does not exist.
