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
    conn: Arc<PoolConnector>,
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
    Env { db: Arc::new(PgDatabase::new(conn.clone())), conn, _container: container }
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

/// The proof a call passes. The adapter reads only the tenant key from it, so the user is a placeholder.
fn scope(ws: WorkspaceId, p: ProjectId) -> Scope {
    Scope::mint(ws, p, UserId(Uuid::nil()))
}

/// Slugs are unique in the database and one container serves every test, so each workspace's follows its id.
fn slug_of(ws: WorkspaceId) -> WorkspaceSlug {
    WorkspaceSlug::new(format!("ws-{}", ws.0.simple()))
}

async fn seeded(env: &Env) -> (WorkspaceId, WorkspaceId, ProjectId, UserId) {
    let ws = WorkspaceId(new_id());
    let other_ws = WorkspaceId(new_id());
    let p = ProjectId(new_id());
    let u = UserId(new_id());
    env.db
        .seed(&SystemScope::mint(), &SeedFixture {
            workspaces: vec![SeedWorkspace { id: ws, slug: slug_of(ws) }],
            users: vec![u],
            workspace_members: vec![SeedWorkspaceMember { workspace_id: ws, user_id: u }],
            projects: vec![SeedProject { id: p, workspace_id: ws, name: "p".into() }],
            memberships: vec![SeedMembership { project_id: p, workspace_id: ws, user_id: u, role: Role::Member }],
            sessions: vec![],
            ..SeedFixture::default()
        })
        .await
        .unwrap();
    (ws, other_ws, p, u)
}

#[tokio::test]
async fn membership_distinguishes_member_non_member_and_other_tenant() {
    let env = env().await;
    let (ws, other_ws, p, u) = seeded(&env).await;
    let slug = slug_of(ws);
    assert_eq!(
        env.db.membership(&slug, p, u).await.unwrap(),
        Membership::Member { workspace_id: ws, role: Role::Member }
    );
    // a member of the workspace with no row for the project is a non-member
    // and a second, real workspace that `u` also belongs to, in which the project does not exist
    let v = UserId(new_id());
    env.db
        .seed(&SystemScope::mint(), &SeedFixture {
            workspaces: vec![SeedWorkspace { id: other_ws, slug: slug_of(other_ws) }],
            workspace_members: vec![
                SeedWorkspaceMember { workspace_id: ws, user_id: v },
                SeedWorkspaceMember { workspace_id: other_ws, user_id: u },
            ],
            ..SeedFixture::default()
        })
        .await
        .unwrap();
    assert_eq!(env.db.membership(&slug, p, v).await.unwrap(), Membership::NotAMember);
    // a stranger to the workspace does not learn that the project exists
    assert_eq!(env.db.membership(&slug, p, UserId(new_id())).await.unwrap(), Membership::NoSuchProject);
    // control: the SAME project asked through another tenant's slug, or one that names nothing, does not exist
    assert_eq!(env.db.membership(&slug_of(other_ws), p, u).await.unwrap(), Membership::NoSuchProject);
    assert_eq!(env.db.membership(&WorkspaceSlug::new("nope"), p, u).await.unwrap(), Membership::NoSuchProject);
}

