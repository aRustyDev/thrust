//! The commit-atomicity control for risk R1 of spike 0002, as a library function so that the same scenario runs against
//! rusqlite (`tests/commit.rs`) and against the real `SqlStorage` inside the Durable Object (the `/_do/tx-control`
//! probe, driven by `crates/thrust-contract-tests/tests/worker_only.rs`).
//!
//! The claim: *a `Tx::commit` that fails at any statement leaves the database exactly as it was, and the same
//! transaction then commits.* The scenario fails each statement a commit issues, in turn (`FaultSql`), and compares
//! snapshots of the tables a create touches. It is relative to the state it finds, so it can run in a live database.
//!
//! It takes the `Scope` from its caller: a scope is minted only by `authorize`, never by library code.

use thrust_contracts::*;
use thrust_ports::*;

use crate::fault::{Fault, FaultSql};
use crate::sql::Exec;
use crate::SqliteDatabase;

/// The statements one commit issues, by the text that identifies them: the counter upsert, the item inserts, the soft
/// delete, the outbox inserts.
pub const TARGETS: [&str; 4] = [
    "INSERT INTO project_sequences",
    "INSERT INTO work_items",
    "UPDATE work_items SET deleted_at",
    "INSERT INTO outbox",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Snapshot {
    items: i64,
    live_items: i64,
    outbox: i64,
    counter: i64,
}

fn snapshot(sql: &dyn Exec, scope: &Scope) -> Result<Snapshot, String> {
    let n = |q: &str, p: &[crate::Value]| -> Result<i64, String> {
        sql.query(q, p).map_err(|e| e.to_string())?.first().ok_or("no row")?.int(0).map_err(|e| e.to_string())
    };
    let (ws, project) = (crate::Value::from(scope.workspace_id().0), crate::Value::from(scope.project_id().0));
    Ok(Snapshot {
        items: n("SELECT count(*) FROM work_items WHERE project_id = ?", std::slice::from_ref(&project))?,
        live_items: n("SELECT count(*) FROM work_items WHERE project_id = ? AND deleted_at IS NULL", std::slice::from_ref(&project))?,
        outbox: n("SELECT count(*) FROM outbox WHERE workspace_id = ?", std::slice::from_ref(&ws))?,
        counter: n(
            "SELECT coalesce((SELECT last_sequence_id FROM project_sequences WHERE workspace_id = ? AND project_id = ?), 0)",
            &[ws, project],
        )?,
    })
}

struct Maker<'a> {
    scope: &'a Scope,
    ids: &'a mut dyn FnMut() -> Uuid,
    now: Timestamp,
}

impl Maker<'_> {
    /// One create, as the use-case does it: allocate, insert, outbox row.
    async fn create(&mut self, tx: &mut dyn Tx) -> Result<WorkItem, DbError> {
        let a = tx.allocate(self.scope).await?;
        let item = WorkItem {
            id: WorkItemId((self.ids)()),
            workspace_id: self.scope.workspace_id(),
            project_id: self.scope.project_id(),
            sequence_id: a.sequence_id,
            sort_order: a.sort_order,
            name: format!("control {}", a.sequence_id),
            description: None,
            external_source: None,
            external_id: None,
            created_by: self.scope.user_id(),
            created_at: self.now,
            archived_at: None,
            is_draft: false,
            deleted_at: None,
        };
        tx.insert_work_item(self.scope, &item).await?;
        let row = OutboxRow {
            envelope: Envelope {
                event_id: EventId((self.ids)()),
                schema_version: 1,
                workspace_id: self.scope.workspace_id(),
                event_type: "work_item.created".into(),
                entity_type: "work_item".into(),
                entity_id: item.id.0,
                actor_id: Some(self.scope.user_id()),
                occurred_at: self.now,
                payload: serde_json::json!({ "project_id": self.scope.project_id() }),
                trace_id: None,
            },
            created_at: self.now,
            dispatched_at: None,
            attempts: 0,
        };
        tx.insert_outbox(self.scope, &row).await?;
        Ok(item)
    }
}

