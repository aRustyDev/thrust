-- Issue #44 (the compat slice): what a person says about themselves, and how they have set the app up. One row per user,
-- a directory table like `users` and `sessions` (ADR-0041): keyed by the user, no tenant key. `last_workspace_id` is a
-- value the app writes and reads back, not a tenant key, and has no foreign key (Plane has none either).
--
-- The typed columns are the ones core or the router acts on. Everything else the settings pages write (theme, language,
-- role, use case, the week's first day, ...) is `preferences`, a JSON object, so one more setting is not one more
-- migration; the compat layer fills a missing key with Plane's default.
--
-- Idempotent like the files before it (`migrate` applies every file on every run).

CREATE TABLE IF NOT EXISTS user_profiles (
    user_id            uuid PRIMARY KEY REFERENCES users (id),
    -- the profile row's own id (Plane's `Profile.id`); not the user's
    id                 uuid NOT NULL UNIQUE,
    -- 32 hex digits made at sign-up; never shown as a name
    username           text NOT NULL UNIQUE,
    first_name         text NOT NULL DEFAULT '',
    last_name          text NOT NULL DEFAULT '',
    display_name       text NOT NULL DEFAULT '',
    avatar             text NOT NULL DEFAULT '',
    cover_image        text,
    timezone           text NOT NULL DEFAULT 'UTC',
    date_joined        timestamptz NOT NULL,
    last_login_at      timestamptz,
    is_onboarded       boolean NOT NULL DEFAULT false,
    is_tour_completed  boolean NOT NULL DEFAULT false,
    last_workspace_id  uuid,
    onboarding_step    jsonb NOT NULL DEFAULT '{}'::jsonb,
    preferences        jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at         timestamptz NOT NULL,
    updated_at         timestamptz NOT NULL,
    CONSTRAINT user_profiles_onboarding_step_is_object CHECK (jsonb_typeof(onboarding_step) = 'object'),
    CONSTRAINT user_profiles_preferences_is_object CHECK (jsonb_typeof(preferences) = 'object')
);