/// A database created before issue #36 has projects and member rows but no workspaces. `migrate` backfills them (slug
/// = the id as text, a workspace member for every member row) before it puts the foreign keys on, and applying it a
/// second time changes nothing (`migrate` runs every file on every start). A pinned regression for the upgrade path
/// of a stage-2 database; every other test here starts from an empty one.
#[tokio::test]
async fn migration_0003_backfills_a_database_created_before_it() {
    let container = Postgres::default().start().await.expect("start postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let conn = Arc::new(PoolConnector::from_url(&url, 4).unwrap());
    let raw = conn.raw().await.unwrap();
    raw.batch_execute(thrust_adapters_postgres::MIGRATION_0001).await.unwrap();
    raw.batch_execute(thrust_adapters_postgres::MIGRATION_0002).await.unwrap();
    let (ws, p, u) = (WorkspaceId(new_id()), ProjectId(new_id()), UserId(new_id()));
    raw.execute("INSERT INTO projects (id, workspace_id, name) VALUES ($1, $2, 'legacy')", &[&p.0, &ws.0])
        .await
        .unwrap();
    raw.execute(
        "INSERT INTO project_members (project_id, workspace_id, user_id, role) VALUES ($1, $2, $3, 'member')",
        &[&p.0, &ws.0, &u.0],
    )
    .await
    .unwrap();
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    drop(raw);
    let db = PgDatabase::new(conn);
    let by_id = WorkspaceSlug::new(ws.0.to_string());
    assert_eq!(
        db.membership(&by_id, p, u).await.unwrap(),
        Membership::Member { workspace_id: ws, role: Role::Member },
        "the backfilled slug is the workspace id"
    );
    // control: the backfill made the one workspace and one member the rows imply, not a looser grant
    assert_eq!(db.membership(&by_id, p, UserId(new_id())).await.unwrap(), Membership::NoSuchProject);
    assert_eq!(db.membership(&WorkspaceSlug::new("other"), p, u).await.unwrap(), Membership::NoSuchProject);
}

/// ADR-0052 (Tests): sessions are stored hashed, with no plaintext column, and a session belongs to a user, not a
/// workspace (ADR-0041 `scope: directory`). The table's columns are written out, so a `token`, `secret` or
/// `workspace_id` column added later fails here and has to be argued for; and nothing but a 32-byte digest can be
/// stored as the key.
#[tokio::test]
async fn the_sessions_table_has_a_hash_and_no_plaintext_and_no_workspace() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let rows = raw
        .query(
            "SELECT column_name, data_type FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = 'sessions' ORDER BY column_name",
            &[],
        )
        .await
        .unwrap();
    let got: Vec<(String, String)> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();
    let want: Vec<(String, String)> = [
        ("created_at", "timestamp with time zone"),
        ("expires_at", "timestamp with time zone"),
        ("last_seen_at", "timestamp with time zone"),
        ("revoked_at", "timestamp with time zone"),
        ("token_hash", "bytea"),
        ("user_id", "uuid"),
    ]
    .map(|(n, t)| (n.to_string(), t.to_string()))
    .to_vec();
    assert_eq!(got, want);
    // control: the key is a digest, not a token: anything but 32 bytes is refused
    let refused = raw
        .execute(
            "INSERT INTO sessions (token_hash, user_id, created_at, expires_at, last_seen_at) \
             VALUES ('\\x74686973206973206e6f74206120646967657374'::bytea, $1, now(), now(), now())",
            &[&new_id()],
        )
        .await;
    assert!(refused.is_err(), "a 20-byte key must violate the length check");
}

