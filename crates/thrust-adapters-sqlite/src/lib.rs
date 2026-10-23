//! SQLite adapter for the `Database` port, written once against the synchronous [`Sql`] trait.
//!
//! The only platform-specific part is *how SQL is run*, and that is [`Sql`]:
//!   * native  -> [`NativeSql`] (feature `native`): rusqlite, the twin of the Durable Object cell
//!   * Worker  -> `thrust-adapters-cloudflare::DoSql`: workers-rs `SqlStorage` inside the instance-level Durable Object
//!
//! # How ADR-0045's explicit `Tx` maps onto SQLite (spike 0002, risk R1)
//!
//! `Database::begin()` returns a `Tx` that a use-case drives across awaited calls before it commits or drops it. Neither
//! engine can hold a SQL transaction open across those awaits: the Durable Object's `sql.exec()` rejects `BEGIN` and
//! `SAVEPOINT`, and its `transactionSync` takes a callback that must finish without yielding. So:
//!
//! * the `Tx` **buffers its writes** (`insert_work_item`, `soft_delete_work_item`, `insert_outbox`) and answers the
//!   questions the use-case asks mid-transaction (a duplicate external key, whether a row is live) from the committed
//!   rows plus its own buffer;
//! * `allocate` **reads the counter** and hands out numbers above it; nothing is written yet;
//! * `commit` applies the counter advance and every buffered write in **one [`Sql::atomically`]** (`BEGIN IMMEDIATE`
//!   natively, `transactionSync` in the object), and checks that the counter is still where `allocate` read it;
//! * `rollback` and `Drop` discard the buffer: nothing was written, so there is nothing to undo;
//! * a [`Gate`](gate::Gate) lets **one `Tx` be open at a time**, so no other transaction can move the counter between
//!   `allocate` and `commit`. The counter check turns a violation of that into an error instead of a duplicate number.
//!
//! What it gives up against Postgres: a `Tx` cannot read its own uncommitted writes through the database (only the
//! three questions above are answered from the buffer), and transactions are serialised, which is the cost spike 0002
//! measures (R2).

pub mod fault;
mod gate;
pub mod migrate;
#[cfg(feature = "native")]
mod native;
pub mod sql;

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use thrust_contracts::*;
use thrust_ports::*;

use crate::gate::{Gate, Permit};
pub use migrate::{migrate, split_statements, Migration, MIGRATIONS};
#[cfg(feature = "native")]
pub use native::NativeSql;
pub use sql::{Exec, Row, Sql, SqlError, Value};

fn db_err(e: SqlError) -> DbError {
    match e {
        SqlError::Unique(m) => DbError::Conflict(m),
        SqlError::Other(m) => DbError::Backend(m),
    }
}

/// `&[Value]` from anything `Into<Value>`.
macro_rules! params {
    ($($p:expr),* $(,)?) => { &[$(Value::from($p)),*] };
}

pub struct SqliteDatabase {
    sql: Arc<dyn Sql>,
    gate: Arc<Gate>,
}

impl SqliteDatabase {
    pub fn new(sql: Arc<dyn Sql>) -> Self {
        Self { sql, gate: Arc::default() }
    }

    /// Apply the migrations this build knows to the database. Returns the versions applied (none on a second call).
    pub fn migrate(&self) -> Result<Vec<u32>, SqlError> {
        migrate::migrate(self.sql.as_ref(), MIGRATIONS)
    }

    fn q(&self, sql: &str, p: &[Value]) -> Result<Vec<Row>, DbError> {
        self.sql.query(sql, p).map_err(db_err)
    }

    fn x(&self, sql: &str, p: &[Value]) -> Result<(), DbError> {
        self.sql.execute(sql, p).map_err(db_err)
    }
}

const WI_COLS: &str = "id, workspace_id, project_id, sequence_id, sort_order, name, description, external_source, \
    external_id, created_by, created_at, archived_at, is_draft, deleted_at";

