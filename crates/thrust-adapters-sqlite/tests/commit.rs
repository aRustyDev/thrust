//! The control for risk R1 of spike 0002: how ADR-0045's explicit `Tx` maps onto SQLite.
//!
//! The `Tx` buffers its writes and `commit` applies them in one `Sql::atomically`. The claim to test is the one the
//! create path depends on (ADR-0046): *a commit that fails anywhere leaves neither the work item nor its outbox row
//! nor the counter advance.* `thrust_adapters_sqlite::control` makes it fail anywhere: every statement a commit
//! issues, in turn, over every number of creates in the transaction, and each time the database must be exactly as it
//! was and the same transaction must then commit. The same scenario runs against the real `SqlStorage` inside the
//! Durable Object (`worker_only.rs`, pass `tx` of `contract.sh worker-do`).
//!
//! Rule 8: a pass proves nothing unless the check can fail. `scripts/mutants.sh` builds the adapter with
//! `mutant_non_atomic_commit` (every statement its own transaction) and requires this file to fail on it, and
//! `prop_the_control_catches_a_commit_that_is_no_transaction` runs the scenario over `Unatomic`, the naive mapping.

use std::sync::Arc;

use futures_executor::block_on;
use rstest::rstest;
use thrust_adapters_sqlite::control::{commit_atomicity_case, TARGETS};
use thrust_adapters_sqlite::fault::{FaultSql, Unatomic};
use thrust_adapters_sqlite::{NativeSql, Sql, SqliteDatabase};
use thrust_contracts::chrono::{TimeZone, Utc};
use thrust_contracts::*;
use thrust_ports::*;

/// A database with one workspace, project and member, and the scope of that member (tests may mint one).
fn rig(inner: Arc<dyn Sql>) -> (SqliteDatabase, Arc<FaultSql>, Scope) {
    let sql = Arc::new(FaultSql::new(inner));
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
            ..SeedFixture::default()
        },
    ))
    .unwrap();
    (db, sql, Scope::mint(ws, project, user))
}

fn run(
    inner: Arc<dyn Sql>,
    creates: usize,
    target: &str,
    nth: usize,
) -> Result<bool, String> {
    let (db, sql, scope) = rig(inner);
    let mut n = 1000u128;
    let mut ids = move || {
        n += 1;
        Uuid::from_u128(n)
    };
    block_on(commit_atomicity_case(&db, &sql, &scope, &mut ids, Utc.timestamp_opt(1_800_000_000, 0).unwrap(), creates, target, nth))
}

/// Every statement a commit issues, failed in turn: one counter upsert, `creates` item inserts, one delete and
/// `creates` outbox inserts, with `creates` in 1..=3.
#[rstest]
fn prop_a_commit_that_fails_anywhere_leaves_nothing(
    #[values(1usize, 2, 3)] creates: usize,
    #[values(0usize, 1, 2)] nth: usize,
    #[values(0usize, 1, 2, 3)] target: usize,
) {
    let target = TARGETS[target];
    let fired = run(Arc::new(NativeSql::memory().unwrap()), creates, target, nth).unwrap_or_else(|e| panic!("{e}"));
    // the case fires exactly when the commit issues that many such statements (the counter upsert and the delete: one)
    let issued = if target == TARGETS[0] || target == TARGETS[2] { 1 } else { creates };
    assert_eq!(fired, nth < issued, "{creates} creates, `{target}` #{nth}: fired = {fired}, but the commit issues {issued} of them");
}

/// Rule 8 for the test above, on the real mapping's naive twin: the same scenario over a `Sql` whose `atomically` is no
/// transaction must report violations. If it did not, the scenario above could not tell a transaction from a script.
#[rstest]
fn prop_the_control_catches_a_commit_that_is_no_transaction(
    #[values(1usize, 2, 3)] creates: usize,
    #[values("INSERT INTO work_items", "INSERT INTO outbox")] target: &str,
) {
    // fail the LAST statement of its kind, so earlier statements of the commit have already been applied
    let nth = creates - 1;
    let r = run(Arc::new(Unatomic(Arc::new(NativeSql::memory().unwrap()))), creates, target, nth);
    let e = r.expect_err("a commit with no transaction passed the atomicity control");
    assert!(e.contains("changed the database"), "it failed, but not for the reason under test: {e}");
}
