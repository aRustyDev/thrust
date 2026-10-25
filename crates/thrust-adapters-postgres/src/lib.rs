//! Postgres adapter for the `Database` port, written once against `tokio_postgres::Client`.
//!
//! The only platform-specific part is *how a connected `Client` is obtained*, and that is the
//! [`Connector`] trait:
//!   * native  -> `thrust-adapters-selfhosted`: deadpool-postgres pool over TCP
//!   * Worker  -> `thrust-adapters-cloudflare`: Hyperdrive `Socket` + `Config::connect_raw`
//!
//! Transactions use plain `BEGIN`/`COMMIT` on a connection the transaction owns, so a `Tx` is a
//! self-contained `Send` value (no borrowed `tokio_postgres::Transaction<'_>`).
//!
//! FR-22 (ADR-0043 round 29): the sequence number is taken in its own short transaction, BEFORE the create's. A `Tx`
//! checks its connection out at `begin` but opens no transaction until its first write, so `allocate` (the counter
//! upsert, one statement in autocommit) holds the counter row's lock for one round trip, not for the insert, the outbox
//! row and the commit as when the upsert was the first statement of the transaction. The price is a gap: a number taken
//! by a create that then fails or is abandoned is not given back.

use std::sync::Arc;

use async_trait::async_trait;
use thrust_contracts::*;
use thrust_ports::*;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{Client, Row};

pub const MIGRATION_0001: &str = include_str!("../migrations/0001_init.sql");
pub const MIGRATION_0002: &str = include_str!("../migrations/0002_sequence_tenant_key.sql");
pub const MIGRATION_0003: &str = include_str!("../migrations/0003_workspaces.sql");
pub const MIGRATION_0004: &str = include_str!("../migrations/0004_sessions.sql");
pub const MIGRATION_0005: &str = include_str!("../migrations/0005_identity.sql");
pub const MIGRATION_0006: &str = include_str!("../migrations/0006_search.sql");
pub const MIGRATION_0007: &str = include_str!("../migrations/0007_api_key_hash_version.sql");
pub const MIGRATION_0008: &str = include_str!("../migrations/0008_user_profiles.sql");

pub async fn migrate(client: &Client) -> Result<(), tokio_postgres::Error> {
    client.batch_execute(MIGRATION_0001).await?;
    client.batch_execute(MIGRATION_0002).await?;
    client.batch_execute(MIGRATION_0003).await?;
    client.batch_execute(MIGRATION_0004).await?;
    client.batch_execute(MIGRATION_0005).await?;
    client.batch_execute(MIGRATION_0006).await?;
    client.batch_execute(MIGRATION_0007).await?;
    client.batch_execute(MIGRATION_0008).await
}

pub trait ClientHandle: Send {
    fn client(&self) -> &Client;
    /// A transaction was abandoned without COMMIT/ROLLBACK: the connection must not be reused.
    fn discard(self: Box<Self>) {}
}

impl ClientHandle for Arc<Client> {
    fn client(&self) -> &Client {
        self
    }
}

#[async_trait]
pub trait Connector: Send + Sync {
    /// `exclusive == true` -> a connection no other statement will touch until it is dropped
    /// (used by transactions). `false` -> may be shared.
    async fn checkout(&self, exclusive: bool) -> Result<Box<dyn ClientHandle>, DbError>;
}

fn db_err(e: tokio_postgres::Error) -> DbError {
    if e.code() == Some(&SqlState::UNIQUE_VIOLATION) {
        let c = e.as_db_error().and_then(|d| d.constraint()).unwrap_or("?");
        return DbError::Conflict(format!("unique constraint '{c}' violated"));
    }
    DbError::Backend(e.to_string())
}

// ---------------------------------------------------------------------------------------------
// Statements run as ONE pipeline on the UNNAMED statement
// ---------------------------------------------------------------------------------------------
//
// tokio-postgres' default `query`/`execute(&str, ..)` PREPARES a named statement (`Parse`,
// `Describe`, `Sync`) and then `Bind`/`Execute`s it in a second round trip. Through Hyperdrive that
// flow was unreliable in Stage 2 (research/0004 §10): statements with no result columns (INSERT or
// UPDATE without RETURNING) failed every time (`connection closed`) and SELECTs failed intermittently
// (`unexpected message from server`). The `*_typed` calls send `Parse`, `Bind`, `Describe`, `Execute`,
// `Sync` as one pipeline on the unnamed statement; that passed every time, in one round trip instead
// of two. The price: no server-side statement caching, and every parameter must carry its Postgres
// type, which is what `PgParam` is for.

/// The Postgres type of a bind parameter, known statically so `Option::<T>::None` still has one.
pub trait PgType {
    const TY: Type;
}