fn work_item(r: &Row) -> Result<WorkItem, SqlError> {
    Ok(WorkItem {
        id: WorkItemId(r.uuid(0)?),
        workspace_id: WorkspaceId(r.uuid(1)?),
        project_id: ProjectId(r.uuid(2)?),
        sequence_id: r.int(3)?,
        sort_order: r.int(4)?,
        name: r.text(5)?.to_string(),
        description: r.opt_text(6)?,
        external_source: r.opt_text(7)?,
        external_id: r.opt_text(8)?,
        created_by: UserId(r.uuid(9)?),
        created_at: r.timestamp(10)?,
        archived_at: r.opt_timestamp(11)?,
        is_draft: r.bool(12)?,
        deleted_at: r.opt_timestamp(13)?,
    })
}

fn activity(r: &Row) -> Result<Activity, SqlError> {
    Ok(Activity {
        id: ActivityId(r.uuid(0)?),
        event_id: EventId(r.uuid(1)?),
        workspace_id: WorkspaceId(r.uuid(2)?),
        project_id: ProjectId(r.uuid(3)?),
        entity_id: r.uuid(4)?,
        verb: r.text(5)?.to_string(),
        actor_id: r.opt_uuid(6)?.map(UserId),
        created_at: r.timestamp(7)?,
    })
}

#[async_trait]
impl Database for SqliteDatabase {
    async fn membership(
        &self,
        slug: &WorkspaceSlug,
        project: ProjectId,
        user: UserId,
    ) -> Result<Membership, DbError> {
        // One query, as on Postgres: the slug, the caller's workspace membership (inner join), the project in that
        // workspace (inner join) and the caller's project row (left join, same workspace). No row means the caller
        // may not know the workspace or the project exists.
        let rows = self.q(
            "SELECT w.id, m.role FROM workspaces w \
             JOIN workspace_members wm ON wm.workspace_id = w.id AND wm.user_id = ? \
             JOIN projects p ON p.workspace_id = w.id AND p.id = ? \
             LEFT JOIN project_members m \
               ON m.project_id = p.id AND m.workspace_id = p.workspace_id AND m.user_id = ? \
             WHERE w.slug = ?",
            params![user.0, project.0, user.0, slug.as_str()],
        )?;
        let Some(r) = rows.first() else { return Ok(Membership::NoSuchProject) };
        let role = r.opt_text(1).map_err(db_err)?;
        Ok(match role.as_deref().and_then(Role::parse) {
            Some(role) => Membership::Member { workspace_id: WorkspaceId(r.uuid(0).map_err(db_err)?), role },
            None => Membership::NotAMember,
        })
    }

    async fn session(&self, hash: &TokenHash, now: Timestamp) -> Result<Option<LiveSession>, DbError> {
        let rows = self.q(
            "SELECT user_id, last_seen_at FROM sessions \
             WHERE token_hash = ? AND revoked_at IS NULL AND expires_at > ?",
            params![&hash.0[..], now],
        )?;
        rows.first()
            .map(|r| Ok(LiveSession { user_id: UserId(r.uuid(0)?), last_seen_at: r.timestamp(1)? }))
            .transpose()
            .map_err(db_err)
    }

    async fn touch_session(&self, hash: &TokenHash, at: Timestamp) -> Result<(), DbError> {
        self.x("UPDATE sessions SET last_seen_at = ? WHERE token_hash = ?", params![at, &hash.0[..]])
    }

    async fn revoke_session(&self, hash: &TokenHash, at: Timestamp) -> Result<(), DbError> {
        self.x(
            "UPDATE sessions SET revoked_at = ? WHERE token_hash = ? AND revoked_at IS NULL",
            params![at, &hash.0[..]],
        )
    }

