-- Issue #44 (the compat slice, asset stage: S28, S29, S31 and S33; ADR-0047): the rows that say whose a stored file is and where it
-- stands. The bytes are in the blob store under `object_key`, never here.
--
-- Idempotent like the files before it (`migrate` applies every file on every run): `CREATE TABLE IF NOT EXISTS`, and each CHECK and
-- foreign key its own statement, dropped and added by name, because `IF NOT EXISTS` skips a table that is there whatever it
-- holds. Only the one kind of asset the stock app's project creation needs exists (a project's cover), so `entity_type` allows only
-- that; a later kind widens the CHECK.

CREATE TABLE IF NOT EXISTS assets (
    id                uuid PRIMARY KEY,
    workspace_id      uuid NOT NULL REFERENCES workspaces (id),
    created_by        uuid NOT NULL REFERENCES users (id),
    entity_type       text NOT NULL,
    -- none until the asset is attached: a cover is uploaded before its project exists
    project_id        uuid,
    -- the file's name, sanitised, as a download is called
    name              text NOT NULL,
    content_type      text NOT NULL,
    -- the size the upload must have, in bytes
    size              bigint NOT NULL,
    object_key        text NOT NULL,
    state             text NOT NULL,
    -- the one-time permission to upload with no session: the digest of the token the address carries, and when it stops working.
    -- Both are null once it is spent (an upload was made) or the asset is uploaded.
    upload_token_hash bytea,
    upload_expires_at timestamptz,
    created_at        timestamptz NOT NULL,
    uploaded_at       timestamptz,
    deleted_at        timestamptz,
    -- a project of this workspace (a null project_id skips the check)
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id)
);

ALTER TABLE assets DROP CONSTRAINT IF EXISTS assets_entity_type_valid;
ALTER TABLE assets ADD CONSTRAINT assets_entity_type_valid CHECK (entity_type IN ('PROJECT_COVER'));
ALTER TABLE assets DROP CONSTRAINT IF EXISTS assets_state_valid;
ALTER TABLE assets ADD CONSTRAINT assets_state_valid CHECK (state IN ('pending', 'uploaded'));
ALTER TABLE assets DROP CONSTRAINT IF EXISTS assets_size_positive;
ALTER TABLE assets ADD CONSTRAINT assets_size_positive CHECK (size > 0);
ALTER TABLE assets DROP CONSTRAINT IF EXISTS assets_token_hash_len;
ALTER TABLE assets ADD CONSTRAINT assets_token_hash_len CHECK (upload_token_hash IS NULL OR octet_length(upload_token_hash) = 32);
-- a grant is a token and its expiry together, or neither
ALTER TABLE assets DROP CONSTRAINT IF EXISTS assets_grant_whole;
ALTER TABLE assets ADD CONSTRAINT assets_grant_whole CHECK ((upload_token_hash IS NULL) = (upload_expires_at IS NULL));
-- an uploaded asset has the time it was, has no grant, and a pending one has no upload time
ALTER TABLE assets DROP CONSTRAINT IF EXISTS assets_uploaded_consistent;
ALTER TABLE assets ADD CONSTRAINT assets_uploaded_consistent CHECK (
    (state = 'uploaded' AND uploaded_at IS NOT NULL AND upload_token_hash IS NULL)
    OR (state = 'pending' AND uploaded_at IS NULL)
);

-- the rows of a project, and of a workspace's member (S31 reads the caller's own uploads)
CREATE INDEX IF NOT EXISTS assets_by_project ON assets (project_id) WHERE project_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS assets_by_creator ON assets (workspace_id, created_by);

-- 0012 added the column and left its foreign key to this stage
ALTER TABLE projects DROP CONSTRAINT IF EXISTS projects_cover_image_asset_fkey;
ALTER TABLE projects ADD CONSTRAINT projects_cover_image_asset_fkey FOREIGN KEY (cover_image_asset) REFERENCES assets (id);
