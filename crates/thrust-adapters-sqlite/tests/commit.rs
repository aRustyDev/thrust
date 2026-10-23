//! The control for risk R1 of spike 0002: how ADR-0045's explicit `Tx` maps onto SQLite.
//!
//! The `Tx` buffers its writes and `commit` applies them in one `Sql::atomically`. The claim to test is the one the
//! create path depends on (ADR-0046): *a commit that fails anywhere leaves neither the work item nor its outbox row
//! nor the counter advance.* The test makes it fail anywhere: every statement a commit issues, in turn, is made to
//! fail (`FaultSql`), over every number of creates in the transaction, and each time the database must be exactly as it
//! was, and the same transaction must then commit.
//!
//! Rule 8: a pass proves nothing unless the check can fail. `scripts/mutants.sh` builds the adapter with
//! `mutant_non_atomic_commit` (every statement its own transaction) and requires this file to fail on it.

use std::sync::Arc;

use futures_executor::block_on;
use rstest::rstest;
use thrust_adapters_sqlite::fault::{Fault, FaultSql};
use thrust_adapters_sqlite::{Exec, NativeSql, SqliteDatabase};
use thrust_contracts::chrono::{TimeZone, Utc};
use thrust_contracts::*;
use thrust_ports::*;

struct Rig {
    db: SqliteDatabase,
    sql: Arc<FaultSql>,
    ws: WorkspaceId,
    project: ProjectId,
    user: UserId,
    ids: std::cell::Cell<u128>,
}

impl Rig {
    fn id(&self) -> Uuid {
        let n = self.ids.get() + 1;
        self.ids.set(n);
        Uuid::from_u128(n)
    }

    fn scope(&self) -> Scope {
        Scope::mint(self.ws, self.project, self.user)
    }

    fn item(&self, a: Allocation) -> WorkItem {
        WorkItem {
            id: WorkItemId(self.id()),
            workspace_id: self.ws,
            project_id: self.project,
            sequence_id: a.sequence_id,
            sort_order: a.sort_order,
            name: format!("item {}", a.sequence_id),
            description: None,
            external_source: None,
            external_id: None,
            created_by: self.user,
            created_at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
            archived_at: None,
            is_draft: false,
            deleted_at: None,
        }
    }

    fn outbox(&self, item: &WorkItem) -> OutboxRow {
        OutboxRow {
            envelope: Envelope {
                event_id: EventId(self.id()),
                schema_version: 1,
                workspace_id: self.ws,
                event_type: "work_item.created".into(),
                entity_type: "work_item".into(),
                entity_id: item.id.0,
                actor_id: Some(self.user),
                occurred_at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
                payload: serde_json::json!({ "project_id": self.project }),
                trace_id: None,
            },
            created_at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
            dispatched_at: None,
            attempts: 0,
        }
    }

    /// One create, as the use-case does it: allocate, insert, outbox row.
    async fn create(&self, tx: &mut dyn Tx) -> Result<(WorkItem, OutboxRow), DbError> {
        let a = tx.allocate(&self.scope()).await?;
        let item = self.item(a);
        tx.insert_work_item(&self.scope(), &item).await?;
        let row = self.outbox(&item);
        tx.insert_outbox(&self.scope(), &row).await?;
        Ok((item, row))
    }

    fn count(&self, table: &str) -> i64 {
        self.sql.query(&format!("SELECT count(*) FROM {table}"), &[]).unwrap()[0].int(0).unwrap()
    }
}

fn rig() -> Rig {
    let sql = Arc::new(FaultSql::new(Arc::new(NativeSql::memory().unwrap())));
    let db = SqliteDatabase::new(sql.clone());
    db.migrate().unwrap();
    let (ws, project, user) = (WorkspaceId(Uuid::from_u128(1 << 100)), ProjectId(Uuid::from_u128(2 << 100)), UserId(Uuid::from_u128(3 << 100)));
    block_on(db.seed(
        &SystemScope::mint(),
        &SeedFixture {
            workspaces: vec![SeedWorkspace { id: ws, slug: WorkspaceSlug::new("w") }],
            users: vec![user],
            workspace_members: vec![SeedWorkspaceMember { workspace_id: ws, user_id: user }],
            projects: vec![SeedProject { id: project, workspace_id: ws, name: "p".into() }],
            memberships: vec![SeedMembership { project_id: project, workspace_id: ws, user_id: user, role: Role::Member }],
            sessions: vec![],
        },
    ))
    .unwrap();
    Rig { db, sql, ws, project, user, ids: std::cell::Cell::new(1000) }
}