    async fn insert_session(&self, s: &NewSession) -> Result<(), DbError> {
        self.x(
            "INSERT INTO sessions (token_hash, user_id, created_at, expires_at, last_seen_at) \
             VALUES (?, ?, ?, ?, ?) ON CONFLICT (token_hash) DO NOTHING",
            params![&s.token_hash.0[..], s.user_id.0, s.created_at, s.expires_at, s.created_at],
        )
    }

    async fn begin(&self, _scope: &Scope) -> Result<Box<dyn Tx>, DbError> {
        let permit = self.gate.acquire().await;
        Ok(Box::new(SqliteTx { sql: self.sql.clone(), _permit: permit, counters: BTreeMap::new(), ops: Vec::new() }))
    }

    async fn list_work_items(&self, scope: &Scope, vis: Visibility) -> Result<Vec<WorkItem>, DbError> {
        let hidden = match vis {
            Visibility::Visible => " AND deleted_at IS NULL AND archived_at IS NULL AND is_draft = 0",
            Visibility::WithDeleted => "",
        };
        let rows = self.q(
            &format!("SELECT {WI_COLS} FROM work_items WHERE workspace_id = ? AND project_id = ?{hidden} ORDER BY sequence_id"),
            params![scope.workspace_id().0, scope.project_id().0],
        )?;
        rows.iter().map(work_item).collect::<Result<_, _>>().map_err(db_err)
    }

    async fn get_work_item(
        &self,
        scope: &Scope,
        id: WorkItemId,
        vis: Visibility,
    ) -> Result<Option<WorkItem>, DbError> {
        let hidden = match vis {
            Visibility::Visible => " AND deleted_at IS NULL AND archived_at IS NULL AND is_draft = 0",
            Visibility::WithDeleted => "",
        };
        let rows = self.q(
            &format!("SELECT {WI_COLS} FROM work_items WHERE workspace_id = ? AND project_id = ? AND id = ?{hidden}"),
            params![scope.workspace_id().0, scope.project_id().0, id.0],
        )?;
        rows.first().map(work_item).transpose().map_err(db_err)
    }

    async fn insert_activity(&self, _sys: &SystemScope, a: &Activity) -> Result<bool, DbError> {
        let rows = self.q(
            "INSERT INTO activity (id, event_id, workspace_id, project_id, entity_id, verb, actor_id, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (event_id) DO NOTHING RETURNING id",
            params![
                a.id.0,
                a.event_id.0,
                a.workspace_id.0,
                a.project_id.0,
                a.entity_id,
                a.verb.as_str(),
                a.actor_id.map(|u| u.0),
                a.created_at
            ],
        )?;
        Ok(!rows.is_empty())
    }

    async fn list_activity(&self, scope: &Scope, entity: Option<WorkItemId>) -> Result<Vec<Activity>, DbError> {
        let entity = entity.map(|e| e.0);
        let rows = self.q(
            "SELECT id, event_id, workspace_id, project_id, entity_id, verb, actor_id, created_at \
             FROM activity WHERE workspace_id = ? AND project_id = ? AND (? IS NULL OR entity_id = ?) \
             ORDER BY created_at, id",
            params![scope.workspace_id().0, scope.project_id().0, entity, entity],
        )?;
        rows.iter().map(activity).collect::<Result<_, _>>().map_err(db_err)
    }

    async fn undispatched_outbox(&self, _sys: &SystemScope, limit: u32) -> Result<Vec<OutboxRow>, DbError> {
        let rows = self.q(
            "SELECT envelope, created_at, attempts FROM outbox WHERE dispatched_at IS NULL \
             ORDER BY created_at, event_id LIMIT ?",
            params![i64::from(limit)],
        )?;
        rows.iter()
            .map(|r| {
                let envelope: Envelope = serde_json::from_str(r.text(0).map_err(db_err)?)
                    .map_err(|e| DbError::Backend(format!("bad envelope: {e}")))?;
                Ok(OutboxRow {
                    envelope,
                    created_at: r.timestamp(1).map_err(db_err)?,
                    dispatched_at: None,
                    attempts: r.int(2).map_err(db_err)? as i32,
                })
            })
            .collect()
    }