/// ADR-0052 (Tests), issue #38: no column of the identity tables holds a plaintext token, key or password. The columns
/// are written out, so a `token`, `secret`, `password` or `api_key` column added later fails here and has to be argued
/// for. The constraints that keep a digest a digest are exercised too: only 32 bytes can be stored as an API key's or a
/// pending login's key, a digest and an email are unique, and a key cannot belong to a user that does not exist.
#[tokio::test]
async fn the_identity_tables_have_hashes_and_no_plaintext_and_no_workspace() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let columns = |table: &'static str| {
        let raw = &raw;
        async move {
            let rows = raw
                .query(
                    "SELECT column_name, data_type FROM information_schema.columns \
                     WHERE table_schema = 'public' AND table_name = $1 ORDER BY column_name",
                    &[&table],
                )
                .await
                .unwrap();
            rows.iter().map(|r| (r.get::<_, String>(0), r.get::<_, String>(1))).collect::<Vec<_>>()
        }
    };
    let want = |cols: &[(&str, &str)]| cols.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect::<Vec<_>>();
    let ts = "timestamp with time zone";
    assert_eq!(
        columns("users").await,
        want(&[
            ("created_at", ts),
            ("email", "text"),
            ("id", "uuid"),
            ("is_active", "boolean"),
            ("password_hash", "text"),
        ])
    );
    assert_eq!(
        columns("accounts").await,
        want(&[
            ("created_at", ts),
            ("email", "text"),
            ("provider", "text"),
            ("provider_account_id", "text"),
            ("user_id", "uuid"),
        ])
    );
    assert_eq!(
        columns("api_keys").await,
        want(&[
            ("created_at", ts),
            ("expires_at", ts),
            ("id", "uuid"),
            ("label", "text"),
            ("last_used_at", ts),
            ("prefix", "text"),
            ("revoked_at", ts),
            ("token_hash", "bytea"),
            ("user_id", "uuid"),
        ])
    );
    assert_eq!(
        columns("oidc_logins").await,
        want(&[
            ("code_verifier", "text"),
            ("created_at", ts),
            ("expires_at", ts),
            ("nonce", "text"),
            ("state_hash", "bytea"),
        ])
    );

    // controls: each of these must be refused
    let user = UserId(new_id());
    raw.execute("INSERT INTO users (id, email) VALUES ($1, 'a@example.test')", &[&user.0]).await.unwrap();
    let digest = [7u8; 32];
    let insert_key = |id: Uuid, user: Uuid, hash: Vec<u8>| {
        let raw = &raw;
        async move {
            raw.execute(
                "INSERT INTO api_keys (id, user_id, token_hash, prefix, label, created_at) \
                 VALUES ($1, $2, $3, 'thrk_x', '', now())",
                &[&id, &user, &hash],
            )
            .await
        }
    };
    insert_key(new_id(), user.0, digest.to_vec()).await.expect("a 32-byte digest is accepted");
    assert!(insert_key(new_id(), user.0, digest.to_vec()).await.is_err(), "the same digest twice");
    assert!(insert_key(new_id(), user.0, b"a plaintext token".to_vec()).await.is_err(), "a token is not a digest");
    assert!(insert_key(new_id(), new_id(), vec![8u8; 32]).await.is_err(), "a key of a user that does not exist");
    assert!(
        raw.execute("INSERT INTO users (id, email) VALUES ($1, 'a@example.test')", &[&new_id()]).await.is_err(),
        "the same email twice"
    );
    raw.execute("INSERT INTO users (id) VALUES ($1), ($2)", &[&new_id(), &new_id()])
        .await
        .expect("users without an email do not clash");
    assert!(
        raw.execute(
            "INSERT INTO oidc_logins (state_hash, nonce, code_verifier, created_at, expires_at) \
             VALUES ('\\x7374617465'::bytea, 'n', 'v', now(), now())",
            &[],
        )
        .await
        .is_err(),
        "a pending login's key must be a 32-byte digest too"
    );
}

/// `migrate` runs every file on every start, so 0005 must be re-runnable and must not touch the rows it made.
#[tokio::test]
async fn migration_0005_is_idempotent_and_keeps_its_rows() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let user = UserId(new_id());
    raw.execute("INSERT INTO users (id, email, is_active) VALUES ($1, 'keep@example.test', false)", &[&user.0])
        .await
        .unwrap();
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    let row = raw.query_one("SELECT email, is_active FROM users WHERE id = $1", &[&user.0]).await.unwrap();
    assert_eq!((row.get::<_, String>(0), row.get::<_, bool>(1)), ("keep@example.test".to_string(), false));
}