/// Every statement a commit issues, failed in turn. `creates` creates and one soft delete share the transaction, so a
/// commit issues: one counter upsert, `creates` item inserts, one delete, `creates` outbox inserts.
#[rstest]
fn prop_a_commit_that_fails_anywhere_leaves_nothing(
    #[values(1usize, 2, 3)] creates: usize,
    #[values(
        "INSERT INTO project_sequences",
        "INSERT INTO work_items",
        "UPDATE work_items SET deleted_at",
        "INSERT INTO outbox"
    )]
    target: &str,
    #[values(0usize, 1, 2)] nth: usize,
) {
    let rig = rig();
    // One committed item and its outbox row: the soft delete's target, and a counter that is already at 1.
    let (old, _) = block_on(async {
        let mut tx = rig.db.begin(&rig.scope()).await.unwrap();
        let made = rig.create(tx.as_mut()).await.unwrap();
        tx.commit().await.unwrap();
        made
    });
    assert_eq!((rig.count("work_items"), rig.count("outbox")), (1, 1));

    // The transaction under test. Its buffer is built before the fault is armed: only `commit` writes.
    let run = |rig: &Rig| {
        block_on(async {
            let mut tx = rig.db.begin(&rig.scope()).await.unwrap();
            assert!(tx.soft_delete_work_item(&rig.scope(), old.id, Utc.timestamp_opt(1_800_000_100, 0).unwrap()).await.unwrap());
            let mut made = Vec::new();
            for _ in 0..creates {
                made.push(rig.create(tx.as_mut()).await.unwrap());
            }
            (tx.commit().await, made)
        })
    };

    rig.sql.arm(Fault { contains: target.to_string(), nth });
    let (result, made) = run(&rig);
    let matched = rig.sql.matched();
    let fired = rig.sql.fired();
    rig.sql.disarm();

    if nth < matched {
        // The fault hit a statement of the commit: it must have failed, and nothing of it may remain.
        assert_eq!(fired, 1, "the fault was armed for statement {nth} of `{target}` and did not fire");
        assert!(result.is_err(), "a commit whose `{target}` #{nth} failed reported success");
        assert_eq!(rig.count("work_items"), 1, "a failed commit left a work item behind ({target} #{nth})");
        assert_eq!(rig.count("outbox"), 1, "a failed commit left an outbox row behind ({target} #{nth})");
        let live = block_on(rig.db.list_work_items(&rig.scope(), Visibility::Visible)).unwrap();
        assert_eq!(live.iter().map(|w| w.id).collect::<Vec<_>>(), [old.id], "the soft delete of a failed commit survived");
        // The counter did not advance either: the next allocation is the one this transaction was given.
        let next = block_on(async {
            let mut tx = rig.db.begin(&rig.scope()).await.unwrap();
            tx.allocate(&rig.scope()).await.unwrap()
        });
        assert_eq!(next.sequence_id, 2, "a failed commit advanced the counter ({target} #{nth})");
        // And the same transaction, replayed, commits (nothing it needs is taken).
        let (retry, again) = run(&rig);
        retry.unwrap();
        assert_eq!(again.iter().map(|(w, _)| w.sequence_id).collect::<Vec<_>>(), (2..2 + creates as i64).collect::<Vec<_>>());
        assert_eq!(rig.count("work_items"), 1 + creates as i64);
        let _ = made;
    } else {
        // Control: the fault named a statement the commit does not issue that many times, so nothing failed and the
        // commit applied everything. The failing cases above are not "commit always errors".
        assert_eq!(fired, 0);
        result.unwrap();
        assert_eq!(rig.count("work_items"), 1 + creates as i64);
        assert_eq!(rig.count("outbox"), 1 + creates as i64);
        let live = block_on(rig.db.list_work_items(&rig.scope(), Visibility::Visible)).unwrap();
        assert_eq!(live.len(), creates, "the soft delete did not apply");
        assert_eq!(made.len(), creates);
    }
}