    async fn mark_outbox_dispatched(&self, _sys: &SystemScope, event: EventId, at: Timestamp) -> Result<(), DbError> {
        self.x("UPDATE outbox SET dispatched_at = ?, attempts = attempts + 1 WHERE event_id = ?", params![at, event.0])
    }

    async fn outbox_depth(&self, _sys: &SystemScope) -> Result<u64, DbError> {
        let rows = self.q("SELECT count(*) FROM outbox WHERE dispatched_at IS NULL", &[])?;
        Ok(rows.first().ok_or_else(|| DbError::Backend("count returned no row".into()))?.int(0).map_err(db_err)? as u64)
    }

    async fn seed(&self, _sys: &SystemScope, f: &SeedFixture) -> Result<(), DbError> {
        // `Tx` only exposes slice operations, so the fixture goes through the SQL surface directly, in one transaction.
        self.sql
            .atomically(&mut |x| {
                for w in &f.workspaces {
                    x.execute(
                        "INSERT INTO workspaces (id, slug) VALUES (?, ?) ON CONFLICT (id) DO UPDATE SET slug = excluded.slug",
                        params![w.id.0, w.slug.as_str()],
                    )?;
                }
                for m in &f.workspace_members {
                    x.execute(
                        "INSERT INTO workspace_members (workspace_id, user_id) VALUES (?, ?) ON CONFLICT DO NOTHING",
                        params![m.workspace_id.0, m.user_id.0],
                    )?;
                }
                for p in &f.projects {
                    x.execute(
                        "INSERT INTO projects (id, workspace_id, name) VALUES (?, ?, ?) ON CONFLICT (id) DO NOTHING",
                        params![p.id.0, p.workspace_id.0, p.name.as_str()],
                    )?;
                }
                for m in &f.memberships {
                    x.execute(
                        "INSERT INTO project_members (project_id, workspace_id, user_id, role) VALUES (?, ?, ?, ?) \
                         ON CONFLICT (project_id, user_id) DO UPDATE SET role = excluded.role",
                        params![m.project_id.0, m.workspace_id.0, m.user_id.0, m.role.as_str()],
                    )?;
                }
                Ok(())
            })
            .map_err(db_err)
    }
}

// ---------------------------------------------------------------------------------------------
// The transaction
// ---------------------------------------------------------------------------------------------

/// What `allocate` read for one project, and how many numbers this transaction has taken above it.
struct Counter {
    base_sequence: i64,
    base_sort_order: i64,
    taken: i64,
}

/// A buffered write, applied in order at commit.
enum Op {
    InsertItem(WorkItem),
    SoftDelete { workspace: WorkspaceId, project: ProjectId, id: WorkItemId, at: Timestamp },
    InsertOutbox(OutboxRow),
}

pub struct SqliteTx {
    sql: Arc<dyn Sql>,
    /// Held until the transaction commits, rolls back or is dropped.
    _permit: Permit,
    counters: BTreeMap<(WorkspaceId, ProjectId), Counter>,
    ops: Vec<Op>,
}

const SORT_STEP: i64 = 10_000;

impl SqliteTx {
    fn q(&self, sql: &str, p: &[Value]) -> Result<Vec<Row>, DbError> {
        self.sql.query(sql, p).map_err(db_err)
    }

    fn deleted_in_tx(&self, id: WorkItemId) -> bool {
        self.ops.iter().any(|o| matches!(o, Op::SoftDelete { id: d, .. } if *d == id))
    }