macro_rules! pg_type {
    ($($t:ty => $ty:expr),* $(,)?) => { $(impl PgType for $t { const TY: Type = $ty; })* };
}
pg_type!(
    Uuid => Type::UUID,
    i64 => Type::INT8,
    i32 => Type::INT4,
    // `api_keys.hash_version` is a smallint
    i16 => Type::INT2,
    bool => Type::BOOL,
    String => Type::TEXT,
    Timestamp => Type::TIMESTAMPTZ,
    serde_json::Value => Type::JSONB,
);
/// Whole columns at once, for the bulk seed's `unnest`.
impl PgType for Vec<Uuid> {
    const TY: Type = Type::UUID_ARRAY;
}
impl PgType for Vec<String> {
    const TY: Type = Type::TEXT_ARRAY;
}
impl PgType for Vec<Option<String>> {
    const TY: Type = Type::TEXT_ARRAY;
}
impl PgType for &str {
    const TY: Type = Type::TEXT;
}
/// A session token's hash (`sessions.token_hash`).
impl PgType for &[u8] {
    const TY: Type = Type::BYTEA;
}
impl<T: PgType> PgType for Option<T> {
    const TY: Type = T::TY;
}

/// Object-safe view of a bind parameter that can tell the server its type.
pub trait PgParam: ToSql + Sync {
    fn pg_type(&self) -> Type;
}
impl<T: PgType + ToSql + Sync> PgParam for T {
    fn pg_type(&self) -> Type {
        T::TY
    }
}

type Params<'a> = &'a [&'a dyn PgParam];

fn typed<'a>(p: Params<'a>) -> Vec<(&'a (dyn ToSql + Sync), Type)> {
    p.iter().map(|x| (*x as &(dyn ToSql + Sync), x.pg_type())).collect()
}

/// A connected client whose statements all go through the single-pipeline typed calls.
#[derive(Clone, Copy)]
pub struct Pg<'a>(pub &'a Client);

impl Pg<'_> {
    pub async fn query(&self, sql: &str, p: Params<'_>) -> Result<Vec<Row>, tokio_postgres::Error> {
        self.0.query_typed(sql, &typed(p)).await
    }
    pub async fn query_opt(&self, sql: &str, p: Params<'_>) -> Result<Option<Row>, tokio_postgres::Error> {
        self.0.query_typed_opt(sql, &typed(p)).await
    }
    pub async fn query_one(&self, sql: &str, p: Params<'_>) -> Result<Row, tokio_postgres::Error> {
        self.0.query_typed_one(sql, &typed(p)).await
    }
    pub async fn execute(&self, sql: &str, p: Params<'_>) -> Result<u64, tokio_postgres::Error> {
        self.0.execute_typed(sql, &typed(p)).await
    }
    /// Simple query protocol; no parameters (BEGIN / COMMIT / ROLLBACK).
    pub async fn batch_execute(&self, sql: &str) -> Result<(), tokio_postgres::Error> {
        self.0.batch_execute(sql).await
    }
}

pub struct PgDatabase {
    connector: Arc<dyn Connector>,
}

impl PgDatabase {
    pub fn new(connector: Arc<dyn Connector>) -> Self {
        Self { connector }
    }

    async fn conn(&self) -> Result<Box<dyn ClientHandle>, DbError> {
        self.connector.checkout(false).await
    }
}

const WI_COLS: &str = "id, workspace_id, project_id, sequence_id, sort_order, name, description, \
    external_source, external_id, created_by, created_at, archived_at, is_draft, deleted_at";

fn work_item(r: &Row) -> WorkItem {
    WorkItem {
        id: WorkItemId(r.get("id")),
        workspace_id: WorkspaceId(r.get("workspace_id")),
        project_id: ProjectId(r.get("project_id")),
        sequence_id: r.get("sequence_id"),
        sort_order: r.get("sort_order"),
        name: r.get("name"),
        description: r.get("description"),
        external_source: r.get("external_source"),
        external_id: r.get("external_id"),
        created_by: UserId(r.get("created_by")),
        created_at: r.get("created_at"),
        archived_at: r.get("archived_at"),
        is_draft: r.get("is_draft"),
        deleted_at: r.get("deleted_at"),
    }
}

/// `%query%` for `ILIKE ... ESCAPE '\'`, with the query's own `\`, `%` and `_` escaped so they match themselves.
fn like_pattern(query: &str) -> String {
    let mut p = String::with_capacity(query.len() + 2);
    p.push('%');
    for c in query.chars() {
        if matches!(c, '\\' | '%' | '_') {
            p.push('\\');
        }
        p.push(c);
    }
    p.push('%');
    p
}

const USER_COLS: &str = "id, email, password_hash, is_active";

fn user_record(r: &Row) -> UserRecord {
    UserRecord {
        id: UserId(r.get("id")),
        email: r.get("email"),
        password_hash: r.get("password_hash"),
        is_active: r.get("is_active"),
    }
}

const PROFILE_COLS: &str = "user_id, id, username, first_name, last_name, display_name, avatar, cover_image, timezone, \
    date_joined, last_login_at, is_onboarded, is_tour_completed, last_workspace_id, onboarding_step, preferences, \
    created_at, updated_at";