#[tokio::test]
async fn concurrent_allocation_is_unique_contiguous_and_serialised() {
    let env = env().await;
    let (ws, _o, p, _u) = seeded(&env).await;
    let s = scope(ws, p);
    let mut handles = Vec::new();
    for i in 0..24 {
        let db = env.db.clone();
        handles.push(tokio::spawn(async move {
            let mut tx = db.begin(&s).await.unwrap();
            let a = tx.allocate(&s).await.unwrap();
            tx.insert_work_item(&s, &item(ws, p, a, &format!("w{i}"), None)).await.unwrap();
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
    let s = scope(ws, p);

    let mut tx = env.db.begin(&s).await.unwrap();
    let a = tx.allocate(&s).await.unwrap();
    tx.insert_work_item(&s, &item(ws, p, a, "keep", Some("1"))).await.unwrap();
    tx.commit().await.unwrap();

    // duplicate live external key -> Conflict, and the failed tx rolls back its allocation too
    let mut tx = env.db.begin(&s).await.unwrap();
    let a2 = tx.allocate(&s).await.unwrap();
    assert_eq!(a2.sequence_id, 2);
    let err = tx.insert_work_item(&s, &item(ws, p, a2, "dup", Some("1"))).await.unwrap_err();
    assert!(matches!(err, DbError::Conflict(_)), "got {err:?}");
    tx.rollback().await.unwrap();

    // control: nothing leaked, and the counter went back (Postgres allocation is transactional)
    let rows = env.db.list_work_items(&s, Visibility::WithDeleted).await.unwrap();
    assert_eq!(rows.len(), 1);
    let mut tx = env.db.begin(&s).await.unwrap();
    assert_eq!(tx.allocate(&s).await.unwrap().sequence_id, 2, "rolled-back allocation must not leave a gap");
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn an_abandoned_transaction_does_not_poison_the_pool() {
    let env = env().await;
    let (ws, _o, p, _u) = seeded(&env).await;
    let s = scope(ws, p);
    // bounded: a pool poisoned by recycled in-transaction connections would otherwise hang the test
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        for _ in 0..40 {
            // more iterations than pool slots: if abandoned connections were recycled mid-transaction,
            // later iterations would see a stale open transaction / held row lock and hang or fail.
            let mut tx = env.db.begin(&s).await.unwrap();
            tx.allocate(&s).await.unwrap();
            drop(tx); // never committed
        }
        let mut tx = env.db.begin(&s).await.unwrap();
        let a = tx.allocate(&s).await.unwrap();
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
    let s = scope(ws, p);
    let mut tx = env.db.begin(&s).await.unwrap();
    let a = tx.allocate(&s).await.unwrap();
    let w = item(ws, p, a, "x", Some("9"));
    tx.insert_work_item(&s, &w).await.unwrap();
    assert!(tx.soft_delete_work_item(&s, w.id, Utc::now_for_test()).await.unwrap());
    // soft delete is idempotent-false the second time
    assert!(!tx.soft_delete_work_item(&s, w.id, Utc::now_for_test()).await.unwrap());
    tx.commit().await.unwrap();

    let mut tx = env.db.begin(&s).await.unwrap();
    let a = tx.allocate(&s).await.unwrap();
    assert_eq!(a.sequence_id, 2);
    tx.insert_work_item(&s, &item(ws, p, a, "x again", Some("9"))).await.unwrap();
    tx.commit().await.unwrap();

    let visible = env.db.list_work_items(&s, Visibility::Visible).await.unwrap();
    assert_eq!(visible.len(), 1);
    let all = env.db.list_work_items(&s, Visibility::WithDeleted).await.unwrap();
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
    let s = scope(ws, p);
    let sys = SystemScope::mint();
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
    let mut tx = env.db.begin(&s).await.unwrap();
    let a = tx.allocate(&s).await.unwrap();
    let w = item(ws, p, a, "gone", None);
    tx.insert_work_item(&s, &w).await.unwrap();
    tx.insert_outbox(&s, &mk(w.id)).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(env.db.outbox_depth(&sys).await.unwrap(), 0);

    // committed -> exactly one undispatched row, then marked dispatched
    let mut tx = env.db.begin(&s).await.unwrap();
    let a = tx.allocate(&s).await.unwrap();
    let w = item(ws, p, a, "kept", None);
    tx.insert_work_item(&s, &w).await.unwrap();
    let row = mk(w.id);
    tx.insert_outbox(&s, &row).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(env.db.outbox_depth(&sys).await.unwrap(), 1);
    let rows = env.db.undispatched_outbox(&sys, 10).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].envelope, row.envelope);
    env.db.mark_outbox_dispatched(&sys, row.envelope.event_id, Utc.timestamp_opt(1_800_000_001, 0).unwrap()).await.unwrap();
    assert_eq!(env.db.outbox_depth(&sys).await.unwrap(), 0);

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
    assert!(env.db.insert_activity(&sys, &act).await.unwrap());
    let mut again = act.clone();
    again.id = ActivityId(new_id());
    assert!(!env.db.insert_activity(&sys, &again).await.unwrap(), "redelivery must not insert");
    assert_eq!(env.db.list_activity(&s, Some(w.id)).await.unwrap().len(), 1);
}
