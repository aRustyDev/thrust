-- Email sign-in (F-A04, D-042): the code that is mailed to a person who asks to sign in with a code instead of a password. A code is six
-- digits, shown once in the mail and stored only as the digest of its text (a copy of this table is not the codes themselves, but six
-- digits are guessable by anyone who can read it: the protection of an account is the short life of a code, the count of wrong tries it
-- allows and the count of codes a window allows, all kept here).
--
-- One row a user, so a user has at most one code that works: a new code replaces the old in the statement that stores it. The row also
-- holds the count of codes made in the window that the first of them opened (`window_started_at`, `issued`): it outlives a code that was
-- exhausted, so asking for a new one is still capped, and it goes with the code that is used (a person who signed in may ask again).
--
-- No foreign key to `users`: like `sessions` (0004), `oidc_logins` (0005) and `password_resets` (0017) this is a directory table that is
-- looked up before anyone is known, and a row whose user is gone is never matched (the sign-in looks the user up first) and is dropped
-- by the next insert once its window and its code are over.
--
-- Idempotent like the files before it (`migrate` applies every file on every run).

CREATE TABLE IF NOT EXISTS login_codes (
    user_id            uuid PRIMARY KEY,
    -- the current code's digest; NULL once it was exhausted
    code_hash          bytea CHECK (code_hash IS NULL OR octet_length(code_hash) = 32),
    -- when the current code's time is over
    code_expires_at    timestamptz NOT NULL,
    -- tries at the current code, wrong ones and the right one
    attempts           integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    -- when the window's first code was made, and how many codes the window has had
    window_started_at  timestamptz NOT NULL,
    issued             integer NOT NULL CHECK (issued >= 1)
);

-- the clean-up that every insert does looks for windows that are over
CREATE INDEX IF NOT EXISTS login_codes_window ON login_codes (window_started_at);
