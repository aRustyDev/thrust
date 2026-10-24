//! Postgres adapter for the `Database` port, written once against `tokio_postgres::Client`.
//!
//! The only platform-specific part is *how a connected `Client` is obtained*, and that is the
//! [`Connector`] trait:
//!   * native  -> `thrust-adapters-selfhosted`: deadpool-postgres pool over TCP
//!   * Worker  -> `thrust-adapters-cloudflare`: Hyperdrive `Socket` + `Config::connect_raw`
//!
//! Transactions use plain `BEGIN`/`COMMIT` on a connection the transaction owns, so a `Tx` is a
//! self-contained `Send` value (no borrowed `tokio_postgres::Transaction<'_>`).

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
// 0006 is the SQLite adapter's (search indexes); `migrate` applies files in order, so the gap is harmless.
pub const MIGRATION_0007: &str = include_str!("../migrations/0007_api_key_hash_version.sql");

pub async fn migrate(client: &Client) -> Result<(), tokio_postgres::Error> {
    client.batch_execute(MIGRATION_0001).await?;
    client.batch_execute(MIGRATION_0002).await?;
    client.batch_execute(MIGRATION_0003).await?;
    client.batch_execute(MIGRATION_0004).await?;
    client.batch_execute(MIGRATION_0005).await?;
    client.batch_execute(MIGRATION_0007).await
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

const USER_COLS: &str = "id, email, password_hash, is_active";

fn user_record(r: &Row) -> UserRecord {
    UserRecord {
        id: UserId(r.get("id")),
        email: r.get("email"),
        password_hash: r.get("password_hash"),
        is_active: r.get("is_active"),
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
        let conn = self.connector.checkout(true).await?;
        if let Err(e) = conn.client().batch_execute("BEGIN").await {
            conn.discard();
            return Err(db_err(e));
        }
        Ok(Box::new(PgTx { conn: Some(conn) }))
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
}

impl PgTx {
    fn client(&self) -> Pg<'_> {
        Pg(self.conn.as_ref().expect("transaction already finished").client())
    }
}

impl Drop for PgTx {
    fn drop(&mut self) {
        // Dropped without commit/rollback (e.g. the request future was cancelled): never return
        // a connection that is still inside a transaction to the pool.
        if let Some(c) = self.conn.take() {
            c.discard();
        }
    }
}

#[async_trait]
impl Tx for PgTx {
    /// A scope whose project is outside its workspace violates `project_sequences_tenant_fkey`
    /// (migration 0002): `Backend`. `authorize` never mints one; a test can.
    async fn allocate(&mut self, scope: &Scope) -> Result<Allocation, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let r = self
            .client()
            .query_one(
                "INSERT INTO project_sequences (workspace_id, project_id, last_sequence_id, last_sort_order) \
                 VALUES ($1, $2, 1, 10000) \
                 ON CONFLICT (workspace_id, project_id) DO UPDATE SET \
                   last_sequence_id = project_sequences.last_sequence_id + 1, \
                   last_sort_order  = project_sequences.last_sort_order + 10000 \
                 RETURNING last_sequence_id, last_sort_order",
                &[&ws.0, &project.0],
            )
            .await
            .map_err(db_err)?;
        Ok(Allocation { sequence_id: r.get(0), sort_order: r.get(1) })
    }

    async fn insert_work_item(&mut self, scope: &Scope, w: &WorkItem) -> Result<(), DbError> {
        check_tenant(scope, w.workspace_id, Some(w.project_id))?;
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
        let conn = this.conn.take().expect("transaction already finished");
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
        let conn = this.conn.take().expect("transaction already finished");
        match conn.client().batch_execute("ROLLBACK").await {
            Ok(()) => Ok(()),
            Err(e) => {
                conn.discard();
                Err(db_err(e))
            }
        }
    }
}
