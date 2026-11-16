-- Issue #44 (the compat slice, workspace-load stage): what a member's first visit to a workspace makes for them. The app
-- reads a sidebar, a home page and a set of work-item list settings, and Plane creates the member's rows on that first read;
-- so do we, with `INSERT ... ON CONFLICT DO NOTHING`, which makes two first visits at once make one row each.
--
-- Three tables, one row per (workspace, user) or per (workspace, user, key). A key's set is the code's to decide (a release
-- may add one), so no CHECK names the keys. The JSON columns are objects, and the navigation control is one of two words.
--
-- Idempotent like the files before it (`migrate` applies every file on every run): every statement says IF NOT EXISTS, and a
-- CHECK is its own statement, dropped and added by name.

CREATE TABLE IF NOT EXISTS workspace_user_preferences (
    workspace_id  uuid NOT NULL REFERENCES workspaces (id),
    user_id       uuid NOT NULL REFERENCES users (id),
    key           text NOT NULL,
    is_pinned     boolean NOT NULL,
    sort_order    double precision NOT NULL,
    created_at    timestamptz NOT NULL,
    updated_at    timestamptz NOT NULL,
    PRIMARY KEY (workspace_id, user_id, key)
);

CREATE TABLE IF NOT EXISTS workspace_home_preferences (
    workspace_id  uuid NOT NULL REFERENCES workspaces (id),
    user_id       uuid NOT NULL REFERENCES users (id),
    key           text NOT NULL,
    is_enabled    boolean NOT NULL,
    config        jsonb NOT NULL,
    sort_order    double precision NOT NULL,
    created_at    timestamptz NOT NULL,
    updated_at    timestamptz NOT NULL,
    PRIMARY KEY (workspace_id, user_id, key)
);
ALTER TABLE workspace_home_preferences DROP CONSTRAINT IF EXISTS workspace_home_preferences_config_is_object;
ALTER TABLE workspace_home_preferences
    ADD CONSTRAINT workspace_home_preferences_config_is_object CHECK (jsonb_typeof(config) = 'object');

CREATE TABLE IF NOT EXISTS workspace_user_properties (
    id                             uuid PRIMARY KEY,
    workspace_id                   uuid NOT NULL REFERENCES workspaces (id),
    user_id                        uuid NOT NULL REFERENCES users (id),
    filters                        jsonb NOT NULL,
    display_filters                jsonb NOT NULL,
    display_properties             jsonb NOT NULL,
    rich_filters                   jsonb NOT NULL,
    navigation_project_limit       integer NOT NULL,
    navigation_control_preference  text NOT NULL,
    created_at                     timestamptz NOT NULL,
    updated_at                     timestamptz NOT NULL,
    UNIQUE (workspace_id, user_id)
);
ALTER TABLE workspace_user_properties DROP CONSTRAINT IF EXISTS workspace_user_properties_json_are_objects;
ALTER TABLE workspace_user_properties ADD CONSTRAINT workspace_user_properties_json_are_objects CHECK (
    jsonb_typeof(filters) = 'object' AND jsonb_typeof(display_filters) = 'object'
    AND jsonb_typeof(display_properties) = 'object' AND jsonb_typeof(rich_filters) = 'object'
);
ALTER TABLE workspace_user_properties DROP CONSTRAINT IF EXISTS workspace_user_properties_navigation_control_valid;
ALTER TABLE workspace_user_properties ADD CONSTRAINT workspace_user_properties_navigation_control_valid
    CHECK (navigation_control_preference IN ('ACCORDION', 'TABBED'));
