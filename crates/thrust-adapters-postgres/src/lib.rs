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

pub async fn migrate(client: &Client) -> Result<(), tokio_postgres::Error> {
    client.batch_execute(MIGRATION_0001).await?;
    client.batch_execute(MIGRATION_0002).await
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
    bool => Type::BOOL,
    String => Type::TEXT,
    Timestamp => Type::TIMESTAMPTZ,
    serde_json::Value => Type::JSONB,
);
impl PgType for &str {
    const TY: Type = Type::TEXT;
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
        ws: WorkspaceId,
        project: ProjectId,
        user: UserId,
    ) -> Result<Membership, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client())
            .query_opt(
                "SELECT m.role AS role FROM projects p \
                 LEFT JOIN project_members m \
                   ON m.project_id = p.id AND m.workspace_id = p.workspace_id AND m.user_id = $3 \
                 WHERE p.id = $1 AND p.workspace_id = $2",
                &[&project.0, &ws.0, &user.0],
            )
            .await
            .map_err(db_err)?;
        Ok(match row {
            None => Membership::NoSuchProject,
            Some(r) => match r.get::<_, Option<String>>("role").as_deref().and_then(Role::parse) {
                Some(role) => Membership::Member(role),
                None => Membership::NotAMember,
            },
        })
    }

    async fn begin(&self) -> Result<Box<dyn Tx>, DbError> {
        let conn = self.connector.checkout(true).await?;
        if let Err(e) = conn.client().batch_execute("BEGIN").await {
            conn.discard();
            return Err(db_err(e));
        }
        Ok(Box::new(PgTx { conn: Some(conn) }))
    }

    async fn list_work_items(
        &self,
        ws: WorkspaceId,
        project: ProjectId,
        vis: Visibility,
    ) -> Result<Vec<WorkItem>, DbError> {
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
        ws: WorkspaceId,
        project: ProjectId,
        id: WorkItemId,
        vis: Visibility,
    ) -> Result<Option<WorkItem>, DbError> {
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

    async fn insert_activity(&self, a: &Activity) -> Result<bool, DbError> {
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

    async fn list_activity(
        &self,
        ws: WorkspaceId,
        project: ProjectId,
        entity: Option<WorkItemId>,
    ) -> Result<Vec<Activity>, DbError> {
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

    async fn undispatched_outbox(&self, limit: u32) -> Result<Vec<OutboxRow>, DbError> {
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

    async fn mark_outbox_dispatched(&self, event: EventId, at: Timestamp) -> Result<(), DbError> {
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

    async fn outbox_depth(&self) -> Result<u64, DbError> {
        let c = self.conn().await?;
        let r = Pg(c.client())
            .query_one("SELECT count(*) AS n FROM outbox WHERE dispatched_at IS NULL", &[])
            .await
            .map_err(db_err)?;
        Ok(r.get::<_, i64>("n") as u64)
    }

    async fn seed(&self, f: &SeedFixture) -> Result<(), DbError> {
        // `Tx` only exposes slice operations, so the fixture goes through a connection directly.
        let c = self.connector.checkout(true).await?;
        let cl = Pg(c.client());
        cl.batch_execute("BEGIN").await.map_err(db_err)?;
        let res: Result<(), DbError> = async {
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
    /// A project outside `ws` violates `project_sequences_tenant_fkey` (migration 0002): `Backend`.
    async fn allocate(&mut self, ws: WorkspaceId, project: ProjectId) -> Result<Allocation, DbError> {
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

    async fn insert_work_item(&mut self, w: &WorkItem) -> Result<(), DbError> {
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

    async fn soft_delete_work_item(
        &mut self,
        ws: WorkspaceId,
        project: ProjectId,
        id: WorkItemId,
        at: Timestamp,
    ) -> Result<bool, DbError> {
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

    async fn insert_outbox(&mut self, row: &OutboxRow) -> Result<(), DbError> {
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