    /// Everything the buffer will write, in one call, as the commit applies it.
    fn apply(&self, x: &dyn Exec) -> Result<(), SqlError> {
        for ((workspace, project), c) in &self.counters {
            let rows = x.query(
                "INSERT INTO project_sequences (workspace_id, project_id, last_sequence_id, last_sort_order) \
                 VALUES (?, ?, ?, ?) \
                 ON CONFLICT (workspace_id, project_id) DO UPDATE SET \
                   last_sequence_id = project_sequences.last_sequence_id + ?, \
                   last_sort_order  = project_sequences.last_sort_order + ? \
                 RETURNING last_sequence_id, last_sort_order",
                params![
                    workspace.0,
                    project.0,
                    c.taken,
                    c.taken * SORT_STEP,
                    c.taken,
                    c.taken * SORT_STEP
                ],
            )?;
            let r = rows.first().ok_or_else(|| SqlError::Other("the counter upsert returned no row".into()))?;
            // Another transaction moved the counter between `allocate` and here: the numbers this one handed out may
            // be taken. The gate prevents it; this makes sure a hole in the gate is an error, never a duplicate.
            if r.int(0)? != c.base_sequence + c.taken || r.int(1)? != c.base_sort_order + c.taken * SORT_STEP {
                return Err(SqlError::Other(format!(
                    "the sequence counter of project {project} moved between allocate and commit"
                )));
            }
        }
        for op in &self.ops {
            match op {
                Op::InsertItem(w) => x.execute(
                    &format!(
                        "INSERT INTO work_items ({WI_COLS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
                    ),
                    params![
                        w.id.0,
                        w.workspace_id.0,
                        w.project_id.0,
                        w.sequence_id,
                        w.sort_order,
                        w.name.as_str(),
                        w.description.as_deref(),
                        w.external_source.as_deref(),
                        w.external_id.as_deref(),
                        w.created_by.0,
                        w.created_at,
                        w.archived_at,
                        w.is_draft,
                        w.deleted_at
                    ],
                )?,
                Op::SoftDelete { workspace, project, id, at } => x.execute(
                    "UPDATE work_items SET deleted_at = ? \
                     WHERE workspace_id = ? AND project_id = ? AND id = ? AND deleted_at IS NULL",
                    params![*at, workspace.0, project.0, id.0],
                )?,
                Op::InsertOutbox(row) => {
                    let envelope = serde_json::to_string(&row.envelope)
                        .map_err(|e| SqlError::Other(format!("envelope does not serialise: {e}")))?;
                    x.execute(
                        "INSERT INTO outbox (event_id, workspace_id, envelope, created_at, dispatched_at, attempts) \
                         VALUES (?, ?, ?, ?, NULL, 0)",
                        params![row.envelope.event_id.0, row.envelope.workspace_id.0, envelope, row.created_at],
                    )?
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Tx for SqliteTx {
    /// A scope whose project is outside its workspace is refused (`Backend`), as the Postgres foreign key refuses it.
    async fn allocate(&mut self, scope: &Scope) -> Result<Allocation, DbError> {
        let key = (scope.workspace_id(), scope.project_id());
        if !self.counters.contains_key(&key) {
            let rows = self.q(
                "SELECT last_sequence_id, last_sort_order FROM project_sequences WHERE workspace_id = ? AND project_id = ?",
                params![key.0 .0, key.1 .0],
            )?;
            let (base_sequence, base_sort_order) = match rows.first() {
                Some(r) => (r.int(0).map_err(db_err)?, r.int(1).map_err(db_err)?),
                None => {
                    // A project's first create: there is no counter row yet, and none may start for a project that is
                    // not in this workspace.
                    let project = self.q(
                        "SELECT 1 FROM projects WHERE id = ? AND workspace_id = ?",
                        params![key.1 .0, key.0 .0],
                    )?;
                    if project.is_empty() {
                        return Err(DbError::Backend("project is not in this workspace".into()));
                    }
                    (0, 0)
                }
            };
            self.counters.insert(key, Counter { base_sequence, base_sort_order, taken: 0 });
        }
        let c = self.counters.get_mut(&key).expect("inserted above");
        c.taken += 1;
        // MUTANT (control run only, feature `mutant_dup_sequence`): hand out the same sequence number every time.
        #[cfg(feature = "mutant_dup_sequence")]
        return Ok(Allocation { sequence_id: 1, sort_order: SORT_STEP });
        #[cfg(not(feature = "mutant_dup_sequence"))]
        Ok(Allocation { sequence_id: c.base_sequence + c.taken, sort_order: c.base_sort_order + c.taken * SORT_STEP })
    }

    async fn insert_work_item(&mut self, scope: &Scope, w: &WorkItem) -> Result<(), DbError> {
        check_tenant(scope, w.workspace_id, Some(w.project_id))?;
        if let (Some(source), Some(external)) = (&w.external_source, &w.external_id) {
            // The partial unique index decides at commit; asking now is what lets the use-case see the conflict at
            // the call, as it does on Postgres. A row this transaction already soft-deleted no longer counts.
            let committed = self.q(
                "SELECT id FROM work_items WHERE project_id = ? AND external_source = ? AND external_id = ? \
                 AND deleted_at IS NULL",
                params![w.project_id.0, source.as_str(), external.as_str()],
            )?;
            let live_committed = committed.iter().any(|r| match r.uuid(0) {
                Ok(id) => !self.deleted_in_tx(WorkItemId(id)),
                Err(_) => true,
            });
            let live_buffered = self.ops.iter().any(|o| match o {
                Op::InsertItem(b) => {
                    b.project_id == w.project_id
                        && b.external_source.as_ref() == Some(source)
                        && b.external_id.as_ref() == Some(external)
                        && b.deleted_at.is_none()
                        && !self.deleted_in_tx(b.id)
                }
                _ => false,
            });
            if live_committed || live_buffered {
                return Err(DbError::Conflict("work item external_id already exists".into()));
            }
        }
        self.ops.push(Op::InsertItem(w.clone()));
        Ok(())
    }

    async fn soft_delete_work_item(&mut self, scope: &Scope, id: WorkItemId, at: Timestamp) -> Result<bool, DbError> {
        let (workspace, project) = (scope.workspace_id(), scope.project_id());
        if self.deleted_in_tx(id) {
            return Ok(false);
        }
        let buffered = self.ops.iter().any(|o| match o {
            Op::InsertItem(b) => b.id == id && b.workspace_id == workspace && b.project_id == project && b.deleted_at.is_none(),
            _ => false,
        });
        let live = buffered
            || !self
                .q(
                    "SELECT 1 FROM work_items WHERE workspace_id = ? AND project_id = ? AND id = ? AND deleted_at IS NULL",
                    params![workspace.0, project.0, id.0],
                )?
                .is_empty();
        if live {
            self.ops.push(Op::SoftDelete { workspace, project, id, at });
        }
        Ok(live)
    }

    async fn insert_outbox(&mut self, scope: &Scope, row: &OutboxRow) -> Result<(), DbError> {
        check_tenant(scope, row.envelope.workspace_id, row.envelope.project_id())?;
        self.ops.push(Op::InsertOutbox(row.clone()));
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<(), DbError> {
        // `self` (and the gate permit in it) lives until the transaction has been applied.
        let this = *self;
        // MUTANT (control run only, feature `mutant_non_atomic_commit`): apply the buffer statement by statement, each
        // in its own transaction. A failure halfway then leaves the work item without its outbox row, and
        // `tests/commit.rs` must catch it.
        #[cfg(feature = "mutant_non_atomic_commit")]
        return this.apply(this.sql.as_ref()).map_err(db_err);
        #[cfg(not(feature = "mutant_non_atomic_commit"))]
        this.sql.atomically(&mut |x| this.apply(x)).map_err(db_err)
    }

    async fn rollback(self: Box<Self>) -> Result<(), DbError> {
        // Nothing was written, so there is nothing to undo; dropping the buffer and the permit is the rollback.
        drop(self);
        Ok(())
    }
}