/// A transaction holding one soft delete of `old` and `creates` creates, not yet committed.
async fn buffered(
    db: &SqliteDatabase,
    scope: &Scope,
    maker: &mut Maker<'_>,
    old: WorkItemId,
    now: Timestamp,
    creates: usize,
) -> Result<Box<dyn Tx>, DbError> {
    let mut tx = db.begin(scope).await?;
    if !tx.soft_delete_work_item(scope, old, now).await? {
        return Err(DbError::Backend("the control's own soft delete found no live row".into()));
    }
    for _ in 0..creates {
        maker.create(tx.as_mut()).await?;
    }
    Ok(tx)
}

/// Run one case of the control: `creates` creates and one soft delete in one transaction, with the `nth` statement
/// whose text contains `target` made to fail. Returns whether the fault fired (a case can name a statement the commit
/// does not issue that many times, and then the commit must simply succeed), or what went wrong.
pub async fn commit_atomicity_case(
    db: &SqliteDatabase,
    sql: &FaultSql,
    scope: &Scope,
    ids: &mut dyn FnMut() -> Uuid,
    now: Timestamp,
    creates: usize,
    target: &str,
    nth: usize,
) -> Result<bool, String> {
    let at = format!("{creates} creates, `{target}` #{nth}");
    let db_err = |what: &str, e: DbError| format!("{at}: {what}: {e}");
    let mut maker = Maker { scope, ids, now };

    // One committed item with its outbox row: the soft delete's target, and a counter that is not at zero.
    let old = {
        let mut tx = db.begin(scope).await.map_err(|e| db_err("begin", e))?;
        let item = maker.create(tx.as_mut()).await.map_err(|e| db_err("setup create", e))?;
        tx.commit().await.map_err(|e| db_err("setup commit", e))?;
        item
    };
    let before = snapshot(sql, scope)?;

    // The transaction under test: its buffer is built with no fault armed (only `commit` writes).
    let tx = buffered(db, scope, &mut maker, old.id, now, creates).await.map_err(|e| db_err("buffering", e))?;

    sql.arm(Fault { contains: target.to_string(), nth });
    let result = tx.commit().await;
    let (matched, fired) = (sql.matched(), sql.fired());
    sql.disarm();
    let after = snapshot(sql, scope)?;
    let n = creates as i64;
    let committed = Snapshot { items: before.items + n, live_items: before.live_items + n - 1, outbox: before.outbox + n, counter: before.counter + n };

    if nth < matched {
        if fired != 1 {
            return Err(format!("{at}: the fault was armed for a statement the commit issued and did not fire"));
        }
        if result.is_ok() {
            return Err(format!("{at}: a commit whose statement failed reported success"));
        }
        if after != before {
            return Err(format!("{at}: a failed commit changed the database: before {before:?}, after {after:?}"));
        }
        // The same transaction, replayed, commits: nothing it needs was left taken.
        let tx = buffered(db, scope, &mut maker, old.id, now, creates).await.map_err(|e| db_err("buffering the retry", e))?;
        tx.commit().await.map_err(|e| db_err("the retry", e))?;
        let retried = snapshot(sql, scope)?;
        if retried != committed {
            return Err(format!("{at}: the retry did not apply the whole transaction: want {committed:?}, got {retried:?}"));
        }
        Ok(true)
    } else {
        // The fault named a statement the commit does not issue that many times: nothing fails and everything applies.
        // This is what keeps the failing cases from being "commit always errors".
        if fired != 0 {
            return Err(format!("{at}: the fault fired for a statement the commit did not issue"));
        }
        result.map_err(|e| db_err("a commit with no fault", e))?;
        if after != committed {
            return Err(format!("{at}: a commit with no fault did not apply the whole transaction: want {committed:?}, got {after:?}"));
        }
        Ok(false)
    }
}
