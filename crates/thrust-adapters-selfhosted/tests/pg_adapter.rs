//! Adapter-level tests for the shared Postgres adapter, native side, against a REAL Postgres
//! started by testcontainers (OrbStack docker). These pin the properties the core relies on:
//! atomic sequence allocation, rollback atomicity, partial-unique semantics, outbox, tenancy.
//!
//! The tests below each pin one behaviour; `model` generates them instead: random sequences of `Database` and `Tx`
//! calls against a reference model. It is compiled into this binary, so it needs Docker too and stays out of the
//! hermetic default loop.

mod model;

use std::sync::Arc;

use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;
use thrust_adapters_postgres::PgDatabase;
use thrust_adapters_selfhosted::pg::PoolConnector;
use thrust_contracts::chrono::{TimeZone, Utc};
use thrust_contracts::*;
use thrust_ports::*;

struct Env {
    db: Arc<PgDatabase>,
    _container: ContainerAsync<Postgres>,
}

async fn env() -> Env {
    let container = Postgres::default().start().await.expect("start postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
    // testcontainers' default credentials are a throwaway for this container only
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let conn = Arc::new(PoolConnector::from_url(&url, 16).unwrap());
    let raw = conn.raw().await.unwrap();
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    drop(raw);
    Env { db: Arc::new(PgDatabase::new(conn)), _container: container }
}

fn new_id() -> Uuid {
    // deterministic-enough unique ids without a rng dependency: v7-ish from nanos + counter
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64;
    Uuid::from_u64_pair(t, n)
}

fn item(ws: WorkspaceId, p: ProjectId, a: Allocation, name: &str, ext: Option<&str>) -> WorkItem {
    WorkItem {
        id: WorkItemId(new_id()),
        workspace_id: ws,
        project_id: p,
        sequence_id: a.sequence_id,
        sort_order: a.sort_order,
        name: name.into(),
        description: None,
        external_source: ext.map(|_| "gh".to_string()),
        external_id: ext.map(str::to_string),
        created_by: UserId(new_id()),
        created_at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    }
}

async fn seeded(env: &Env) -> (WorkspaceId, WorkspaceId, ProjectId, UserId) {
    let ws = WorkspaceId(new_id());
    let other_ws = WorkspaceId(new_id());
    let p = ProjectId(new_id());
    let u = UserId(new_id());
    env.db
        .seed(&SeedFixture {
            workspaces: vec![ws],
            users: vec![u],
            projects: vec![SeedProject { id: p, workspace_id: ws, name: "p".into() }],
            memberships: vec![SeedMembership { project_id: p, workspace_id: ws, user_id: u, role: Role::Member }],
            sessions: vec![],
        })
        .await
        .unwrap();
    (ws, other_ws, p, u)
}

#[tokio::test]
async fn membership_distinguishes_member_non_member_and_other_tenant() {
    let env = env().await;
    let (ws, other_ws, p, u) = seeded(&env).await;
    assert_eq!(env.db.membership(ws, p, u).await.unwrap(), Membership::Member(Role::Member));
    assert_eq!(env.db.membership(ws, p, UserId(new_id())).await.unwrap(), Membership::NotAMember);
    // control: the SAME project asked through another tenant does not exist
    assert_eq!(env.db.membership(other_ws, p, u).await.unwrap(), Membership::NoSuchProject);
}

#[tokio::test]
async fn concurrent_allocation_is_unique_contiguous_and_serialised() {
    let env = env().await;
    let (ws, _o, p, _u) = seeded(&env).await;
    let mut handles = Vec::new();
    for i in 0..24 {
        let db = env.db.clone();
        handles.push(tokio::spawn(async move {
            let mut tx = db.begin().await.unwrap();
            let a = tx.allocate(ws, p).await.unwrap();
            tx.insert_work_item(&item(ws, p, a, &format!("w{i}"), None)).await.unwrap();
            tx.commit().await.unwrap();
            a
        }));
    }
    let mut seqs = Vec::new();
    for h in handles {
        seqs.push(h.await.unwrap().sequence_id);
    }
    seqs.sort();
    assert_eq!(seqs, (1..=24).collect::<Vec<_>>());
}

#[tokio::test]
async fn rollback_is_atomic_and_unique_violation_is_a_conflict() {
    let env = env().await;
    let (ws, _o, p, _u) = seeded(&env).await;

    let mut tx = env.db.begin().await.unwrap();
    let a = tx.allocate(ws, p).await.unwrap();
    tx.insert_work_item(&item(ws, p, a, "keep", Some("1"))).await.unwrap();
    tx.commit().await.unwrap();

    // duplicate live external key -> Conflict, and the failed tx rolls back its allocation too
    let mut tx = env.db.begin().await.unwrap();
    let a2 = tx.allocate(ws, p).await.unwrap();
    assert_eq!(a2.sequence_id, 2);
    let err = tx.insert_work_item(&item(ws, p, a2, "dup", Some("1"))).await.unwrap_err();
    assert!(matches!(err, DbError::Conflict(_)), "got {err:?}");
    tx.rollback().await.unwrap();

    // control: nothing leaked, and the counter went back (Postgres allocation is transactional)
    let rows = env.db.list_work_items(ws, p, Visibility::WithDeleted).await.unwrap();
    assert_eq!(rows.len(), 1);
    let mut tx = env.db.begin().await.unwrap();
    assert_eq!(tx.allocate(ws, p).await.unwrap().sequence_id, 2, "rolled-back allocation must not leave a gap");
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn an_abandoned_transaction_does_not_poison_the_pool() {
    let env = env().await;
    let (ws, _o, p, _u) = seeded(&env).await;
    // bounded: a pool poisoned by recycled in-transaction connections would otherwise hang the test
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        for _ in 0..40 {
            // more iterations than pool slots: if abandoned connections were recycled mid-transaction,
            // later iterations would see a stale open transaction / held row lock and hang or fail.
            let mut tx = env.db.begin().await.unwrap();
            tx.allocate(ws, p).await.unwrap();
            drop(tx); // never committed
        }
        let mut tx = env.db.begin().await.unwrap();
        let a = tx.allocate(ws, p).await.unwrap();
        assert_eq!(a.sequence_id, 1, "abandoned transactions must roll back");
        tx.commit().await.unwrap();
    })
    .await
    .expect("abandoned transactions wedged the pool");
}