fn user_profile(r: &Row) -> UserProfile {
    UserProfile {
        id: r.get("id"),
        user_id: UserId(r.get("user_id")),
        username: r.get("username"),
        first_name: r.get("first_name"),
        last_name: r.get("last_name"),
        display_name: r.get("display_name"),
        avatar: r.get("avatar"),
        cover_image: r.get("cover_image"),
        timezone: r.get("timezone"),
        date_joined: r.get("date_joined"),
        last_login_at: r.get("last_login_at"),
        is_onboarded: r.get("is_onboarded"),
        is_tour_completed: r.get("is_tour_completed"),
        last_workspace_id: r.get::<_, Option<Uuid>>("last_workspace_id").map(WorkspaceId),
        onboarding_step: r.get("onboarding_step"),
        preferences: r.get("preferences"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn activity(r: &Row) -> Activity {
    Activity {
        id: ActivityId(r.get("id")),
        event_id: EventId(r.get("event_id")),
        workspace_id: WorkspaceId(r.get("workspace_id")),
        project_id: ProjectId(r.get("project_id")),
        entity_id: r.get("entity_id"),
        verb: r.get("verb"),
        actor_id: r.get::<_, Option<Uuid>>("actor_id").map(UserId),
        created_at: r.get("created_at"),
    }
}

#[async_trait]
impl Database for PgDatabase {
    async fn membership(
        &self,
        slug: &WorkspaceSlug,
        project: ProjectId,
        user: UserId,
    ) -> Result<Membership, DbError> {
        let c = self.conn().await?;
        // One query: the slug, the caller's workspace membership (inner join), the project in that
        // workspace (inner join) and the caller's project row (left join, same workspace). No row
        // means the caller may not know the workspace or the project exists.
        let row = Pg(c.client())
            .query_opt(
                "SELECT w.id AS workspace_id, m.role AS role FROM workspaces w \
                 JOIN workspace_members wm ON wm.workspace_id = w.id AND wm.user_id = $3 \
                 JOIN projects p ON p.workspace_id = w.id AND p.id = $2 \
                 LEFT JOIN project_members m \
                   ON m.project_id = p.id AND m.workspace_id = p.workspace_id AND m.user_id = $3 \
                 WHERE w.slug = $1",
                &[&slug.as_str(), &project.0, &user.0],
            )
            .await
            .map_err(db_err)?;
        Ok(match row {
            None => Membership::NoSuchProject,
            Some(r) => match r.get::<_, Option<String>>("role").as_deref().and_then(Role::parse) {
                Some(role) => Membership::Member { workspace_id: WorkspaceId(r.get("workspace_id")), role },
                None => Membership::NotAMember,
            },
        })
    }

    async fn session(&self, hash: &TokenHash, now: Timestamp) -> Result<Option<LiveSession>, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client())
            .query_opt(
                "SELECT s.user_id, s.last_seen_at FROM sessions s JOIN users u ON u.id = s.user_id \
                 WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > $2 AND u.is_active",
                &[&&hash.0[..], &now],
            )
            .await
            .map_err(db_err)?;
        Ok(row.map(|r| LiveSession { user_id: UserId(r.get("user_id")), last_seen_at: r.get("last_seen_at") }))
    }

    async fn touch_session(&self, hash: &TokenHash, at: Timestamp) -> Result<(), DbError> {
        let c = self.conn().await?;
        Pg(c.client())
            .execute("UPDATE sessions SET last_seen_at = $2 WHERE token_hash = $1", &[&&hash.0[..], &at])
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn revoke_session(&self, hash: &TokenHash, at: Timestamp) -> Result<(), DbError> {
        let c = self.conn().await?;
        Pg(c.client())
            .execute(
                "UPDATE sessions SET revoked_at = $2 WHERE token_hash = $1 AND revoked_at IS NULL",
                &[&&hash.0[..], &at],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn insert_session(&self, s: &NewSession) -> Result<(), DbError> {
        let c = self.conn().await?;
        Pg(c.client())
            .execute(
                "INSERT INTO sessions (token_hash, user_id, created_at, expires_at, last_seen_at) \
                 VALUES ($1, $2, $3, $4, $3) ON CONFLICT (token_hash) DO NOTHING",
                &[&&s.token_hash.0[..], &s.user_id.0, &s.created_at, &s.expires_at],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn revoke_user_sessions(&self, user: UserId, at: Timestamp) -> Result<u64, DbError> {
        let c = self.conn().await?;
        Pg(c.client())
            .execute(
                "UPDATE sessions SET revoked_at = $2 WHERE user_id = $1 AND revoked_at IS NULL",
                &[&user.0, &at],
            )
            .await
            .map_err(db_err)
    }

    async fn user_by_email(&self, email: &str) -> Result<Option<UserRecord>, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client())
            .query_opt(&format!("SELECT {USER_COLS} FROM users WHERE email = $1"), &[&email])
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(user_record))
    }

    async fn user_by_id(&self, id: UserId) -> Result<Option<UserRecord>, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client())
            .query_opt(&format!("SELECT {USER_COLS} FROM users WHERE id = $1"), &[&id.0])
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(user_record))
    }

    async fn insert_profile(&self, p: &UserProfile) -> Result<bool, DbError> {
        // ON CONFLICT on the user only: a taken `id` or `username` is a unique violation, which `db_err` makes a `Conflict`
        let c = self.conn().await?;
        let n = Pg(c.client())
            .execute(
                &format!(
                    "INSERT INTO user_profiles ({PROFILE_COLS}) VALUES \
                     ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18) \
                     ON CONFLICT (user_id) DO NOTHING"
                ),
                &[
                    &p.user_id.0,
                    &p.id,
                    &p.username,
                    &p.first_name,
                    &p.last_name,
                    &p.display_name,
                    &p.avatar,
                    &p.cover_image,
                    &p.timezone,
                    &p.date_joined,
                    &p.last_login_at,
                    &p.is_onboarded,
                    &p.is_tour_completed,
                    &p.last_workspace_id.map(|w| w.0),
                    &p.onboarding_step,
                    &p.preferences,
                    &p.created_at,
                    &p.updated_at,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn profile(&self, user: UserId) -> Result<Option<UserProfile>, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client())
            .query_opt(&format!("SELECT {PROFILE_COLS} FROM user_profiles WHERE user_id = $1"), &[&user.0])
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(user_profile))
    }

    async fn update_profile(&self, user: UserId, patch: &ProfilePatch, at: Timestamp) -> Result<Option<UserProfile>, DbError> {
        // ONE statement: every column the patch does not name keeps its value, and the preferences are merged by jsonb `||`
        // inside the row lock, so two requests that change different fields cannot overwrite each other.
        let set_cover = patch.cover_image.is_some();
        let cover: Option<&str> = patch.cover_image.as_ref().and_then(|c| c.as_deref());
        let set_workspace = patch.last_workspace_id.is_some();
        let workspace: Option<Uuid> = patch.last_workspace_id.and_then(|w| w.map(|w| w.0));
        let preferences = serde_json::Value::Object(patch.preferences.clone());
        let c = self.conn().await?;
        let row = Pg(c.client())
            .query_opt(
                &format!(
                    "UPDATE user_profiles SET \
                       first_name = COALESCE($2, first_name), last_name = COALESCE($3, last_name), \
                       display_name = COALESCE($4, display_name), avatar = COALESCE($5, avatar), \
                       cover_image = CASE WHEN $6 THEN $7 ELSE cover_image END, \
                       timezone = COALESCE($8, timezone), last_login_at = COALESCE($9, last_login_at), \
                       is_onboarded = COALESCE($10, is_onboarded), is_tour_completed = COALESCE($11, is_tour_completed), \
                       last_workspace_id = CASE WHEN $12 THEN $13 ELSE last_workspace_id END, \
                       onboarding_step = COALESCE($14, onboarding_step), preferences = preferences || $15, \
                       updated_at = $16 \
                     WHERE user_id = $1 RETURNING {PROFILE_COLS}"
                ),
                &[
                    &user.0,
                    &patch.first_name.as_deref(),
                    &patch.last_name.as_deref(),
                    &patch.display_name.as_deref(),
                    &patch.avatar.as_deref(),
                    &set_cover,
                    &cover,
                    &patch.timezone.as_deref(),
                    &patch.last_login_at,
                    &patch.is_onboarded,
                    &patch.is_tour_completed,
                    &set_workspace,
                    &workspace,
                    &patch.onboarding_step,
                    &preferences,
                    &at,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(user_profile))
    }

    async fn insert_user(&self, user: &UserRecord) -> Result<(), DbError> {
        // No ON CONFLICT: a taken id or email must fail, and `db_err` turns the unique violation into `Conflict`.
        let c = self.conn().await?;
        Pg(c.client())
            .execute(
                "INSERT INTO users (id, email, password_hash, is_active) VALUES ($1, $2, $3, $4)",
                &[&user.id.0, &user.email.as_deref(), &user.password_hash.as_deref(), &user.is_active],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn user_for_account(
        &self,
        provider: &str,
        provider_account_id: &str,
    ) -> Result<Option<UserRecord>, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client())
            .query_opt(
                "SELECT u.id, u.email, u.password_hash, u.is_active FROM accounts a JOIN users u ON u.id = a.user_id \
                 WHERE a.provider = $1 AND a.provider_account_id = $2",
                &[&provider, &provider_account_id],
            )
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(user_record))
    }

    async fn link_account(&self, a: &NewAccount) -> Result<(), DbError> {
        let c = self.conn().await?;
        Pg(c.client())
            .execute(
                "INSERT INTO accounts (provider, provider_account_id, user_id, email, created_at) \
                 VALUES ($1, $2, $3, $4, $5) ON CONFLICT (provider, provider_account_id) DO NOTHING",
                &[&a.provider.as_str(), &a.provider_account_id.as_str(), &a.user_id.0, &a.email.as_deref(), &a.created_at],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn insert_api_key(&self, k: &NewApiKey) -> Result<(), DbError> {
        let c = self.conn().await?;
        Pg(c.client())
            .execute(
                "INSERT INTO api_keys (id, user_id, token_hash, hash_version, prefix, label, created_at, expires_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &k.id.0,
                    &k.user_id.0,
                    &&k.token_hash.0[..],
                    &k.hash_version,
                    &k.prefix.as_str(),
                    &k.label.as_str(),
                    &k.created_at,
                    &k.expires_at,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn api_key(&self, hash: &TokenHash, now: Timestamp) -> Result<Option<LiveApiKey>, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client())
            .query_opt(
                "SELECT k.id, k.user_id, k.last_used_at, k.hash_version FROM api_keys k JOIN users u ON u.id = k.user_id \
                 WHERE k.token_hash = $1 AND k.revoked_at IS NULL AND (k.expires_at IS NULL OR k.expires_at > $2) \
                 AND u.is_active",
                &[&&hash.0[..], &now],
            )
            .await
            .map_err(db_err)?;
        Ok(row.map(|r| LiveApiKey {
            id: ApiKeyId(r.get("id")),
            user_id: UserId(r.get("user_id")),
            last_used_at: r.get("last_used_at"),
            hash_version: r.get("hash_version"),
        }))
    }

    async fn touch_api_key(&self, id: ApiKeyId, at: Timestamp) -> Result<(), DbError> {
        let c = self.conn().await?;
        Pg(c.client())
            .execute("UPDATE api_keys SET last_used_at = $2 WHERE id = $1", &[&id.0, &at])
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn rehash_api_key(
        &self,
        id: ApiKeyId,
        from_version: i16,
        new_hash: &TokenHash,
        new_version: i16,
    ) -> Result<bool, DbError> {
        let c = self.conn().await?;
        // The compare-and-set is the WHERE clause: a row that is not (or no longer) at `from_version` is not touched,
        // and neither of its other constraints is looked at. Owner, expiry, revocation and last use stay.
        let n = Pg(c.client())
            .execute(
                "UPDATE api_keys SET token_hash = $3, hash_version = $4 WHERE id = $1 AND hash_version = $2",
                &[&id.0, &from_version, &&new_hash.0[..], &new_version],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn list_api_keys(&self, user: UserId) -> Result<Vec<ApiKeyInfo>, DbError> {
        let c = self.conn().await?;
        let rows = Pg(c.client())
            .query(
                "SELECT id, prefix, label, created_at, expires_at, revoked_at, last_used_at FROM api_keys \
                 WHERE user_id = $1 ORDER BY created_at, id",
                &[&user.0],
            )
            .await
            .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|r| ApiKeyInfo {
                id: ApiKeyId(r.get("id")),
                prefix: r.get("prefix"),
                label: r.get("label"),
                created_at: r.get("created_at"),
                expires_at: r.get("expires_at"),
                revoked_at: r.get("revoked_at"),
                last_used_at: r.get("last_used_at"),
            })
            .collect())
    }

    async fn revoke_api_key(&self, user: UserId, id: ApiKeyId, at: Timestamp) -> Result<bool, DbError> {
        let c = self.conn().await?;
        // COALESCE keeps the first revocation time, so a second revoke changes nothing but still says "yours".
        let n = Pg(c.client())
            .execute(
                "UPDATE api_keys SET revoked_at = COALESCE(revoked_at, $3) WHERE id = $1 AND user_id = $2",
                &[&id.0, &user.0, &at],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn insert_oidc_login(&self, l: &NewOidcLogin) -> Result<(), DbError> {
        let c = self.conn().await?;
        let pg = Pg(c.client());
        pg.execute("DELETE FROM oidc_logins WHERE expires_at <= $1", &[&l.created_at]).await.map_err(db_err)?;
        pg.execute(
            "INSERT INTO oidc_logins (state_hash, nonce, code_verifier, created_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5)",
            &[&&l.state_hash.0[..], &l.nonce.as_str(), &l.code_verifier.as_str(), &l.created_at, &l.expires_at],
        )
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn take_oidc_login(&self, state: &TokenHash) -> Result<Option<OidcLogin>, DbError> {
        let c = self.conn().await?;
        // One statement: the row is returned to exactly one caller, and gone for the next.
        let row = Pg(c.client())
            .query_opt(
                "DELETE FROM oidc_logins WHERE state_hash = $1 RETURNING nonce, code_verifier, expires_at",
                &[&&state.0[..]],
            )
            .await
            .map_err(db_err)?;
        Ok(row.map(|r| OidcLogin {
            nonce: r.get("nonce"),
            code_verifier: r.get("code_verifier"),
            expires_at: r.get("expires_at"),
        }))
    }

    async fn set_user_active(&self, _sys: &SystemScope, user: UserId, active: bool) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let n = Pg(c.client())
            .execute("UPDATE users SET is_active = $2 WHERE id = $1", &[&user.0, &active])
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn seed_users(&self, _sys: &SystemScope, users: &[UserRecord]) -> Result<(), DbError> {
        let c = self.connector.checkout(true).await?;
        let cl = Pg(c.client());
        cl.batch_execute("BEGIN").await.map_err(db_err)?;
        let res: Result<(), DbError> = async {
            for u in users {
                cl.execute(
                    "INSERT INTO users (id, email, password_hash, is_active) VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (id) DO UPDATE SET email = EXCLUDED.email, password_hash = EXCLUDED.password_hash, \
                       is_active = EXCLUDED.is_active",
                    &[&u.id.0, &u.email.as_deref(), &u.password_hash.as_deref(), &u.is_active],
                )
                .await
                .map_err(db_err)?;
            }
            Ok(())
        }
        .await;
        match res {
            Ok(()) => cl.batch_execute("COMMIT").await.map_err(db_err),
            Err(e) => {
                let _ = cl.batch_execute("ROLLBACK").await;
                Err(e)
            }
        }
    }

    async fn begin(&self, _scope: &Scope) -> Result<Box<dyn Tx>, DbError> {
        // The connection is the transaction's own, but nothing is open on it yet: `BEGIN` waits for the first write
        // (FR-22, see the module docs), so `allocate` can run before it.
        let conn = self.connector.checkout(true).await?;
        Ok(Box::new(PgTx { conn: Some(conn), in_tx: false, connector: self.connector.clone() }))
    }

    async fn list_work_items(&self, scope: &Scope, vis: Visibility) -> Result<Vec<WorkItem>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let sql = match vis {
            Visibility::Visible => format!(
                "SELECT {WI_COLS} FROM work_items WHERE workspace_id = $1 AND project_id = $2 \
                 AND deleted_at IS NULL AND archived_at IS NULL AND is_draft = false ORDER BY sequence_id"
            ),
            Visibility::WithDeleted => format!(
                "SELECT {WI_COLS} FROM work_items WHERE workspace_id = $1 AND project_id = $2 ORDER BY sequence_id"
            ),
        };
        let c = self.conn().await?;
        let rows = Pg(c.client()).query(&sql, &[&ws.0, &project.0]).await.map_err(db_err)?;
        Ok(rows.iter().map(work_item).collect())
    }

    async fn get_work_item(
        &self,
        scope: &Scope,
        id: WorkItemId,
        vis: Visibility,
    ) -> Result<Option<WorkItem>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let hidden = match vis {
            Visibility::Visible => " AND deleted_at IS NULL AND archived_at IS NULL AND is_draft = false",
            Visibility::WithDeleted => "",
        };
        let sql = format!(
            "SELECT {WI_COLS} FROM work_items WHERE workspace_id = $1 AND project_id = $2 AND id = $3{hidden}"
        );
        let c = self.conn().await?;
        let row = Pg(c.client()).query_opt(&sql, &[&ws.0, &project.0, &id.0]).await.map_err(db_err)?;
        Ok(row.as_ref().map(work_item))
    }

    async fn search_work_items(&self, scope: &Scope, query: &str, limit: u32) -> Result<Vec<WorkItem>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        // `%`, `_` and `\` are literal in a query, so they are escaped for LIKE (the trigram GIN index serves `ILIKE`).
        let pattern = like_pattern(query);
        let c = self.conn().await?;
        let rows = Pg(c.client())
            .query(
                &format!(
                    "SELECT {WI_COLS} FROM work_items WHERE workspace_id = $1 AND project_id = $2 \
                     AND deleted_at IS NULL AND archived_at IS NULL AND is_draft = false \
                     AND (name ILIKE $3 ESCAPE '\\' OR description ILIKE $3 ESCAPE '\\') ORDER BY sequence_id LIMIT $4"
                ),
                &[&ws.0, &project.0, &pattern, &i64::from(limit)],
            )
            .await
            .map_err(db_err)?;
        Ok(rows.iter().map(work_item).collect())
    }

    async fn seed_work_items(
        &self,
        _sys: &SystemScope,
        workspace: WorkspaceId,
        project: ProjectId,
        items: &[SeedWorkItem],
    ) -> Result<i64, DbError> {
        // One transaction on a connection of its own: the counter advance and the rows commit together. The rows go in as
        // columns (`unnest`), so 5,000 of them are one round trip, which is what makes 100k seedable through Hyperdrive.
        let n = items.len() as i64;
        let c = self.connector.checkout(true).await?;
        let cl = Pg(c.client());
        cl.batch_execute("BEGIN").await.map_err(db_err)?;
        let res: Result<i64, DbError> = async {
            let r = cl
                .query_one(
                    "INSERT INTO project_sequences (workspace_id, project_id, last_sequence_id, last_sort_order) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (workspace_id, project_id) DO UPDATE SET \
                       last_sequence_id = project_sequences.last_sequence_id + $3, \
                       last_sort_order  = project_sequences.last_sort_order + $4 \
                     RETURNING last_sequence_id, last_sort_order",
                    &[&workspace.0, &project.0, &n, &(n * 10_000)],
                )
                .await
                .map_err(db_err)?;
            let (last_sequence, last_sort): (i64, i64) = (r.get(0), r.get(1));
            let (base_sequence, base_sort) = (last_sequence - n, last_sort - n * 10_000);
            let ids: Vec<Uuid> = items.iter().map(|i| i.id.0).collect();
            let names: Vec<String> = items.iter().map(|i| i.name.clone()).collect();
            let descriptions: Vec<Option<String>> = items.iter().map(|i| i.description.clone()).collect();
            let (created_by, created_at) = items.first().map(|i| (i.created_by.0, i.created_at)).ok_or_else(|| DbError::Backend("no items".into()))?;
            cl.execute(
                "INSERT INTO work_items (id, workspace_id, project_id, sequence_id, sort_order, name, description, created_by, created_at, is_draft) \
                 SELECT t.id, $1, $2, $3 + t.ord, $4 + t.ord * 10000, t.name, t.description, $5, $6, false \
                 FROM unnest($7, $8, $9) WITH ORDINALITY AS t(id, name, description, ord)",
                &[&workspace.0, &project.0, &base_sequence, &base_sort, &created_by, &created_at, &ids, &names, &descriptions],
            )
            .await
            .map_err(db_err)?;
            Ok(base_sequence + 1)
        }
        .await;
        match res {
            Ok(first) => cl.batch_execute("COMMIT").await.map(|()| first).map_err(db_err),
            Err(e) => {
                let _ = cl.batch_execute("ROLLBACK").await;
                Err(e)
            }
        }
    }

    async fn insert_activity(&self, _sys: &SystemScope, a: &Activity) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let n = Pg(c.client())
            .execute(
                "INSERT INTO activity (id, event_id, workspace_id, project_id, entity_id, verb, actor_id, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (event_id) DO NOTHING",
                &[
                    &a.id.0,
                    &a.event_id.0,
                    &a.workspace_id.0,
                    &a.project_id.0,
                    &a.entity_id,
                    &a.verb,
                    &a.actor_id.map(|u| u.0),
                    &a.created_at,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn list_activity(&self, scope: &Scope, entity: Option<WorkItemId>) -> Result<Vec<Activity>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let c = self.conn().await?;
        let entity: Option<Uuid> = entity.map(|e| e.0);
        let rows = Pg(c.client())
            .query(
                "SELECT id, event_id, workspace_id, project_id, entity_id, verb, actor_id, created_at \
                 FROM activity WHERE workspace_id = $1 AND project_id = $2 \
                 AND ($3::uuid IS NULL OR entity_id = $3::uuid) ORDER BY created_at, id",
                &[&ws.0, &project.0, &entity],
            )
            .await
            .map_err(db_err)?;
        Ok(rows.iter().map(activity).collect())
    }

    async fn undispatched_outbox(&self, _sys: &SystemScope, limit: u32) -> Result<Vec<OutboxRow>, DbError> {
        let c = self.conn().await?;
        let rows = Pg(c.client())
            .query(
                "SELECT envelope, created_at, attempts FROM outbox WHERE dispatched_at IS NULL \
                 ORDER BY created_at, event_id LIMIT $1",
                &[&(limit as i64)],
            )
            .await
            .map_err(db_err)?;
        rows.iter()
            .map(|r| {
                let v: serde_json::Value = r.get("envelope");
                let envelope: Envelope =
                    serde_json::from_value(v).map_err(|e| DbError::Backend(format!("bad envelope: {e}")))?;
                Ok(OutboxRow {
                    envelope,
                    created_at: r.get("created_at"),
                    dispatched_at: None,
                    attempts: r.get("attempts"),
                })
            })
            .collect()
    }

    async fn mark_outbox_dispatched(&self, _sys: &SystemScope, event: EventId, at: Timestamp) -> Result<(), DbError> {
        let c = self.conn().await?;
        Pg(c.client())
            .execute(
                "UPDATE outbox SET dispatched_at = $2, attempts = attempts + 1 WHERE event_id = $1",
                &[&event.0, &at],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn outbox_depth(&self, _sys: &SystemScope) -> Result<u64, DbError> {
        let c = self.conn().await?;
        let r = Pg(c.client())
            .query_one("SELECT count(*) AS n FROM outbox WHERE dispatched_at IS NULL", &[])
            .await
            .map_err(db_err)?;
        Ok(r.get::<_, i64>("n") as u64)
    }

    async fn seed(&self, _sys: &SystemScope, f: &SeedFixture) -> Result<(), DbError> {
        // `Tx` only exposes slice operations, so the fixture goes through a connection directly.
        let c = self.connector.checkout(true).await?;
        let cl = Pg(c.client());
        cl.batch_execute("BEGIN").await.map_err(db_err)?;
        let res: Result<(), DbError> = async {
            // users that exist by id only; a row that is there already (with an email, say) is left as it is
            for u in &f.users {
                cl.execute("INSERT INTO users (id) VALUES ($1) ON CONFLICT (id) DO NOTHING", &[&u.0])
                    .await
                    .map_err(db_err)?;
            }
            for w in &f.workspaces {
                cl.execute(
                    "INSERT INTO workspaces (id, slug) VALUES ($1, $2) ON CONFLICT (id) DO UPDATE SET slug = EXCLUDED.slug",
                    &[&w.id.0, &w.slug.as_str()],
                )
                .await
                .map_err(db_err)?;
            }
            for m in &f.workspace_members {
                cl.execute(
                    "INSERT INTO workspace_members (workspace_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                    &[&m.workspace_id.0, &m.user_id.0],
                )
                .await
                .map_err(db_err)?;
            }
            for p in &f.projects {
                cl.execute(
                    "INSERT INTO projects (id, workspace_id, name) VALUES ($1, $2, $3) ON CONFLICT (id) DO NOTHING",
                    &[&p.id.0, &p.workspace_id.0, &p.name],
                )
                .await
                .map_err(db_err)?;
            }
            for m in &f.memberships {
                cl.execute(
                    "INSERT INTO project_members (project_id, workspace_id, user_id, role) VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (project_id, user_id) DO UPDATE SET role = EXCLUDED.role",
                    &[&m.project_id.0, &m.workspace_id.0, &m.user_id.0, &m.role.as_str()],
                )
                .await
                .map_err(db_err)?;
            }
            Ok(())
        }
        .await;
        match res {
            Ok(()) => cl.batch_execute("COMMIT").await.map_err(db_err),
            Err(e) => {
                let _ = cl.batch_execute("ROLLBACK").await;
                Err(e)
            }
        }
    }
}

pub struct PgTx {
    conn: Option<Box<dyn ClientHandle>>,
    /// `BEGIN` has been sent on `conn`: the first write opens the transaction.
    in_tx: bool,
    connector: Arc<dyn Connector>,
}

impl PgTx {
    fn client(&self) -> Pg<'_> {
        Pg(self.conn.as_ref().expect("transaction already finished").client())
    }

    /// Open the transaction on first use. If `BEGIN` fails the connection's state is unknown, so it is not reused.
    async fn ensure_tx(&mut self) -> Result<(), DbError> {
        if self.in_tx {
            return Ok(());
        }
        match self.client().batch_execute("BEGIN").await {
            Ok(()) => {
                self.in_tx = true;
                Ok(())
            }
            Err(e) => {
                if let Some(c) = self.conn.take() {
                    c.discard();
                }
                Err(db_err(e))
            }
        }
    }
}

impl Drop for PgTx {
    fn drop(&mut self) {
        // Dropped without commit/rollback (e.g. the request future was cancelled): never return a connection that is
        // still inside a transaction to the pool. One that never opened a transaction is clean and goes back as it is.
        if let Some(c) = self.conn.take() {
            if self.in_tx {
                c.discard();
            }
        }
    }
}

#[async_trait]
impl Tx for PgTx {
    /// A scope whose project is outside its workspace violates `project_sequences_tenant_fkey`
    /// (migration 0002): `Backend`, and nothing is taken. `authorize` never mints one; a test can.
    ///
    /// FR-22: the upsert is a transaction of its own (one statement, autocommit), so the counter row is locked for one
    /// round trip. Before the first write that is this transaction's own connection; if a write has opened the
    /// transaction already (a second `allocate` in one create), another connection takes the number, because on this
    /// one the upsert would join the transaction and keep the lock until it ends.
    async fn allocate(&mut self, scope: &Scope) -> Result<Allocation, DbError> {
        const UPSERT: &str = "INSERT INTO project_sequences (workspace_id, project_id, last_sequence_id, last_sort_order) \
             VALUES ($1, $2, 1, 10000) \
             ON CONFLICT (workspace_id, project_id) DO UPDATE SET \
               last_sequence_id = project_sequences.last_sequence_id + 1, \
               last_sort_order  = project_sequences.last_sort_order + 10000 \
             RETURNING last_sequence_id, last_sort_order";
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let r = if self.in_tx {
            let other = self.connector.checkout(true).await?;
            let r = Pg(other.client()).query_one(UPSERT, &[&ws.0, &project.0]).await;
            drop(other); // autocommit: nothing is open on it, it is clean
            r
        } else {
            self.client().query_one(UPSERT, &[&ws.0, &project.0]).await
        }
        .map_err(db_err)?;
        Ok(Allocation { sequence_id: r.get(0), sort_order: r.get(1) })
    }

    async fn insert_work_item(&mut self, scope: &Scope, w: &WorkItem) -> Result<(), DbError> {
        check_tenant(scope, w.workspace_id, Some(w.project_id))?;
        self.ensure_tx().await?;
        self.client()
            .execute(
                &format!(
                    "INSERT INTO work_items ({WI_COLS}) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"
                ),
                &[
                    &w.id.0,
                    &w.workspace_id.0,
                    &w.project_id.0,
                    &w.sequence_id,
                    &w.sort_order,
                    &w.name,
                    &w.description,
                    &w.external_source,
                    &w.external_id,
                    &w.created_by.0,
                    &w.created_at,
                    &w.archived_at,
                    &w.is_draft,
                    &w.deleted_at,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn soft_delete_work_item(&mut self, scope: &Scope, id: WorkItemId, at: Timestamp) -> Result<bool, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        self.ensure_tx().await?;
        let n = self
            .client()
            .execute(
                "UPDATE work_items SET deleted_at = $4 \
                 WHERE workspace_id = $1 AND project_id = $2 AND id = $3 AND deleted_at IS NULL",
                &[&ws.0, &project.0, &id.0, &at],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn insert_outbox(&mut self, scope: &Scope, row: &OutboxRow) -> Result<(), DbError> {
        check_tenant(scope, row.envelope.workspace_id, row.envelope.project_id())?;
        let envelope = serde_json::to_value(&row.envelope).map_err(|e| DbError::Backend(e.to_string()))?;
        let params: Params = &[
            &row.envelope.event_id.0,
            &row.envelope.workspace_id.0,
            &envelope,
            &row.created_at,
        ];
        self.ensure_tx().await?;
        self.client()
            .execute(
                "INSERT INTO outbox (event_id, workspace_id, envelope, created_at, dispatched_at, attempts) \
                 VALUES ($1, $2, $3, $4, NULL, 0)",
                params,
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<(), DbError> {
        let mut this = *self;
        let in_tx = this.in_tx;
        let conn = this.conn.take().expect("transaction already finished");
        if !in_tx {
            // Nothing was written, so no transaction was opened: the connection is clean and goes back as it is.
            return Ok(());
        }
        match conn.client().batch_execute("COMMIT").await {
            Ok(()) => Ok(()),
            Err(e) => {
                conn.discard();
                Err(db_err(e))
            }
        }
    }

    async fn rollback(self: Box<Self>) -> Result<(), DbError> {
        let mut this = *self;
        let in_tx = this.in_tx;
        let conn = this.conn.take().expect("transaction already finished");
        if !in_tx {
            return Ok(());
        }
        match conn.client().batch_execute("ROLLBACK").await {
            Ok(()) => Ok(()),
            Err(e) => {
                conn.discard();
                Err(db_err(e))
            }
        }
    }
}