#[tokio::test]
async fn soft_delete_frees_the_external_key_but_not_the_sequence_number() {
    let env = env().await;
    let (ws, _o, p, _u) = seeded(&env).await;
    let mut tx = env.db.begin().await.unwrap();
    let a = tx.allocate(ws, p).await.unwrap();
    let w = item(ws, p, a, "x", Some("9"));
    tx.insert_work_item(&w).await.unwrap();
    assert!(tx.soft_delete_work_item(ws, p, w.id, Utc::now_for_test()).await.unwrap());
    // soft delete is idempotent-false the second time
    assert!(!tx.soft_delete_work_item(ws, p, w.id, Utc::now_for_test()).await.unwrap());
    tx.commit().await.unwrap();

    let mut tx = env.db.begin().await.unwrap();
    let a = tx.allocate(ws, p).await.unwrap();
    assert_eq!(a.sequence_id, 2);
    tx.insert_work_item(&item(ws, p, a, "x again", Some("9"))).await.unwrap();
    tx.commit().await.unwrap();

    let visible = env.db.list_work_items(ws, p, Visibility::Visible).await.unwrap();
    assert_eq!(visible.len(), 1);
    let all = env.db.list_work_items(ws, p, Visibility::WithDeleted).await.unwrap();
    assert_eq!(all.len(), 2);
}

trait NowForTest {
    fn now_for_test() -> Timestamp;
}
impl NowForTest for Utc {
    fn now_for_test() -> Timestamp {
        Utc.timestamp_opt(1_800_000_100, 0).unwrap()
    }
}

#[tokio::test]
async fn outbox_is_atomic_with_the_entity_and_dispatch_is_recorded() {
    let env = env().await;
    let (ws, _o, p, u) = seeded(&env).await;
    let mk = |wi: WorkItemId| OutboxRow {
        envelope: Envelope {
            event_id: EventId(new_id()),
            schema_version: 1,
            workspace_id: ws,
            event_type: "work_item.created".into(),
            entity_type: "work_item".into(),
            entity_id: wi.0,
            actor_id: Some(u),
            occurred_at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
            payload: serde_json::json!({ "project_id": p }),
            trace_id: None,
        },
        created_at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
        dispatched_at: None,
        attempts: 0,
    };

    // rolled back -> no outbox row
    let mut tx = env.db.begin().await.unwrap();
    let a = tx.allocate(ws, p).await.unwrap();
    let w = item(ws, p, a, "gone", None);
    tx.insert_work_item(&w).await.unwrap();
    tx.insert_outbox(&mk(w.id)).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(env.db.outbox_depth().await.unwrap(), 0);

    // committed -> exactly one undispatched row, then marked dispatched
    let mut tx = env.db.begin().await.unwrap();
    let a = tx.allocate(ws, p).await.unwrap();
    let w = item(ws, p, a, "kept", None);
    tx.insert_work_item(&w).await.unwrap();
    let row = mk(w.id);
    tx.insert_outbox(&row).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(env.db.outbox_depth().await.unwrap(), 1);
    let rows = env.db.undispatched_outbox(10).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].envelope, row.envelope);
    env.db.mark_outbox_dispatched(row.envelope.event_id, Utc.timestamp_opt(1_800_000_001, 0).unwrap()).await.unwrap();
    assert_eq!(env.db.outbox_depth().await.unwrap(), 0);

    // activity projection is idempotent on event_id
    let act = Activity {
        id: ActivityId(new_id()),
        event_id: row.envelope.event_id,
        workspace_id: ws,
        project_id: p,
        entity_id: w.id.0,
        verb: "created".into(),
        actor_id: Some(u),
        created_at: Utc.timestamp_opt(1_800_000_002, 0).unwrap(),
    };
    assert!(env.db.insert_activity(&act).await.unwrap());
    let mut again = act.clone();
    again.id = ActivityId(new_id());
    assert!(!env.db.insert_activity(&again).await.unwrap(), "redelivery must not insert");
    assert_eq!(env.db.list_activity(ws, p, Some(w.id)).await.unwrap().len(), 1);
}
