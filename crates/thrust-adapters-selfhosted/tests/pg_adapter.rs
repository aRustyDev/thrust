//! Adapter-level tests for the shared Postgres adapter, native side, against a REAL Postgres
//! started by testcontainers (OrbStack docker). These pin the properties the core relies on:
//! atomic sequence allocation, rollback atomicity, partial-unique semantics, outbox, tenancy.
//!
//! The tests below each pin one behaviour; `model` generates them instead: random sequences of `Database` and `Tx`
//! calls against a reference model. It is compiled into this binary, so it needs Docker too and stays out of the
//! hermetic default loop.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod assets;
mod counting;
mod model;
mod tls;

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
        created_by: Some(UserId(new_id())),
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

/// The names of every work item row of the scope's project, in sequence order, the deleted, archived and draft ones too: read
/// with SQL on the table, because the port has no read that includes them (its `list_work_items` went with the slice's list, step
/// 5c-3 of #58). It reads the table and not the adapter's code, so a row a failed write leaked cannot hide behind the adapter's
/// own filter.
async fn every_item_name(env: &Env, s: &Scope) -> Vec<String> {
    let raw = env.conn.raw().await.unwrap();
    let rows = raw
        .query(
            "SELECT name FROM work_items WHERE workspace_id = $1 AND project_id = $2 ORDER BY sequence_id",
            &[&s.workspace_id().0, &s.project_id().0],
        )
        .await
        .unwrap();
    rows.iter().map(|r| r.get(0)).collect()
}

/// How many live work items (not deleted, archived or a draft) the scope's project has, read with SQL on the table.
async fn live_item_count(env: &Env, s: &Scope) -> i64 {
    let raw = env.conn.raw().await.unwrap();
    raw.query_one(
        "SELECT count(*) FROM work_items WHERE workspace_id = $1 AND project_id = $2 \
         AND deleted_at IS NULL AND archived_at IS NULL AND is_draft = false",
        &[&s.workspace_id().0, &s.project_id().0],
    )
    .await
    .unwrap()
    .get(0)
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
        .seed(
            &SystemScope::mint(),
            &SeedFixture {
                workspaces: vec![SeedWorkspace { id: ws, slug: slug_of(ws) }],
                users: vec![u],
                workspace_members: vec![SeedWorkspaceMember { workspace_id: ws, user_id: u }],
                projects: vec![SeedProject { id: p, workspace_id: ws, name: "p".into() }],
                memberships: vec![SeedMembership { project_id: p, workspace_id: ws, user_id: u, role: Role::Member }],
                sessions: vec![],
                ..SeedFixture::default()
            },
        )
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
        .seed(
            &SystemScope::mint(),
            &SeedFixture {
                workspaces: vec![SeedWorkspace { id: other_ws, slug: slug_of(other_ws) }],
                workspace_members: vec![
                    SeedWorkspaceMember { workspace_id: ws, user_id: v },
                    SeedWorkspaceMember { workspace_id: other_ws, user_id: u },
                ],
                ..SeedFixture::default()
            },
        )
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
            // which digest scheme made `token_hash` (Q-0052.3, migration 0007); a version, not a secret
            ("hash_version", "smallint"),
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

/// Migration 0007 (Q-0052.3) adds `api_keys.hash_version` to a database that has keys already: every one of them was made
/// before there was a second scheme, so every one is version 1 (the column's default), and none of them is touched by
/// running the migration again. The column is NOT NULL and must be positive, and applying the file twice leaves one
/// check, not two. A pinned regression for the upgrade path (every other test starts from an empty database).
#[tokio::test]
async fn migration_0007_gives_existing_keys_version_1_and_is_idempotent() {
    let container = Postgres::default().start().await.expect("start postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let conn = Arc::new(PoolConnector::from_url(&url, 4).unwrap());
    let raw = conn.raw().await.unwrap();
    // a database as stage 2 left it: everything up to 0005, a user and two keys that have no `hash_version`
    for sql in [
        thrust_adapters_postgres::MIGRATION_0001,
        thrust_adapters_postgres::MIGRATION_0002,
        thrust_adapters_postgres::MIGRATION_0003,
        thrust_adapters_postgres::MIGRATION_0004,
        thrust_adapters_postgres::MIGRATION_0005,
    ] {
        raw.batch_execute(sql).await.unwrap();
    }
    let user = UserId(new_id());
    raw.execute("INSERT INTO users (id, email) VALUES ($1, 'legacy@example.test')", &[&user.0]).await.unwrap();
    let (a, b) = (new_id(), new_id());
    for (id, digest) in [(a, [1u8; 32]), (b, [2u8; 32])] {
        raw.execute(
            "INSERT INTO api_keys (id, user_id, token_hash, prefix, label, created_at) VALUES ($1, $2, $3, 'thrk_x', '', now())",
            &[&id, &user.0, &digest.to_vec()],
        )
        .await
        .unwrap();
    }
    let versions = || {
        let raw = &raw;
        async move {
            raw.query("SELECT hash_version FROM api_keys ORDER BY token_hash", &[])
                .await
                .unwrap()
                .iter()
                .map(|r| r.get::<_, i16>(0))
                .collect::<Vec<_>>()
        }
    };
    assert!(raw.query("SELECT hash_version FROM api_keys", &[]).await.is_err(), "control: there is no such column yet");

    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    assert_eq!(versions().await, [1, 1], "every key made before the column is version 1");
    // a key moved to another version keeps it when the migration runs again
    raw.execute("UPDATE api_keys SET hash_version = 2 WHERE id = $1", &[&a]).await.unwrap();
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    assert_eq!(versions().await, [2, 1], "applying the file again does not reset a version");

    // the column's default, NOT NULL and CHECK: a key inserted without a version is version 1; none, zero or a
    // negative one is refused; the largest smallint is accepted
    let insert = |digest: u8, version: Option<i16>| {
        let raw = &raw;
        let user = user.0;
        async move {
            match version {
                None => {
                    raw.execute(
                        "INSERT INTO api_keys (id, user_id, token_hash, prefix, label, created_at) \
                         VALUES ($1, $2, $3, 'thrk_x', '', now())",
                        &[&new_id(), &user, &vec![digest; 32]],
                    )
                    .await
                }
                Some(v) => {
                    raw.execute(
                        "INSERT INTO api_keys (id, user_id, token_hash, hash_version, prefix, label, created_at) \
                         VALUES ($1, $2, $3, $4, 'thrk_x', '', now())",
                        &[&new_id(), &user, &vec![digest; 32], &v],
                    )
                    .await
                }
            }
        }
    };
    insert(10, None).await.expect("the default");
    assert_eq!(
        raw.query_one("SELECT hash_version FROM api_keys WHERE token_hash = $1", &[&vec![10u8; 32]])
            .await
            .unwrap()
            .get::<_, i16>(0),
        1
    );
    insert(11, Some(i16::MAX)).await.expect("the largest version");
    assert!(insert(12, Some(0)).await.is_err(), "zero is not a version");
    assert!(insert(13, Some(-1)).await.is_err(), "a negative one is not either");
    assert!(raw.execute("UPDATE api_keys SET hash_version = NULL WHERE id = $1", &[&a]).await.is_err(), "NOT NULL");
    // idempotent in the schema too: one CHECK on the version, not one per run
    let checks = raw
        .query_one(
            "SELECT count(*) FROM pg_constraint WHERE conrelid = 'api_keys'::regclass AND contype = 'c' \
             AND pg_get_constraintdef(oid) LIKE '%hash_version%'",
            &[],
        )
        .await
        .unwrap()
        .get::<_, i64>(0);
    assert_eq!(checks, 1);
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

fn profile_for(user: UserId) -> UserProfile {
    let now = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
    UserProfile {
        id: new_id(),
        user_id: user,
        username: format!("u-{}", new_id().simple()),
        first_name: String::new(),
        last_name: String::new(),
        display_name: "d".into(),
        avatar: String::new(),
        cover_image: None,
        timezone: "UTC".into(),
        date_joined: now,
        last_login_at: None,
        is_onboarded: false,
        is_tour_completed: false,
        last_workspace_id: None,
        onboarding_step: serde_json::json!({}),
        preferences: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    }
}

async fn user_with_profile(env: &Env) -> UserId {
    let user = UserId(new_id());
    env.db
        .insert_user(&UserRecord {
            id: user,
            email: Some(format!("{}@example.test", user.0.simple())),
            password_hash: None,
            is_active: true,
        })
        .await
        .unwrap();
    assert!(env.db.insert_profile(&profile_for(user)).await.unwrap());
    user
}

/// Issue #44, migration 0008: the profile table's columns are written out, so a column added later has to be argued for
/// (it is a directory table: keyed by the user, with no tenant key and no secret), and the constraints that keep a profile
/// well formed are exercised: one per user, a user that exists, a unique id and username, JSON objects and not scalars.
#[tokio::test]
async fn the_profile_table_has_the_columns_it_says_and_the_constraints_that_keep_it_well_formed() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let rows = raw
        .query(
            "SELECT column_name, data_type FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = 'user_profiles' ORDER BY column_name",
            &[],
        )
        .await
        .unwrap();
    let got: Vec<(String, String)> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();
    let ts = "timestamp with time zone";
    let want: Vec<(&str, &str)> = vec![
        ("avatar", "text"),
        ("cover_image", "text"),
        ("created_at", ts),
        ("date_joined", ts),
        ("display_name", "text"),
        ("first_name", "text"),
        ("id", "uuid"),
        ("is_onboarded", "boolean"),
        ("is_tour_completed", "boolean"),
        // a value the app writes and reads back, not a tenant key: no foreign key, so it names no workspace the database knows
        ("last_login_at", ts),
        ("last_name", "text"),
        ("last_workspace_id", "uuid"),
        ("onboarding_step", "jsonb"),
        ("preferences", "jsonb"),
        ("timezone", "text"),
        ("updated_at", ts),
        ("user_id", "uuid"),
        ("username", "text"),
    ];
    assert_eq!(got, want.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect::<Vec<_>>());

    let user = user_with_profile(&env).await;
    // one profile per user: the second is refused without an error, and the first is untouched
    let mut again = profile_for(user);
    again.display_name = "second".into();
    assert!(!env.db.insert_profile(&again).await.unwrap());
    assert_eq!(env.db.profile(user).await.unwrap().unwrap().display_name, "d");
    // the same user wins over the unique columns, as in the memory adapter: a second profile that also reuses the first's id
    // and username is still `false`, not a conflict
    let held = env.db.profile(user).await.unwrap().unwrap();
    let mut clash = profile_for(user);
    (clash.id, clash.username) = (held.id, held.username.clone());
    assert!(!env.db.insert_profile(&clash).await.unwrap());
    // a profile for a user that does not exist
    let stranger = UserId(new_id());
    assert!(env.db.insert_profile(&profile_for(stranger)).await.is_err());
    assert!(env.db.profile(stranger).await.unwrap().is_none());
    // a taken profile id, and a taken username, are conflicts and write nothing
    let other = UserId(new_id());
    env.db.insert_user(&UserRecord { id: other, email: None, password_hash: None, is_active: true }).await.unwrap();
    let mut same_id = profile_for(other);
    same_id.id = env.db.profile(user).await.unwrap().unwrap().id;
    assert!(matches!(env.db.insert_profile(&same_id).await, Err(DbError::Conflict(_))));
    let mut same_name = profile_for(other);
    same_name.username = env.db.profile(user).await.unwrap().unwrap().username;
    assert!(matches!(env.db.insert_profile(&same_name).await, Err(DbError::Conflict(_))));
    assert!(env.db.profile(other).await.unwrap().is_none());
    // the JSON columns hold objects: a scalar or an array is refused by the database itself
    for (column, bad) in
        [("onboarding_step", "[]"), ("onboarding_step", "1"), ("preferences", "[]"), ("preferences", "\"x\"")]
    {
        let sql = format!("UPDATE user_profiles SET {column} = $2::jsonb WHERE user_id = $1");
        let refused = raw.execute(&sql, &[&user.0, &bad]).await;
        assert!(refused.is_err(), "{column} = {bad}");
    }
    // control: an object is accepted
    raw.execute("UPDATE user_profiles SET preferences = '{\"k\": 1}'::jsonb WHERE user_id = $1", &[&user.0])
        .await
        .unwrap();
}

#[tokio::test]
async fn migration_0008_is_idempotent_and_keeps_its_rows() {
    let env = env().await;
    let user = user_with_profile(&env).await;
    env.db
        .update_profile(
            user,
            &ProfilePatch { first_name: Some("Kept".into()), ..ProfilePatch::default() },
            Utc.timestamp_opt(1_800_000_100, 0).unwrap(),
        )
        .await
        .unwrap();
    let raw = env.conn.raw().await.unwrap();
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    assert_eq!(env.db.profile(user).await.unwrap().unwrap().first_name, "Kept");
}

fn workspace_for(owner: UserId, tag: &str) -> NewWorkspace {
    let now = Utc.timestamp_opt(1_800_000_200, 0).unwrap();
    let id = WorkspaceId(new_id());
    NewWorkspace {
        workspace: Workspace {
            id,
            slug: WorkspaceSlug::new(format!("{tag}-{}", new_id().simple())),
            name: format!("Workspace {tag}"),
            owner_id: Some(owner),
            organization_size: None,
            timezone: "UTC".into(),
            background_color: "#808080".into(),
            created_at: now,
            updated_at: now,
        },
        owner: WorkspaceMember {
            id: new_id(),
            workspace_id: id,
            user_id: owner,
            role: Role::Admin,
            company_role: None,
            is_active: true,
            props: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        },
    }
}

async fn user_without_login(env: &Env) -> UserId {
    let user = UserId(new_id());
    env.db.insert_user(&UserRecord { id: user, email: None, password_hash: None, is_active: true }).await.unwrap();
    user
}

/// Issue #44, migration 0009: the workspace tables' columns are written out, so a column added later has to be argued for,
/// and the constraints that keep a workspace and a membership well formed are exercised: a name, a role from the three, view
/// settings that are an object, a unique membership id, an owner who is a user.
#[tokio::test]
async fn the_workspace_tables_have_the_columns_they_say_and_the_constraints_that_keep_them_well_formed() {
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
    let ts = "timestamp with time zone";
    let pairs = |v: Vec<(&str, &str)>| v.into_iter().map(|(n, t)| (n.to_string(), t.to_string())).collect::<Vec<_>>();
    assert_eq!(
        columns("workspaces").await,
        pairs(vec![
            ("background_color", "text"),
            ("created_at", ts),
            ("id", "uuid"),
            ("name", "text"),
            ("organization_size", "text"),
            ("owner_id", "uuid"),
            ("slug", "text"),
            ("timezone", "text"),
            ("updated_at", ts),
        ])
    );
    assert_eq!(
        columns("workspace_members").await,
        pairs(vec![
            ("company_role", "text"),
            ("created_at", ts),
            ("id", "uuid"),
            ("is_active", "boolean"),
            ("props", "jsonb"),
            ("role", "text"),
            ("updated_at", ts),
            ("user_id", "uuid"),
            ("workspace_id", "uuid"),
        ])
    );

    let owner = user_without_login(&env).await;
    let w = workspace_for(owner, "constraints");
    env.db.insert_workspace(&w).await.unwrap();
    let (ws, id) = (w.workspace.id.0, w.owner.id);
    // a role outside the three, and view settings that are not an object, are refused by the database itself
    for (set, why) in
        [("role = 'owner'", "role"), ("props = '[]'::jsonb", "props array"), ("props = '1'::jsonb", "props scalar")]
    {
        let refused = raw.execute(&format!("UPDATE workspace_members SET {set} WHERE workspace_id = $1"), &[&ws]).await;
        assert!(refused.is_err(), "{why}");
    }
    // control: a role from the three and an object are accepted
    raw.execute(
        "UPDATE workspace_members SET role = 'guest', props = '{\"k\": 1}'::jsonb WHERE workspace_id = $1",
        &[&ws],
    )
    .await
    .unwrap();
    assert_eq!(
        env.db.workspace_member(&WorkspaceScope::mint(w.workspace.id, owner, Role::Guest)).await.unwrap().unwrap().role,
        Role::Guest
    );
    // a workspace has a name
    assert!(raw.execute("UPDATE workspaces SET name = NULL WHERE id = $1", &[&ws]).await.is_err());
    // a membership id is unique, and a taken one is a conflict that takes its workspace back with it
    let mut clash = workspace_for(user_without_login(&env).await, "clash");
    clash.owner.id = id;
    let slug = clash.workspace.slug.as_str().to_owned();
    assert!(matches!(env.db.insert_workspace(&clash).await, Err(DbError::Conflict(_))));
    assert!(!env.db.workspace_slug_taken(&slug).await.unwrap(), "the workspace of a failed creation stayed");
    // an owner who is no user is refused (a foreign key is a backend error, not a conflict)
    let stranger = workspace_for(UserId(new_id()), "stranger");
    assert!(matches!(env.db.insert_workspace(&stranger).await, Err(DbError::Backend(_))));
    assert!(!env.db.workspace_slug_taken(stranger.workspace.slug.as_str()).await.unwrap());
}

/// A database created before migration 0009 has workspaces that are a slug and members that are a pair of ids. `migrate`
/// gives them a name (the slug), no owner, an id of their own, the role `member`, active, an empty view-settings object and
/// the default time zone and colour, and applying it a second time changes nothing. A pinned regression for the upgrade
/// path of a database that has data; every other test here starts from an empty one.
#[tokio::test]
async fn migration_0009_upgrades_a_database_that_has_workspaces_and_members() {
    let container = Postgres::default().start().await.expect("start postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let conn = Arc::new(PoolConnector::from_url(&url, 4).unwrap());
    let raw = conn.raw().await.unwrap();
    for sql in [
        thrust_adapters_postgres::MIGRATION_0001,
        thrust_adapters_postgres::MIGRATION_0002,
        thrust_adapters_postgres::MIGRATION_0003,
        thrust_adapters_postgres::MIGRATION_0004,
        thrust_adapters_postgres::MIGRATION_0005,
        thrust_adapters_postgres::MIGRATION_0006,
        thrust_adapters_postgres::MIGRATION_0007,
        thrust_adapters_postgres::MIGRATION_0008,
    ] {
        raw.batch_execute(sql).await.unwrap();
    }
    let (ws, u1, u2) = (new_id(), new_id(), new_id());
    raw.execute("INSERT INTO workspaces (id, slug) VALUES ($1, 'legacy-space')", &[&ws]).await.unwrap();
    raw.execute("INSERT INTO workspace_members (workspace_id, user_id) VALUES ($1, $2), ($1, $3)", &[&ws, &u1, &u2])
        .await
        .unwrap();
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    let first = raw.query("SELECT user_id, id FROM workspace_members ORDER BY user_id", &[]).await.unwrap();
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    let again = raw.query("SELECT user_id, id FROM workspace_members ORDER BY user_id", &[]).await.unwrap();
    let ids = |rows: &[tokio_postgres::Row]| {
        rows.iter().map(|r| (r.get::<_, Uuid>(0), r.get::<_, Uuid>(1))).collect::<Vec<_>>()
    };
    assert_eq!(ids(&first), ids(&again), "a second run gave the members new ids");
    assert_ne!(ids(&first)[0].1, ids(&first)[1].1, "two members share an id");

    drop(raw);
    let db = PgDatabase::new(conn);
    let listed = db.workspaces_of(UserId(u1)).await.unwrap();
    assert_eq!(listed.len(), 1);
    let (w, role, total) = (&listed[0].workspace, listed[0].role, listed[0].total_members);
    assert_eq!(
        (w.id, w.slug.as_str(), w.name.as_str(), w.owner_id),
        (WorkspaceId(ws), "legacy-space", "legacy-space", None)
    );
    assert_eq!(
        (w.timezone.as_str(), w.background_color.as_str(), w.organization_size.clone()),
        ("UTC", "#808080", None)
    );
    assert_eq!((role, total), (Role::Member, 2));
    let m =
        db.workspace_member(&WorkspaceScope::mint(WorkspaceId(ws), UserId(u2), Role::Member)).await.unwrap().unwrap();
    assert_eq!((m.is_active, m.company_role.clone(), m.props.clone()), (true, None, serde_json::json!({})));
    assert!(db.workspace_access(&WorkspaceSlug::new("legacy-space"), UserId(u1)).await.unwrap().is_some());
}

/// Two creations racing for one slug cannot both win, and the loser leaves no member row behind: the unique index decides,
/// not a check-then-insert. 16 users from as many tasks make a workspace with the same slug.
#[tokio::test]
async fn creations_racing_for_one_slug_make_exactly_one_workspace() {
    let env = env().await;
    let slug = WorkspaceSlug::new(format!("race-{}", new_id().simple()));
    let mut users = Vec::new();
    for _ in 0..16 {
        users.push(user_without_login(&env).await);
    }
    let mut handles = Vec::new();
    for owner in users.clone() {
        let db = env.db.clone();
        let mut w = workspace_for(owner, "race");
        w.workspace.slug = slug.clone();
        handles.push(tokio::spawn(async move { db.insert_workspace(&w).await }));
    }
    let mut won = 0;
    for h in handles {
        match h.await.unwrap() {
            Ok(()) => won += 1,
            Err(DbError::Conflict(_)) => {}
            Err(e) => panic!("a loser was refused with {e:?}, not a conflict"),
        }
    }
    assert_eq!(won, 1);
    let mut have = 0;
    for u in users {
        have += env.db.workspaces_of(u).await.unwrap().len();
    }
    assert_eq!(have, 1, "a loser kept a membership");
}

/// Issue #44, migrations 0010 and 0011: the columns of the project and first-visit tables are written out, and the constraints
/// that keep them well formed are exercised: a network of two words, JSON that is an object, a navigation control of two words,
/// rows that belong to a workspace and a user that exist, and one row per member and key.
#[tokio::test]
async fn the_project_and_first_visit_tables_have_the_columns_they_say_and_the_constraints_that_keep_them_well_formed() {
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
    let ts = "timestamp with time zone";
    let pairs = |v: Vec<(&str, &str)>| v.into_iter().map(|(n, t)| (n.to_string(), t.to_string())).collect::<Vec<_>>();
    // (the columns of `projects` and `project_user_properties` are asserted whole by the test of migration 0012, which extends both)
    assert_eq!(
        columns("workspace_user_preferences").await,
        pairs(vec![
            ("created_at", ts),
            ("is_pinned", "boolean"),
            ("key", "text"),
            ("sort_order", "double precision"),
            ("updated_at", ts),
            ("user_id", "uuid"),
            ("workspace_id", "uuid"),
        ])
    );
    assert_eq!(
        columns("workspace_home_preferences").await,
        pairs(vec![
            ("config", "jsonb"),
            ("created_at", ts),
            ("is_enabled", "boolean"),
            ("key", "text"),
            ("sort_order", "double precision"),
            ("updated_at", ts),
            ("user_id", "uuid"),
            ("workspace_id", "uuid"),
        ])
    );
    assert_eq!(
        columns("workspace_user_properties").await,
        pairs(vec![
            ("created_at", ts),
            ("display_filters", "jsonb"),
            ("display_properties", "jsonb"),
            ("filters", "jsonb"),
            ("id", "uuid"),
            ("navigation_control_preference", "text"),
            ("navigation_project_limit", "integer"),
            ("rich_filters", "jsonb"),
            ("updated_at", ts),
            ("user_id", "uuid"),
            ("workspace_id", "uuid"),
        ])
    );

    let owner = user_without_login(&env).await;
    let w = workspace_for(owner, "first-visit");
    env.db.insert_workspace(&w).await.unwrap();
    let ws = w.workspace.id.0;
    let project = new_id();
    raw.execute(
        "INSERT INTO projects (id, workspace_id, name, identifier) VALUES ($1, $2, 'p', 'P')",
        &[&project, &ws],
    )
    .await
    .unwrap();
    // a network outside the two words, and a logo that is not an object, are refused by the database itself
    for (set, why) in [
        ("network = 'private'", "network"),
        ("logo_props = '[]'::jsonb", "logo array"),
        ("logo_props = '1'::jsonb", "logo scalar"),
    ] {
        assert!(raw.execute(&format!("UPDATE projects SET {set} WHERE id = $1"), &[&project]).await.is_err(), "{why}");
    }
    // a project has an identifier
    assert!(raw.execute("UPDATE projects SET identifier = NULL WHERE id = $1", &[&project]).await.is_err());
    // control: the two networks and an object are accepted
    raw.execute(
        "UPDATE projects SET network = 'secret', logo_props = '{\"in_use\": \"emoji\"}'::jsonb WHERE id = $1",
        &[&project],
    )
    .await
    .unwrap();

    // the first-visit rows belong to a workspace and a user that exist
    let stranger = new_id();
    let pref = "INSERT INTO workspace_user_preferences (workspace_id, user_id, key, is_pinned, sort_order, created_at, updated_at) \
                VALUES ($1, $2, 'k', false, 1, now(), now())";
    assert!(raw.execute(pref, &[&ws, &stranger]).await.is_err(), "a row for a user who does not exist");
    assert!(raw.execute(pref, &[&stranger, &owner.0]).await.is_err(), "a row for a workspace that does not exist");
    raw.execute(pref, &[&ws, &owner.0]).await.unwrap();
    assert!(raw.execute(pref, &[&ws, &owner.0]).await.is_err(), "two rows for one key");
    let home = "INSERT INTO workspace_home_preferences (workspace_id, user_id, key, is_enabled, config, sort_order, created_at, updated_at) \
                VALUES ($1, $2, 'k', true, $3, 1, now(), now())";
    assert!(
        raw.execute(home, &[&ws, &owner.0, &serde_json::json!([])]).await.is_err(),
        "a widget config that is not an object"
    );
    raw.execute(home, &[&ws, &owner.0, &serde_json::json!({"a": 1})]).await.unwrap();
    let props = "INSERT INTO workspace_user_properties (id, workspace_id, user_id, filters, display_filters, display_properties, \
                   rich_filters, navigation_project_limit, navigation_control_preference, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, '{}', '{}', '{}', 10, $5, now(), now())";
    let object = serde_json::json!({});
    assert!(
        raw.execute(props, &[&new_id(), &ws, &owner.0, &serde_json::json!([]), &"ACCORDION"]).await.is_err(),
        "filters that are no object"
    );
    assert!(
        raw.execute(props, &[&new_id(), &ws, &owner.0, &object, &"SIDEWAYS"]).await.is_err(),
        "a navigation control of another word"
    );
    raw.execute(props, &[&new_id(), &ws, &owner.0, &object, &"TABBED"]).await.unwrap();
    assert!(
        raw.execute(props, &[&new_id(), &ws, &owner.0, &object, &"TABBED"]).await.is_err(),
        "two settings rows for one member"
    );
}

/// A database created before migration 0010 has projects that are an id, a workspace and a name, and 0011 adds tables beside
/// them. `migrate` gives each project the identifier `default_project_identifier` makes from its id, a public network, no
/// archive time, an empty logo and the default views (only pages on), and applying it again changes nothing. A pinned regression
/// for the upgrade path of a database that has data; every other test here starts from an empty one.
#[tokio::test]
async fn migrations_0010_and_0011_upgrade_a_database_that_has_projects() {
    let container = Postgres::default().start().await.expect("start postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let conn = Arc::new(PoolConnector::from_url(&url, 4).unwrap());
    let raw = conn.raw().await.unwrap();
    for sql in [
        thrust_adapters_postgres::MIGRATION_0001,
        thrust_adapters_postgres::MIGRATION_0002,
        thrust_adapters_postgres::MIGRATION_0003,
        thrust_adapters_postgres::MIGRATION_0004,
        thrust_adapters_postgres::MIGRATION_0005,
        thrust_adapters_postgres::MIGRATION_0006,
        thrust_adapters_postgres::MIGRATION_0007,
        thrust_adapters_postgres::MIGRATION_0008,
        thrust_adapters_postgres::MIGRATION_0009,
    ] {
        raw.batch_execute(sql).await.unwrap();
    }
    let (ws, user) = (new_id(), new_id());
    // ids that differ only in their first group, as random ones do, and ones that differ only in their last
    let projects = [new_id(), Uuid::from_u128(0xAB), Uuid::from_u128(0xCD)];
    raw.execute("INSERT INTO users (id) VALUES ($1)", &[&user]).await.unwrap();
    raw.execute("INSERT INTO workspaces (id, slug, name) VALUES ($1, 'legacy-projects', 'Legacy')", &[&ws])
        .await
        .unwrap();
    raw.execute("INSERT INTO workspace_members (workspace_id, user_id) VALUES ($1, $2)", &[&ws, &user]).await.unwrap();
    for p in projects {
        raw.execute(
            "INSERT INTO projects (id, workspace_id, name) VALUES ($1, $2, $3)",
            &[&p, &ws, &format!("legacy {p}")],
        )
        .await
        .unwrap();
        raw.execute(
            "INSERT INTO project_members (project_id, workspace_id, user_id, role) VALUES ($1, $2, $3, 'member')",
            &[&p, &ws, &user],
        )
        .await
        .unwrap();
    }
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    async fn state(raw: &tokio_postgres::Client) -> Vec<(Uuid, String, String, Timestamp)> {
        raw.query("SELECT id, identifier, network, created_at FROM projects ORDER BY id", &[])
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get::<_, Uuid>(0), r.get::<_, String>(1), r.get::<_, String>(2), r.get::<_, Timestamp>(3)))
            .collect()
    }
    let first = state(&raw).await;
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    assert_eq!(first, state(&raw).await, "a second run changed the projects");
    for (id, identifier, network, _) in &first {
        assert_eq!(identifier, &default_project_identifier(ProjectId(*id)), "the backfill and the helper disagree");
        assert_eq!(network, "public");
    }
    assert_eq!(first.iter().find(|r| r.0 == Uuid::from_u128(0xAB)).unwrap().1, "0000000000AB");

    drop(raw);
    let db = PgDatabase::new(conn);
    let listed =
        db.workspace_projects(&WorkspaceScope::mint(WorkspaceId(ws), UserId(user), Role::Member)).await.unwrap();
    assert_eq!(listed.len(), 3);
    for l in &listed {
        let p = &l.project;
        assert_eq!(
            (p.network, p.archived_at, p.logo_props.clone(), p.project_lead, p.created_by),
            (ProjectNetwork::Public, None, serde_json::json!({}), None, None)
        );
        assert_eq!(
            (p.cycle_view, p.issue_views_view, p.module_view, p.page_view, p.intake_view, p.guest_view_all_features),
            (false, false, false, true, false, false)
        );
        assert_eq!((l.role, l.sort_order), (Some(Role::Member), None));
    }
}

/// A member's first visits can race (the app reads the sidebar, the home page and the list settings at once, and a second tab
/// does the same): 16 tasks ask for the same rows, and every one of them gets every row, each made once. The unique key
/// decides, and the loser's SELECT, which comes after its INSERT, sees what the winner made.
#[tokio::test]
async fn first_visits_racing_make_each_row_once_and_every_visit_sees_all_of_them() {
    let env = env().await;
    let owner = user_without_login(&env).await;
    let w = workspace_for(owner, "visit-race");
    env.db.insert_workspace(&w).await.unwrap();
    let scope = WorkspaceScope::mint(w.workspace.id, owner, Role::Admin);
    let now = Utc.timestamp_opt(1_800_000_300, 0).unwrap();
    let sidebar: Vec<SidebarPreference> = (0..7)
        .map(|i| SidebarPreference {
            key: format!("key{i}"),
            is_pinned: i % 2 == 0,
            sort_order: 65535.0 + 10000.0 * f64::from(i),
        })
        .collect();
    let home: Vec<HomePreference> = (0..3)
        .map(|i| HomePreference {
            key: format!("widget{i}"),
            is_enabled: true,
            config: serde_json::json!({}),
            sort_order: 999.0 - f64::from(i),
            created_at: now,
        })
        .collect();
    let mut handles = Vec::new();
    for task in 0..16u128 {
        let (db, sidebar, home) = (env.db.clone(), sidebar.clone(), home.clone());
        let properties = UserProperties {
            id: Uuid::from_u128(0x7000 + task),
            filters: serde_json::json!({}),
            display_filters: serde_json::json!({}),
            display_properties: serde_json::json!({}),
            rich_filters: serde_json::json!({}),
            navigation_project_limit: 10,
            navigation_control_preference: "ACCORDION".into(),
            created_at: now,
            updated_at: now,
        };
        handles.push(tokio::spawn(async move {
            let s = db.ensure_sidebar_preferences(&scope, &sidebar, now).await.unwrap();
            let h = db.ensure_home_preferences(&scope, &home).await.unwrap();
            let p = db.ensure_user_properties(&scope, &properties).await.unwrap();
            (s, h, p.id)
        }));
    }
    let mut ids = std::collections::BTreeSet::new();
    for h in handles {
        let (s, h, id) = h.await.unwrap();
        assert_eq!(s, sidebar, "a visit did not see all the sidebar rows");
        assert_eq!(h.len(), 3, "a visit did not see all the widgets");
        ids.insert(id);
    }
    assert_eq!(ids.len(), 1, "the settings were made more than once, or the visits saw different rows");
    let raw = env.conn.raw().await.unwrap();
    for (table, want) in
        [("workspace_user_preferences", 7i64), ("workspace_home_preferences", 3), ("workspace_user_properties", 1)]
    {
        let n: i64 = raw
            .query_one(&format!("SELECT count(*) FROM {table} WHERE workspace_id = $1"), &[&w.workspace.id.0])
            .await
            .unwrap()
            .get(0);
        assert_eq!(n, want, "{table}");
    }
}

/// A project of workspace `ws` made by `owner`, who is its one admin, with two states (the default one and the intake one).
fn project_for(ws: WorkspaceId, owner: UserId, name: &str, identifier: &str) -> NewProject {
    let now = Utc.timestamp_opt(1_800_000_400, 0).unwrap();
    let id = ProjectId(new_id());
    let mut project = ProjectRow::bare(id, ws, name, now);
    project.identifier = identifier.to_string();
    project.created_by = Some(owner);
    let state = |name: &str, sequence: f64, group: StateGroup, is_default: bool| StateRow {
        id: new_id(),
        workspace_id: ws,
        project_id: id,
        name: name.into(),
        description: String::new(),
        color: "#000000".into(),
        sequence,
        group,
        is_default,
        created_by: Some(owner),
        created_at: now,
        updated_at: now,
        external_source: None,
        external_id: None,
        updated_by: None,
    };
    NewProject {
        members: vec![NewProjectMember {
            member: ProjectMemberRow {
                id: new_id(),
                project_id: id,
                workspace_id: ws,
                user_id: owner,
                role: Role::Admin,
                comment: None,
                view_props: serde_json::json!({}),
                default_props: serde_json::json!({}),
                preferences: serde_json::json!({}),
                sort_order: 65535.0,
                created_by: Some(owner),
                created_at: now,
                updated_at: now,
            },
            properties: ProjectUserProperties {
                id: new_id(),
                project_id: id,
                workspace_id: ws,
                user_id: owner,
                filters: serde_json::json!({}),
                display_filters: serde_json::json!({}),
                display_properties: serde_json::json!({}),
                rich_filters: serde_json::json!({}),
                preferences: serde_json::json!({}),
                sort_order: 65535.0,
                created_at: now,
                updated_at: now,
            },
        }],
        states: vec![
            state("Backlog", 15000.0, StateGroup::Backlog, true),
            state("Triage", 65000.0, StateGroup::Triage, false),
        ],
        project,
    }
}

/// Issue #44, migration 0012: the project, member, settings, state and label tables' columns are written out, so a column added later
/// has to be argued for, and the constraints that keep them well formed are exercised: months from 0 to 12, a name and a key that are
/// each unique in their workspace, users that exist, settings that are objects, a state group from the six, one default state, a
/// state or a label that belongs to a project of its own workspace.
#[tokio::test]
async fn the_project_tables_have_the_columns_they_say_and_the_constraints_that_keep_them_well_formed() {
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
    let ts = "timestamp with time zone";
    let (text, uuid, boolean, jsonb, double) = ("text", "uuid", "boolean", "jsonb", "double precision");
    let pairs = |v: Vec<(&str, &str)>| v.into_iter().map(|(n, t)| (n.to_string(), t.to_string())).collect::<Vec<_>>();
    assert_eq!(
        columns("projects").await,
        pairs(vec![
            ("archive_in", "integer"),
            ("archived_at", ts),
            ("close_in", "integer"),
            ("cover_image", text),
            ("cover_image_asset", uuid),
            ("created_at", ts),
            ("created_by", uuid),
            ("cycle_view", boolean),
            ("default_assignee", uuid),
            ("default_state", uuid),
            ("deleted_at", ts),
            ("description", text),
            ("description_html", jsonb),
            ("description_text", jsonb),
            ("emoji", text),
            ("estimate", uuid),
            ("external_id", text),
            ("external_source", text),
            ("guest_view_all_features", boolean),
            ("icon_prop", jsonb),
            ("id", uuid),
            ("identifier", text),
            ("intake_view", boolean),
            ("is_issue_type_enabled", boolean),
            ("is_time_tracking_enabled", boolean),
            ("issue_views_view", boolean),
            ("logo_props", jsonb),
            ("module_view", boolean),
            ("name", text),
            ("network", text),
            ("page_view", boolean),
            ("project_lead", uuid),
            ("timezone", text),
            ("updated_at", ts),
            ("updated_by", uuid),
            ("workspace_id", uuid),
        ])
    );
    assert_eq!(
        columns("project_members").await,
        pairs(vec![
            ("comment", text),
            ("created_at", ts),
            ("created_by", uuid),
            ("default_props", jsonb),
            ("id", uuid),
            ("preferences", jsonb),
            ("project_id", uuid),
            ("role", text),
            ("sort_order", double),
            ("updated_at", ts),
            ("user_id", uuid),
            ("view_props", jsonb),
            ("workspace_id", uuid),
        ])
    );
    assert_eq!(
        columns("project_user_properties").await,
        pairs(vec![
            ("created_at", ts),
            ("display_filters", jsonb),
            ("display_properties", jsonb),
            ("filters", jsonb),
            ("id", uuid),
            ("preferences", jsonb),
            ("project_id", uuid),
            ("rich_filters", jsonb),
            ("sort_order", double),
            ("updated_at", ts),
            ("user_id", uuid),
            ("workspace_id", uuid),
        ])
    );
    assert_eq!(
        columns("states").await,
        pairs(vec![
            ("color", text),
            ("created_at", ts),
            ("created_by", uuid),
            ("deleted_at", ts),
            ("description", text),
            ("external_id", text),
            ("external_source", text),
            ("id", uuid),
            ("is_default", boolean),
            ("name", text),
            ("project_id", uuid),
            ("sequence", double),
            ("state_group", text),
            ("updated_at", ts),
            ("updated_by", uuid),
            ("workspace_id", uuid),
        ])
    );
    assert_eq!(
        columns("labels").await,
        pairs(vec![
            ("color", text),
            ("created_at", ts),
            ("created_by", uuid),
            ("deleted_at", ts),
            ("description", text),
            ("external_id", text),
            ("external_source", text),
            ("id", uuid),
            ("name", text),
            ("parent_id", uuid),
            ("project_id", uuid),
            ("sort_order", double),
            ("updated_at", ts),
            ("updated_by", uuid),
            ("workspace_id", uuid),
        ])
    );

    let owner = user_without_login(&env).await;
    let (w1, w2) = (workspace_for(owner, "projects-one"), workspace_for(owner, "projects-two"));
    env.db.insert_workspace(&w1).await.unwrap();
    env.db.insert_workspace(&w2).await.unwrap();
    let scope = |w: &NewWorkspace| WorkspaceScope::mint(w.workspace.id, owner, Role::Admin);
    let first = project_for(w1.workspace.id, owner, "First", "FST");
    env.db.insert_project(&scope(&w1), &first).await.unwrap();
    let project = first.project.id.0;
    // months from 0 to 12
    for (set, ok) in
        [("archive_in = 12", true), ("archive_in = 0", true), ("archive_in = 13", false), ("close_in = -1", false)]
    {
        assert_eq!(
            raw.execute(&format!("UPDATE projects SET {set} WHERE id = $1"), &[&project]).await.is_ok(),
            ok,
            "{set}"
        );
    }
    // a name and a key are each unique in their workspace, and free in another
    let clash = |name: &str, identifier: &str| project_for(w1.workspace.id, owner, name, identifier);
    assert!(
        matches!(env.db.insert_project(&scope(&w1), &clash("First", "OTH")).await, Err(DbError::Conflict(_))),
        "the name"
    );
    assert!(
        matches!(env.db.insert_project(&scope(&w1), &clash("Other", "FST")).await, Err(DbError::Conflict(_))),
        "the key"
    );
    env.db.insert_project(&scope(&w2), &project_for(w2.workspace.id, owner, "First", "FST")).await.unwrap();
    // the users a project names are users
    let nobody = new_id();
    for column in ["created_by", "updated_by", "project_lead", "default_assignee"] {
        assert!(
            raw.execute(&format!("UPDATE projects SET {column} = $2 WHERE id = $1"), &[&project, &nobody])
                .await
                .is_err(),
            "{column}"
        );
    }
    // settings are objects
    for set in ["view_props = '[]'::jsonb", "default_props = '1'::jsonb", "preferences = 'null'::jsonb"] {
        assert!(
            raw.execute(&format!("UPDATE project_members SET {set} WHERE project_id = $1"), &[&project]).await.is_err(),
            "{set}"
        );
    }
    for set in [
        "filters = '[]'::jsonb",
        "display_filters = '1'::jsonb",
        "display_properties = '\"x\"'::jsonb",
        "rich_filters = '[]'::jsonb",
        "preferences = '1'::jsonb",
    ] {
        assert!(
            raw.execute(&format!("UPDATE project_user_properties SET {set} WHERE project_id = $1"), &[&project])
                .await
                .is_err(),
            "{set}"
        );
    }
    // control: objects are accepted
    raw.execute("UPDATE project_members SET view_props = '{\"a\": 1}'::jsonb WHERE project_id = $1", &[&project])
        .await
        .unwrap();
    // a state: a group from the six, a name once per project, one default, a project of its own workspace
    let state = "INSERT INTO states (id, workspace_id, project_id, name, color, sequence, state_group, is_default, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, '#fff', 1, $5, $6, now(), now())";
    let ws1 = w1.workspace.id.0;
    assert!(
        raw.execute(state, &[&new_id(), &ws1, &project, &"Review", &"reviewing", &false]).await.is_err(),
        "a group of its own"
    );
    assert!(
        raw.execute(state, &[&new_id(), &ws1, &project, &"Backlog", &"backlog", &false]).await.is_err(),
        "a name twice"
    );
    assert!(
        raw.execute(state, &[&new_id(), &ws1, &project, &"Second default", &"started", &true]).await.is_err(),
        "two default states"
    );
    assert!(
        raw.execute(state, &[&new_id(), &w2.workspace.id.0, &project, &"Elsewhere", &"started", &false]).await.is_err(),
        "a state in another workspace than its project's"
    );
    raw.execute(state, &[&new_id(), &ws1, &project, &"Review", &"started", &false]).await.unwrap();
    // a label: a name once per project, a parent that exists, a project of its own workspace
    let label = "INSERT INTO labels (id, workspace_id, project_id, parent_id, name, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, now(), now())";
    let parent = new_id();
    raw.execute(label, &[&parent, &ws1, &project, &None::<Uuid>, &"bug"]).await.unwrap();
    assert!(
        raw.execute(label, &[&new_id(), &ws1, &project, &None::<Uuid>, &"bug"]).await.is_err(),
        "a label name twice"
    );
    assert!(
        raw.execute(label, &[&new_id(), &ws1, &project, &Some(new_id()), &"child"]).await.is_err(),
        "a parent that is no label"
    );
    assert!(
        raw.execute(label, &[&new_id(), &w2.workspace.id.0, &project, &None::<Uuid>, &"x"]).await.is_err(),
        "a label in another workspace than its project's"
    );
    raw.execute(label, &[&new_id(), &ws1, &project, &Some(parent), &"child"]).await.unwrap();
}

/// A database created before migration 0012 has projects with members that are a project, a workspace, a user and a role, and
/// settings rows that are a sort order alone. `migrate` gives each member an id of its own and the pin's empty settings, each settings
/// row the workspace of its project, and applying it a second time changes nothing. A pinned regression for the upgrade path of a
/// database that has data; every other test here starts from an empty one.
#[tokio::test]
async fn migration_0012_upgrades_a_database_that_has_projects_members_and_settings() {
    let container = Postgres::default().start().await.expect("start postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let conn = Arc::new(PoolConnector::from_url(&url, 4).unwrap());
    let raw = conn.raw().await.unwrap();
    for sql in [
        thrust_adapters_postgres::MIGRATION_0001,
        thrust_adapters_postgres::MIGRATION_0002,
        thrust_adapters_postgres::MIGRATION_0003,
        thrust_adapters_postgres::MIGRATION_0004,
        thrust_adapters_postgres::MIGRATION_0005,
        thrust_adapters_postgres::MIGRATION_0006,
        thrust_adapters_postgres::MIGRATION_0007,
        thrust_adapters_postgres::MIGRATION_0008,
        thrust_adapters_postgres::MIGRATION_0009,
        thrust_adapters_postgres::MIGRATION_0010,
        thrust_adapters_postgres::MIGRATION_0011,
    ] {
        raw.batch_execute(sql).await.unwrap();
    }
    let (ws, project, u1, u2) = (new_id(), new_id(), new_id(), new_id());
    for u in [u1, u2] {
        raw.execute("INSERT INTO users (id) VALUES ($1)", &[&u]).await.unwrap();
    }
    raw.execute("INSERT INTO workspaces (id, slug, name) VALUES ($1, 'legacy-members', 'Legacy')", &[&ws])
        .await
        .unwrap();
    raw.execute("INSERT INTO workspace_members (workspace_id, user_id) VALUES ($1, $2), ($1, $3)", &[&ws, &u1, &u2])
        .await
        .unwrap();
    raw.execute(
        "INSERT INTO projects (id, workspace_id, name, identifier) VALUES ($1, $2, 'Legacy project', 'LEG')",
        &[&project, &ws],
    )
    .await
    .unwrap();
    for u in [u1, u2] {
        raw.execute(
            "INSERT INTO project_members (project_id, workspace_id, user_id, role) VALUES ($1, $2, $3, 'member')",
            &[&project, &ws, &u],
        )
        .await
        .unwrap();
    }
    raw.execute(
        "INSERT INTO project_user_properties (project_id, user_id, sort_order) VALUES ($1, $2, 7.5)",
        &[&project, &u1],
    )
    .await
    .unwrap();

    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    async fn state(raw: &tokio_postgres::Client) -> (Vec<(Uuid, Uuid)>, Vec<(Uuid, Uuid, f64, serde_json::Value)>) {
        let members = raw.query("SELECT user_id, id FROM project_members ORDER BY user_id", &[]).await.unwrap();
        let props = raw
            .query(
                "SELECT user_id, workspace_id, sort_order, filters FROM project_user_properties ORDER BY user_id",
                &[],
            )
            .await
            .unwrap();
        (
            members.iter().map(|r| (r.get(0), r.get(1))).collect(),
            props.iter().map(|r| (r.get(0), r.get(1), r.get(2), r.get(3))).collect(),
        )
    }
    let first = state(&raw).await;
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    assert_eq!(first, state(&raw).await, "a second run changed the members or their settings");
    assert_ne!(first.0[0].1, first.0[1].1, "two members share an id");
    assert_eq!(
        first.1,
        vec![(u1, ws, 7.5, serde_json::json!({}))],
        "a settings row has the workspace of its project and the empty object"
    );

    drop(raw);
    let db = PgDatabase::new(conn);
    let scope = WorkspaceScope::mint(WorkspaceId(ws), UserId(u1), Role::Member);
    let d = db.project_detail(&scope, ProjectId(project)).await.unwrap().unwrap();
    assert_eq!(
        (d.listing.role, d.listing.sort_order, d.members.len(), d.next_sequence),
        (Some(Role::Member), Some(7.5), 2, 1)
    );
    assert_eq!(
        (d.listing.project.description.as_str(), d.listing.project.timezone.as_str(), d.listing.project.archive_in),
        ("", "UTC", 0)
    );
    // a member who has no settings row yet gets one on their first ask, and the one that was there is kept
    let m = db.project_member(&scope, ProjectId(project)).await.unwrap().unwrap();
    assert_eq!(
        (m.member.role, m.member.sort_order, m.member.view_props.clone()),
        (Role::Member, 65535.0, serde_json::json!({}))
    );
}

/// Two creations racing for one name cannot both win, and the loser leaves nothing behind: the unique index decides, not a
/// check-then-insert. 16 projects with one name and sixteen keys are made from as many tasks; exactly one exists, with its own member,
/// settings row and states, and no row of a loser is left.
#[tokio::test]
async fn creations_racing_for_one_project_name_make_exactly_one_project_with_nothing_left_behind() {
    let env = env().await;
    let owner = user_without_login(&env).await;
    let w = workspace_for(owner, "project-race");
    env.db.insert_workspace(&w).await.unwrap();
    let scope = WorkspaceScope::mint(w.workspace.id, owner, Role::Admin);
    let mut handles = Vec::new();
    for i in 0..16 {
        let db = env.db.clone();
        let p = project_for(w.workspace.id, owner, "One name", &format!("K{i}"));
        handles.push(tokio::spawn(async move { db.insert_project(&scope, &p).await }));
    }
    let mut won = 0;
    for h in handles {
        match h.await.unwrap() {
            Ok(()) => won += 1,
            Err(DbError::Conflict(_)) => {}
            Err(e) => panic!("a loser was refused with {e:?}, not a conflict"),
        }
    }
    assert_eq!(won, 1);
    let raw = env.conn.raw().await.unwrap();
    for (table, want) in [("projects", 1i64), ("project_members", 1), ("project_user_properties", 1), ("states", 2)] {
        let n: i64 = raw
            .query_one(&format!("SELECT count(*) FROM {table} WHERE workspace_id = $1"), &[&w.workspace.id.0])
            .await
            .unwrap()
            .get(0);
        assert_eq!(n, want, "{table}: a loser left rows behind, or the winner is incomplete");
    }
}

/// Issue #58, migration 0015: a deleted project stays as a row, marked with the time, and leaves every read; its name and key are free
/// again, while two live projects still cannot share one; its states and members are kept; and a second deletion finds nothing. The
/// memory adapter is held to the same by `thrust-core/tests/project_v1.rs`.
#[tokio::test]
async fn a_deleted_project_leaves_every_read_and_frees_its_name_and_key() {
    let env = env().await;
    let owner = user_without_login(&env).await;
    let w = workspace_for(owner, "project-delete");
    env.db.insert_workspace(&w).await.unwrap();
    let (ws, slug) = (w.workspace.id, w.workspace.slug.clone());
    let scope = WorkspaceScope::mint(ws, owner, Role::Admin);

    let doomed = project_for(ws, owner, "Doomed", "DMD");
    let id = doomed.project.id;
    env.db.insert_project(&scope, &doomed).await.unwrap();
    // while it is live the name and the key are taken, which the partial unique indexes still say
    for (name, key) in [("Doomed", "DM2"), ("Doomed Two", "DMD")] {
        let got = env.db.insert_project(&scope, &project_for(ws, owner, name, key)).await;
        assert!(matches!(got, Err(DbError::Conflict(_))), "{name} {key}: {got:?}");
    }
    assert!(env.db.project_detail(&scope, id).await.unwrap().is_some());

    let at = Utc.timestamp_opt(1_800_000_500, 0).unwrap();
    assert!(env.db.delete_project(&scope, id, at).await.unwrap());

    // the row, its states and its members are kept, and the row says when
    let raw = env.conn.raw().await.unwrap();
    let count = |table: &'static str| {
        let raw = &raw;
        async move {
            let q = format!("SELECT count(*) FROM {table} WHERE project_id = $1");
            raw.query_one(&q, &[&id.0]).await.unwrap().get::<_, i64>(0)
        }
    };
    let when: Option<Timestamp> =
        raw.query_one("SELECT deleted_at FROM projects WHERE id = $1", &[&id.0]).await.unwrap().get(0);
    assert_eq!(when, Some(at));
    assert_eq!((count("states").await, count("project_members").await), (2, 1));

    // and every read leaves it out
    assert!(matches!(env.db.membership(&slug, id, owner).await.unwrap(), Membership::NoSuchProject));
    assert!(env.db.project_detail(&scope, id).await.unwrap().is_none());
    assert!(env.db.project_member(&scope, id).await.unwrap().is_none());
    assert!(!env.db.update_project(&scope, &doomed.project).await.unwrap());
    let ids = |l: Vec<ProjectListing>| l.into_iter().map(|x| x.project.id).collect::<Vec<_>>();
    assert!(!ids(env.db.workspace_projects(&scope).await.unwrap()).contains(&id));
    assert!(!ids(env.db.visible_projects(&scope).await.unwrap()).contains(&id));
    assert!(!env.db.project_roles(&scope).await.unwrap().iter().any(|(p, _)| *p == id));
    assert!(!env.db.project_stats(&scope, None).await.unwrap().iter().any(|s| s.id == id));
    assert!(env.db.workspace_states(&scope).await.unwrap().iter().all(|s| s.project_id != id));

    // its name and its key are free, and a project can be made with them
    let c = env.db.project_clashes(&scope, Some("Doomed"), Some("DMD"), None).await.unwrap();
    assert_eq!((c.name, c.identifier), (false, false));
    env.db.insert_project(&scope, &project_for(ws, owner, "Doomed", "DMD")).await.unwrap();
    // and the second deletion of the first finds no project
    assert!(!env.db.delete_project(&scope, id, at).await.unwrap());
}

/// Issue #58: `visible_projects` is Plane's v1 rule, whatever the caller's role in the workspace: the projects they are a member of and
/// the public ones. A workspace GUEST who is in no project sees the public one (the internal API's `workspace_projects` shows them
/// nothing), nobody outside a secret project sees it, and a caller's own role and place come with each.
#[tokio::test]
async fn visible_projects_are_the_callers_own_and_the_public_ones_whatever_their_workspace_role() {
    let env = env().await;
    let owner = user_without_login(&env).await;
    let guest = user_without_login(&env).await;
    let w = workspace_for(owner, "project-visible");
    env.db.insert_workspace(&w).await.unwrap();
    let ws = w.workspace.id;
    // the guest is in the workspace and in no project
    env.db
        .seed(
            &SystemScope::mint(),
            &SeedFixture {
                workspace_members: vec![SeedWorkspaceMember { workspace_id: ws, user_id: guest }],
                workspace_roles: vec![SeedWorkspaceRole { workspace_id: ws, user_id: guest, role: Role::Guest }],
                ..SeedFixture::default()
            },
        )
        .await
        .unwrap();
    let owner_scope = WorkspaceScope::mint(ws, owner, Role::Admin);
    let guest_scope = WorkspaceScope::mint(ws, guest, Role::Guest);

    let mut hidden = project_for(ws, owner, "Hidden", "HID");
    hidden.project.network = ProjectNetwork::Secret;
    let shown = project_for(ws, owner, "Shown", "SHW");
    let (hidden_id, shown_id) = (hidden.project.id, shown.project.id);
    env.db.insert_project(&owner_scope, &hidden).await.unwrap();
    env.db.insert_project(&owner_scope, &shown).await.unwrap();

    let seen = |l: Vec<ProjectListing>| -> Vec<(ProjectId, Option<Role>)> {
        let mut v: Vec<_> = l.into_iter().map(|x| (x.project.id, x.role)).collect();
        v.sort_by_key(|(id, _)| id.0);
        v
    };
    let mut both = vec![(hidden_id, Some(Role::Admin)), (shown_id, Some(Role::Admin))];
    both.sort_by_key(|(id, _)| id.0);
    assert_eq!(seen(env.db.visible_projects(&owner_scope).await.unwrap()), both);
    // the guest: the public one, as a non-member (no role), and not the secret one
    assert_eq!(seen(env.db.visible_projects(&guest_scope).await.unwrap()), vec![(shown_id, None)]);
    // the internal API's listing says the opposite of the guest's: nothing, as a guest sees only the projects they are in
    assert!(env.db.workspace_projects(&guest_scope).await.unwrap().is_empty());
}

/// Issue #58, migration 0015: the two unique indexes of a project's name and key hold among the projects that are not deleted. A
/// database from before 0015 has them plain; migrating replaces them, and does it ONCE: a restart (which runs every file again, 0012
/// among them) leaves the index as it is, not made again (its object id is the same), so a start does not rebuild a unique index and
/// cannot fail on a deleted project's reused name.
#[tokio::test]
async fn migration_0015_makes_the_unique_indexes_partial_once_and_a_restart_keeps_them() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let indexes = ["projects_workspace_identifier_key", "projects_workspace_name_key"];
    let def = |name: &'static str| {
        let raw = &raw;
        async move {
            raw.query_one(
                "SELECT indexdef FROM pg_indexes WHERE schemaname = 'public' AND tablename = 'projects' AND indexname = $1",
                &[&name],
            )
            .await
            .unwrap()
            .get::<_, String>(0)
        }
    };
    let oid = |name: &'static str| {
        let raw = &raw;
        async move {
            raw.query_one("SELECT oid::int8 FROM pg_class WHERE relname = $1", &[&name]).await.unwrap().get::<_, i64>(0)
        }
    };
    for name in indexes {
        assert!(def(name).await.contains("deleted_at IS NULL"), "{name} is not partial after migrating");
    }

    // a database from before 0015: the two indexes plain
    raw.batch_execute(
        "DROP INDEX projects_workspace_identifier_key; DROP INDEX projects_workspace_name_key; \
         CREATE UNIQUE INDEX projects_workspace_identifier_key ON projects (workspace_id, identifier); \
         CREATE UNIQUE INDEX projects_workspace_name_key ON projects (workspace_id, name);",
    )
    .await
    .unwrap();
    for name in indexes {
        assert!(!def(name).await.contains("deleted_at"), "{name} should be plain before 0015");
    }
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    for name in indexes {
        assert!(def(name).await.contains("deleted_at IS NULL"), "{name} was not replaced");
    }

    // a restart: every file runs again, and the indexes are the same objects
    let before = [oid(indexes[0]).await, oid(indexes[1]).await];
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    raw.batch_execute(thrust_adapters_postgres::MIGRATION_0012).await.unwrap();
    assert_eq!([oid(indexes[0]).await, oid(indexes[1]).await], before, "a restart made an index again");
    for name in indexes {
        assert!(def(name).await.contains("deleted_at IS NULL"), "{name} lost its condition after a restart");
    }
}

// --- states (issue #58, migration 0015) ---------------------------------------------------------------------------------------
// The memory adapter is held to the same answers by `tests/projects.rs` in its own crate.

/// A project of its own with the two states `project_for` gives it (Backlog at 15000, the default, and the intake Triage at 65000).
async fn project_with_states(env: &Env, tag: &str) -> (WorkspaceId, ProjectId, UserId, Scope) {
    let owner = user_without_login(env).await;
    let w = workspace_for(owner, tag);
    env.db.insert_workspace(&w).await.unwrap();
    let ws = w.workspace.id;
    let p = project_for(ws, owner, "States", "STA");
    env.db.insert_project(&WorkspaceScope::mint(ws, owner, Role::Admin), &p).await.unwrap();
    (ws, p.project.id, owner, Scope::mint(ws, p.project.id, owner))
}

fn state_for(
    ws: WorkspaceId,
    project: ProjectId,
    owner: UserId,
    name: &str,
    sequence: f64,
    is_default: bool,
) -> StateRow {
    let now = Utc.timestamp_opt(1_800_000_700, 0).unwrap();
    StateRow {
        id: new_id(),
        workspace_id: ws,
        project_id: project,
        name: name.into(),
        description: String::new(),
        color: "#123456".into(),
        sequence,
        group: StateGroup::Started,
        is_default,
        created_by: Some(owner),
        created_at: now,
        updated_at: now,
        external_source: None,
        external_id: None,
        updated_by: None,
    }
}

async fn default_names(env: &Env, s: &Scope) -> Vec<String> {
    env.db.project_states(s).await.unwrap().into_iter().filter(|x| x.is_default).map(|x| x.name).collect()
}

/// A new state goes after the greatest live one that is not the intake state, by 15000, whatever it asked for; the default moves to a
/// state that becomes it, in the same transaction (the one-default index would refuse otherwise), and a change that finds no state, or
/// is refused, leaves the default where it was.
#[tokio::test]
async fn a_new_state_is_placed_as_plane_places_it_and_the_default_moves_with_a_state() {
    let env = env().await;
    let (ws, project, owner, s) = project_with_states(&env, "state-place").await;
    let new = |name: &str, sequence: f64, default: bool| state_for(ws, project, owner, name, sequence, default);

    // Backlog is at 15000, and the intake state at 65000 does not count
    let review = env.db.insert_state(&s, &new("Review", 1.0, false)).await.unwrap();
    assert_eq!(review.sequence, 30000.0);
    assert_eq!(env.db.project_states(&s).await.unwrap().iter().find(|x| x.id == review.id), Some(&review));
    let qa = env.db.insert_state(&s, &new("QA", 9e9, false)).await.unwrap();
    assert_eq!(qa.sequence, 45000.0);
    // a deleted state is not counted
    assert!(env.db.delete_state(&s, qa.id, Utc.timestamp_opt(1_800_000_800, 0).unwrap()).await.unwrap());
    assert_eq!(env.db.insert_state(&s, &new("Ops", 0.0, false)).await.unwrap().sequence, 45000.0);

    // the default
    assert_eq!(default_names(&env, &s).await, ["Backlog"]);
    env.db.insert_state(&s, &new("Plan", 0.0, true)).await.unwrap();
    assert_eq!(default_names(&env, &s).await, ["Plan"]);
    let backlog = env.db.project_states(&s).await.unwrap().into_iter().find(|x| x.name == "Backlog").unwrap();
    assert!(env.db.update_state(&s, &StateRow { is_default: true, ..backlog.clone() }).await.unwrap());
    assert_eq!(default_names(&env, &s).await, ["Backlog"]);
    // a change that finds no state takes the default from no one
    let nowhere = new("Nowhere", 0.0, true);
    assert!(!env.db.update_state(&s, &nowhere).await.unwrap());
    assert_eq!(default_names(&env, &s).await, ["Backlog"]);
    // and one that is refused gives the default back: the name is a live state's, and the default it would take stays
    let refused = StateRow {
        name: "Review".into(),
        is_default: true,
        ..env.db.project_states(&s).await.unwrap().into_iter().find(|x| x.name == "Plan").unwrap()
    };
    let got = env.db.update_state(&s, &refused).await;
    assert!(matches!(got, Err(DbError::Conflict(_))), "{got:?}");
    assert_eq!(default_names(&env, &s).await, ["Backlog"]);
}

/// A state's name is unique among the live states of its project: another case is another name, another project's states do not
/// count, and a deleted state's name is free.
#[tokio::test]
async fn a_state_name_is_unique_among_the_live_states_of_its_project() {
    let env = env().await;
    let (ws, project, owner, s) = project_with_states(&env, "state-names").await;
    let new = |name: &str| state_for(ws, project, owner, name, 0.0, false);
    let taken = |r: Result<StateRow, DbError>| matches!(r, Err(DbError::Conflict(c)) if c.contains("states_project_id_name_key"));
    let first = env.db.insert_state(&s, &new("Review")).await.unwrap();
    assert!(taken(env.db.insert_state(&s, &new("Review")).await));
    env.db.insert_state(&s, &new("review")).await.unwrap();
    // another project may have a state of the same name
    let (ws2, project2, owner2, s2) = project_with_states(&env, "state-names-2").await;
    env.db.insert_state(&s2, &state_for(ws2, project2, owner2, "Review", 0.0, false)).await.unwrap();
    // a deleted state's name is free, and its id is not
    assert!(env.db.delete_state(&s, first.id, Utc.timestamp_opt(1_800_000_800, 0).unwrap()).await.unwrap());
    env.db.insert_state(&s, &new("Review")).await.unwrap();
    // a change cannot take a live name
    let backlog = env.db.project_states(&s).await.unwrap().into_iter().find(|x| x.name == "Backlog").unwrap();
    let got = env.db.update_state(&s, &StateRow { name: "review".into(), ..backlog.clone() }).await;
    assert!(matches!(got, Err(DbError::Conflict(_))), "{got:?}");
    assert!(env.db.update_state(&s, &backlog).await.unwrap(), "and its own name is not another's");
}

/// A deleted state is marked and not removed: the lists leave it out, a second deletion and a state of another project are not there,
/// and a work item that names it still reads its group. What counts as a work item in a state is the pin's `Issue.objects`: a draft,
/// an archived one and a deleted one do not, and nothing in an archived project does.
#[tokio::test]
async fn a_deleted_state_leaves_the_lists_and_work_items_in_a_state_count_as_the_pin_counts_them() {
    let env = env().await;
    let (ws, project, owner, s) = project_with_states(&env, "state-delete").await;
    let at = Utc.timestamp_opt(1_800_000_800, 0).unwrap();
    let mut states = Vec::new();
    for name in ["Live", "Draft", "Archived", "Deleted", "Empty"] {
        states.push(env.db.insert_state(&s, &state_for(ws, project, owner, name, 0.0, false)).await.unwrap());
    }
    // a work item in each of the first four, and then the draft, the archived and the deleted marked as such
    let mut items = Vec::new();
    for st in &states[..4] {
        let attrs = WorkItemAttrs { state_id: Some(st.id), ..WorkItemAttrs::bare(at) };
        let mut tx = env.db.begin(&s).await.unwrap();
        let a = tx.allocate(&s).await.unwrap();
        items.push(insert_with_attrs(&mut tx, s, a, &attrs).await.unwrap());
        tx.commit().await.unwrap();
    }
    let raw = env.conn.raw().await.unwrap();
    raw.execute("UPDATE work_items SET is_draft = true WHERE id = $1", &[&items[1].0]).await.unwrap();
    raw.execute("UPDATE work_items SET archived_at = now() WHERE id = $1", &[&items[2].0]).await.unwrap();
    raw.execute("UPDATE work_items SET deleted_at = now() WHERE id = $1", &[&items[3].0]).await.unwrap();
    for (st, counts) in states.iter().zip([true, false, false, false, false]) {
        assert_eq!(env.db.state_has_work_items(&s, st.id).await.unwrap(), counts, "{}", st.name);
    }
    assert!(!env.db.state_has_work_items(&s, new_id()).await.unwrap());

    // the live item's state is deleted, and the item still reads its group
    let live = &states[0];
    assert_eq!(env.db.work_item_row(&s, items[0]).await.unwrap().unwrap().state_group, Some(StateGroup::Started));
    assert!(env.db.delete_state(&s, live.id, at).await.unwrap());
    assert!(env.db.project_states(&s).await.unwrap().iter().all(|x| x.id != live.id));
    let admin = WorkspaceScope::mint(ws, owner, Role::Admin);
    assert!(env.db.workspace_states(&admin).await.unwrap().iter().all(|x| x.id != live.id));
    let row = env.db.work_item_row(&s, items[0]).await.unwrap().unwrap();
    assert_eq!((row.attrs.state_id, row.state_group), (Some(live.id), Some(StateGroup::Started)));
    // the row is kept, with the time
    let when: Option<Timestamp> =
        raw.query_one("SELECT deleted_at FROM states WHERE id = $1", &[&live.id]).await.unwrap().get(0);
    assert_eq!(when, Some(at));

    // the intake state can be deleted, and then there is none; a second deletion and another project's state are not there
    let triage = env.db.triage_state(&s).await.unwrap().unwrap();
    assert!(env.db.delete_state(&s, triage.id, at).await.unwrap());
    assert!(env.db.triage_state(&s).await.unwrap().is_none());
    assert!(!env.db.delete_state(&s, live.id, at).await.unwrap());
    let (_, _, _, other) = project_with_states(&env, "state-delete-2").await;
    let others_state = env.db.project_states(&other).await.unwrap().remove(0);
    assert!(!env.db.delete_state(&s, others_state.id, at).await.unwrap());

    // and once the project is archived none of its work items counts, nor is the project archived before
    assert!(!env.db.project_is_archived(&s).await.unwrap());
    let mut detail = env.db.project_detail(&admin, project).await.unwrap().unwrap().listing.project;
    detail.archived_at = Some(at);
    assert!(env.db.update_project(&admin, &detail).await.unwrap());
    assert!(env.db.project_is_archived(&s).await.unwrap());
    assert!(!env.db.state_has_work_items(&s, states[1].id).await.unwrap());
    assert!(!env.db.state_has_work_items(&s, states[0].id).await.unwrap());
}

/// Issue #58, migration 0015: the name of a state is unique among the live ones by an index that took the place of 0012's table
/// constraint, and a start does it once. A database from before 0015 has the constraint; migrating replaces it with the index of the
/// same name, and a restart (0012 again, 0015 again) leaves that index as it is, the same object, with no constraint made again.
#[tokio::test]
async fn migration_0015_replaces_the_state_name_constraint_once_and_a_restart_keeps_the_index() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let name = "states_project_id_name_key";
    let index_def = || {
        let raw = &raw;
        async move {
            raw.query_one("SELECT indexdef FROM pg_indexes WHERE schemaname = 'public' AND tablename = 'states' AND indexname = $1", &[&name])
                .await
                .unwrap()
                .get::<_, String>(0)
        }
    };
    let constraints = || {
        let raw = &raw;
        async move {
            raw.query_one("SELECT count(*) FROM pg_constraint WHERE conname = $1", &[&name])
                .await
                .unwrap()
                .get::<_, i64>(0)
        }
    };
    let oid = || {
        let raw = &raw;
        async move {
            raw.query_one("SELECT oid::int8 FROM pg_class WHERE relname = $1", &[&name]).await.unwrap().get::<_, i64>(0)
        }
    };
    assert!(index_def().await.contains("deleted_at IS NULL"));
    assert_eq!(constraints().await, 0, "after migrating there is the index and no constraint");

    // a database from before 0015: the table constraint 0012 made
    raw.batch_execute(&format!(
        "DROP INDEX {name}; ALTER TABLE states ADD CONSTRAINT {name} UNIQUE (project_id, name);"
    ))
    .await
    .unwrap();
    assert_eq!(constraints().await, 1);
    assert!(!index_def().await.contains("deleted_at"));
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    assert!(index_def().await.contains("deleted_at IS NULL"), "the constraint was not replaced");
    assert_eq!(constraints().await, 0);

    // a restart makes nothing again
    let before = oid().await;
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    raw.batch_execute(thrust_adapters_postgres::MIGRATION_0012).await.unwrap();
    assert_eq!(oid().await, before, "a restart made the index again");
    assert_eq!(constraints().await, 0, "a restart made the constraint again");
}

/// `project_stats` counts a project's live work items (not a draft, an archived or a deleted one) and every member row, for each
/// project of the workspace. The model cannot reach this: the slice's `create` never makes a draft or an archived item, so the
/// clauses that leave them out are held here, the way the memory adapter's `tests/projects.rs` holds them.
#[tokio::test]
async fn project_counts_leave_out_drafts_archived_and_deleted_work_items_and_count_every_member() {
    let env = env().await;
    let owner = user_without_login(&env).await;
    let w = workspace_for(owner, "project-counts");
    env.db.insert_workspace(&w).await.unwrap();
    let ws = w.workspace.id;
    let scope_of = |id: ProjectId| Scope::mint(ws, id, owner);
    let (one, two) = (project_for(ws, owner, "Counted", "CNT"), project_for(ws, owner, "Other", "OTH"));
    let admin = WorkspaceScope::mint(ws, owner, Role::Admin);
    env.db.insert_project(&admin, &one).await.unwrap();
    env.db.insert_project(&admin, &two).await.unwrap();
    let (p1, p2) = (one.project.id, two.project.id);

    let gone = Utc.timestamp_opt(1_800_000_500, 0).unwrap();
    // project one: two live items, a draft and a deleted one; project two: an archived one
    for (project, draft, archived, deleted) in [
        (p1, false, false, false),
        (p1, true, false, false),
        (p1, false, false, false),
        (p1, false, false, true),
        (p2, false, true, false),
    ] {
        let s = scope_of(project);
        let mut tx = env.db.begin(&s).await.unwrap();
        let a = tx.allocate(&s).await.unwrap();
        let mut w = item(ws, project, a, "counted", None);
        w.is_draft = draft;
        w.archived_at = archived.then_some(gone);
        tx.insert_work_item(&s, &w).await.unwrap();
        if deleted {
            assert!(tx.soft_delete_work_item(&s, w.id, gone).await.unwrap());
        }
        tx.commit().await.unwrap();
    }
    let counts = |stats: Vec<ProjectStat>| {
        let mut v: Vec<(ProjectId, i64, i64)> = stats.iter().map(|s| (s.id, s.total_issues, s.total_members)).collect();
        v.sort_by_key(|s| s.0 .0);
        v
    };
    let mut want = vec![(p1, 2, 1), (p2, 0, 1)];
    want.sort_by_key(|s| s.0 .0);
    assert_eq!(counts(env.db.project_stats(&admin, None).await.unwrap()), want);
    assert_eq!(counts(env.db.project_stats(&admin, Some(&[p1])).await.unwrap()), vec![(p1, 2, 1)]);
}

/// The work items' compat columns and their two join tables, and the constraints that keep them well formed: a priority from the
/// five, dates in order, a state, a person and a label that exist, a person and a label once, a project of its own workspace.
#[tokio::test]
async fn the_work_item_tables_have_the_columns_they_say_and_the_constraints_that_keep_them_well_formed() {
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
    let ts = "timestamp with time zone";
    let (text, uuid, boolean, bigint, date) = ("text", "uuid", "boolean", "bigint", "date");
    let pairs = |v: Vec<(&str, &str)>| v.into_iter().map(|(n, t)| (n.to_string(), t.to_string())).collect::<Vec<_>>();
    assert_eq!(
        columns("work_items").await,
        pairs(vec![
            ("archived_at", ts),
            ("completed_at", ts),
            ("created_at", ts),
            ("created_by", uuid),
            ("deleted_at", ts),
            ("description", text),
            ("description_html", text),
            ("external_id", text),
            ("external_source", text),
            ("id", uuid),
            ("is_draft", boolean),
            ("name", text),
            ("parent_id", uuid),
            ("point", "integer"),
            ("priority", text),
            ("project_id", uuid),
            ("sequence_id", bigint),
            ("sort_order", bigint),
            ("start_date", date),
            ("state_id", uuid),
            ("target_date", date),
            ("updated_at", ts),
            ("updated_by", uuid),
            ("workspace_id", uuid),
        ])
    );
    assert_eq!(
        columns("work_item_assignees").await,
        pairs(vec![
            ("created_at", ts),
            ("project_id", uuid),
            ("user_id", uuid),
            ("work_item_id", uuid),
            ("workspace_id", uuid)
        ])
    );
    assert_eq!(
        columns("work_item_labels").await,
        pairs(vec![
            ("created_at", ts),
            ("label_id", uuid),
            ("project_id", uuid),
            ("work_item_id", uuid),
            ("workspace_id", uuid)
        ])
    );

    let owner = user_without_login(&env).await;
    let (w1, w2) = (workspace_for(owner, "items-one"), workspace_for(owner, "items-two"));
    env.db.insert_workspace(&w1).await.unwrap();
    env.db.insert_workspace(&w2).await.unwrap();
    let first = project_for(w1.workspace.id, owner, "First", "FST");
    env.db.insert_project(&WorkspaceScope::mint(w1.workspace.id, owner, Role::Admin), &first).await.unwrap();
    let second = project_for(w2.workspace.id, owner, "Second", "SND");
    env.db.insert_project(&WorkspaceScope::mint(w2.workspace.id, owner, Role::Admin), &second).await.unwrap();
    let (ws, project) = (w1.workspace.id, first.project.id);
    let s = scope(ws, project);
    let mut tx = env.db.begin(&s).await.unwrap();
    let a = tx.allocate(&s).await.unwrap();
    let w = item(ws, project, a, "constrained", None);
    tx.insert_work_item(&s, &w).await.unwrap();
    tx.commit().await.unwrap();
    let id = w.id.0;
    // what a work item has when nothing says otherwise: the defaults a slice-made item reads as
    let row = raw.query_one("SELECT priority, description_html, state_id IS NULL, start_date IS NULL, updated_at IS NOT NULL FROM work_items WHERE id = $1", &[&id]).await.unwrap();
    assert_eq!(
        (
            row.get::<_, String>(0),
            row.get::<_, String>(1),
            row.get::<_, bool>(2),
            row.get::<_, bool>(3),
            row.get::<_, bool>(4)
        ),
        ("none".into(), "<p></p>".into(), true, true, true)
    );
    // a priority from the five
    for (priority, ok) in [
        ("urgent", true),
        ("high", true),
        ("medium", true),
        ("low", true),
        ("none", true),
        ("huge", false),
        ("", false),
        ("High", false),
    ] {
        assert_eq!(
            raw.execute("UPDATE work_items SET priority = $2 WHERE id = $1", &[&id, &priority]).await.is_ok(),
            ok,
            "{priority:?}"
        );
    }
    // a start that is not after the target, or either missing
    for (set, ok) in [
        ("start_date = '2026-11-01', target_date = '2026-11-01'", true),
        ("start_date = '2026-11-01', target_date = '2026-11-02'", true),
        ("start_date = '2026-11-03', target_date = '2026-11-02'", false),
        ("start_date = '2026-11-03', target_date = NULL", true),
        ("start_date = NULL, target_date = '2026-01-01'", true),
    ] {
        assert_eq!(
            raw.execute(&format!("UPDATE work_items SET {set} WHERE id = $1"), &[&id]).await.is_ok(),
            ok,
            "{set}"
        );
    }
    // a state that exists, whatever project it is of (the foreign key is to the state)
    let state = first.states[0].id;
    assert!(raw.execute("UPDATE work_items SET state_id = $2 WHERE id = $1", &[&id, &state]).await.is_ok());
    assert!(
        raw.execute("UPDATE work_items SET state_id = $2 WHERE id = $1", &[&id, &second.states[0].id]).await.is_ok(),
        "the key is to the state, as the core's rule is"
    );
    assert!(
        raw.execute("UPDATE work_items SET state_id = $2 WHERE id = $1", &[&id, &new_id()]).await.is_err(),
        "a state that is not there"
    );
    // who changed it is a user
    assert!(raw.execute("UPDATE work_items SET updated_by = $2 WHERE id = $1", &[&id, &owner.0]).await.is_ok());
    assert!(raw.execute("UPDATE work_items SET updated_by = $2 WHERE id = $1", &[&id, &new_id()]).await.is_err());
    // a person, once, a user, in the work item's own workspace and project
    let assign = "INSERT INTO work_item_assignees (work_item_id, workspace_id, project_id, user_id, created_at) VALUES ($1, $2, $3, $4, now())";
    raw.execute(assign, &[&id, &ws.0, &project.0, &owner.0]).await.unwrap();
    assert!(raw.execute(assign, &[&id, &ws.0, &project.0, &owner.0]).await.is_err(), "a person twice");
    assert!(raw.execute(assign, &[&id, &ws.0, &project.0, &new_id()]).await.is_err(), "a person who is no user");
    assert!(
        raw.execute(assign, &[&id, &w2.workspace.id.0, &project.0, &owner.0]).await.is_err(),
        "another workspace than the project's"
    );
    assert!(
        raw.execute(assign, &[&new_id(), &ws.0, &project.0, &owner.0]).await.is_err(),
        "a work item that is not there"
    );
    // a label, once, that exists
    let label_id = new_id();
    raw.execute("INSERT INTO labels (id, workspace_id, project_id, name, created_at, updated_at) VALUES ($1, $2, $3, 'bug', now(), now())", &[&label_id, &ws.0, &project.0]).await.unwrap();
    let tag = "INSERT INTO work_item_labels (work_item_id, workspace_id, project_id, label_id, created_at) VALUES ($1, $2, $3, $4, now())";
    raw.execute(tag, &[&id, &ws.0, &project.0, &label_id]).await.unwrap();
    assert!(raw.execute(tag, &[&id, &ws.0, &project.0, &label_id]).await.is_err(), "a label twice");
    assert!(raw.execute(tag, &[&id, &ws.0, &project.0, &new_id()]).await.is_err(), "a label that is not there");
    // a point from 0 to 12, or none (migration 0015)
    for (point, ok) in [(Some(0), true), (Some(12), true), (None, true), (Some(13), false), (Some(-1), false)] {
        assert_eq!(
            raw.execute("UPDATE work_items SET point = $2 WHERE id = $1", &[&id, &point]).await.is_ok(),
            ok,
            "{point:?}"
        );
    }
    // a parent is a work item that is there (itself too, as the pin allows), or none
    let other = {
        let mut tx = env.db.begin(&s).await.unwrap();
        let a = tx.allocate(&s).await.unwrap();
        let w = item(ws, project, a, "another", None);
        tx.insert_work_item(&s, &w).await.unwrap();
        tx.commit().await.unwrap();
        w.id.0
    };
    for (parent, ok) in [(Some(other), true), (Some(id), true), (None, true), (Some(new_id()), false)] {
        assert_eq!(
            raw.execute("UPDATE work_items SET parent_id = $2 WHERE id = $1", &[&id, &parent]).await.is_ok(),
            ok,
            "{parent:?}"
        );
    }
    // who made an item is optional (migration 0015)
    assert!(raw.execute("UPDATE work_items SET created_by = NULL WHERE id = $1", &[&id]).await.is_ok());
}

/// A database created before migration 0013 has work items with a name, a number and a place. `migrate` gives each the pin's
/// defaults (no state, priority `none`, no dates, the empty paragraph) and the time it was made as the time it was changed, and
/// applying it a second time changes nothing. A pinned regression for the upgrade path of a database that has data; every other
/// test here starts from an empty one.
#[tokio::test]
async fn migration_0013_upgrades_a_database_that_has_work_items() {
    let container = Postgres::default().start().await.expect("start postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let conn = Arc::new(PoolConnector::from_url(&url, 4).unwrap());
    let raw = conn.raw().await.unwrap();
    for sql in [
        thrust_adapters_postgres::MIGRATION_0001,
        thrust_adapters_postgres::MIGRATION_0002,
        thrust_adapters_postgres::MIGRATION_0003,
        thrust_adapters_postgres::MIGRATION_0004,
        thrust_adapters_postgres::MIGRATION_0005,
        thrust_adapters_postgres::MIGRATION_0006,
        thrust_adapters_postgres::MIGRATION_0007,
        thrust_adapters_postgres::MIGRATION_0008,
        thrust_adapters_postgres::MIGRATION_0009,
        thrust_adapters_postgres::MIGRATION_0010,
        thrust_adapters_postgres::MIGRATION_0011,
        thrust_adapters_postgres::MIGRATION_0012,
    ] {
        raw.batch_execute(sql).await.unwrap();
    }
    let (ws, project, user, item) = (new_id(), new_id(), new_id(), new_id());
    raw.execute("INSERT INTO users (id) VALUES ($1)", &[&user]).await.unwrap();
    raw.execute("INSERT INTO workspaces (id, slug, name) VALUES ($1, 'legacy-items', 'Legacy')", &[&ws]).await.unwrap();
    raw.execute("INSERT INTO workspace_members (workspace_id, user_id) VALUES ($1, $2)", &[&ws, &user]).await.unwrap();
    raw.execute(
        "INSERT INTO projects (id, workspace_id, name, identifier) VALUES ($1, $2, 'Legacy project', 'LEG')",
        &[&project, &ws],
    )
    .await
    .unwrap();
    raw.execute(
        "INSERT INTO project_members (project_id, workspace_id, user_id, role) VALUES ($1, $2, $3, 'member')",
        &[&project, &ws, &user],
    )
    .await
    .unwrap();
    let made = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    raw.execute(
        "INSERT INTO work_items (id, workspace_id, project_id, sequence_id, sort_order, name, created_by, created_at) VALUES ($1, $2, $3, 1, 10000, 'Legacy item', $4, $5)",
        &[&item, &ws, &project, &user, &made],
    )
    .await
    .unwrap();

    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    async fn state(raw: &tokio_postgres::Client) -> Vec<String> {
        let r = raw
            .query_one(
                "SELECT priority, description_html, state_id::text, start_date::text, target_date::text, completed_at::text, updated_at::text, updated_by::text \
                 FROM work_items",
                &[],
            )
            .await
            .unwrap();
        (0..8).map(|i| r.get::<_, Option<String>>(i).unwrap_or_else(|| "NULL".into())).collect()
    }
    let first = state(&raw).await;
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    assert_eq!(first, state(&raw).await, "a second run changed the work item");
    assert_eq!(&first[..6], ["none", "<p></p>", "NULL", "NULL", "NULL", "NULL"]);
    assert_eq!(
        first[6],
        raw.query_one("SELECT created_at::text FROM work_items", &[]).await.unwrap().get::<_, String>(0),
        "changed when it was made"
    );

    drop(raw);
    let db = PgDatabase::new(conn);
    let s = Scope::mint(WorkspaceId(ws), ProjectId(project), UserId(user));
    let row = db.work_item_row(&s, WorkItemId(item)).await.unwrap().expect("the item is there");
    assert_eq!(row.attrs, WorkItemAttrs::bare(made));
    assert_eq!((row.item.name.as_str(), row.state_group), ("Legacy item", None));
}

/// A work item of the scope's project with these attributes, in the open transaction.
async fn insert_with_attrs(
    tx: &mut Box<dyn Tx>,
    s: Scope,
    a: Allocation,
    attrs: &WorkItemAttrs,
) -> Result<WorkItemId, DbError> {
    let w = item(s.workspace_id(), s.project_id(), a, "with attributes", None);
    tx.insert_work_item(&s, &w).await?;
    tx.insert_work_item_attrs(&s, w.id, attrs).await?;
    Ok(w.id)
}

/// A work item and its attributes are one transaction: all of it commits, or none of it does, and what is refused (a state, a person
/// or a label that is not there; a start after a target) refuses all of it. The read gives back every attribute exactly as it was
/// written, with the people and the labels in id order and the group of the state.
#[tokio::test]
async fn a_work_item_and_its_attributes_are_one_transaction() {
    let env = env().await;
    let owner = user_without_login(&env).await;
    let w = workspace_for(owner, "item-attrs");
    env.db.insert_workspace(&w).await.unwrap();
    let ws = w.workspace.id;
    let new_project = project_for(ws, owner, "Attrs", "ATR");
    env.db.insert_project(&WorkspaceScope::mint(ws, owner, Role::Admin), &new_project).await.unwrap();
    let project = new_project.project.id;
    let s = Scope::mint(ws, project, owner);
    let (label, done) = (new_id(), new_id());
    {
        let raw = env.conn.raw().await.unwrap();
        raw.execute("INSERT INTO labels (id, workspace_id, project_id, name, created_at, updated_at) VALUES ($1, $2, $3, 'bug', now(), now())", &[&label, &ws.0, &project.0]).await.unwrap();
        // the project `project_for` makes has a default state and an intake state; one of the completed group is added
        raw.execute(
            "INSERT INTO states (id, workspace_id, project_id, name, color, sequence, state_group, is_default, created_at, updated_at) \
             VALUES ($1, $2, $3, 'Done', '#fff', 45000, 'completed', false, now(), now())",
            &[&done, &ws.0, &project.0],
        )
        .await
        .unwrap();
    }
    let when = Utc.timestamp_opt(1_800_000_600, 0).unwrap();
    let attrs = WorkItemAttrs {
        state_id: Some(done),
        priority: Priority::High,
        start_date: chrono::NaiveDate::from_ymd_opt(2026, 11, 1),
        target_date: chrono::NaiveDate::from_ymd_opt(2026, 11, 30),
        completed_at: Some(when),
        updated_at: when,
        updated_by: Some(owner),
        description_html: "<p>Résumé &amp; more</p>".into(),
        assignee_ids: vec![owner, owner],
        label_ids: vec![label],
        parent_id: None,
        point: Some(7),
    };
    // committed: every attribute as written, the owner once, the group of the state
    let mut tx = env.db.begin(&s).await.unwrap();
    let a = tx.allocate(&s).await.unwrap();
    let id = insert_with_attrs(&mut tx, s, a, &attrs).await.unwrap();
    tx.commit().await.unwrap();
    let row = env.db.work_item_row(&s, id).await.unwrap().expect("committed");
    let mut want = attrs.clone();
    want.assignee_ids = vec![owner];
    assert_eq!(row.attrs, want);
    assert_eq!(row.state_group, Some(StateGroup::Completed));
    // rolled back: nothing of it is there, and the number is gone (a rollback leaves a gap)
    let mut tx = env.db.begin(&s).await.unwrap();
    let a = tx.allocate(&s).await.unwrap();
    let rolled = insert_with_attrs(&mut tx, s, a, &attrs).await.unwrap();
    tx.rollback().await.unwrap();
    assert!(env.db.work_item_row(&s, rolled).await.unwrap().is_none());
    // refused: each, and the item with it
    let bad = |f: &dyn Fn(&mut WorkItemAttrs)| {
        let mut a = attrs.clone();
        f(&mut a);
        a
    };
    for refused in [
        bad(&|a| a.state_id = Some(new_id())),
        bad(&|a| a.assignee_ids = vec![UserId(new_id())]),
        bad(&|a| a.label_ids = vec![new_id()]),
        bad(&|a| a.start_date = chrono::NaiveDate::from_ymd_opt(2026, 12, 1)),
    ] {
        let mut tx = env.db.begin(&s).await.unwrap();
        let a = tx.allocate(&s).await.unwrap();
        let got = insert_with_attrs(&mut tx, s, a, &refused).await;
        assert!(matches!(got, Err(DbError::Backend(_))), "{refused:?}: {got:?}");
        let _ = tx.rollback().await;
    }
    assert_eq!(
        env.db
            .work_item_page(
                &s,
                &WorkItemQuery {
                    order: WorkItemOrder { field: WorkItemOrderField::SequenceId, descending: false },
                    limit: 10,
                    page: 0,
                    updated_after: None,
                    limited_guest: None
                }
            )
            .await
            .unwrap()
            .total,
        1
    );
}

/// The place a new work item takes is read from the greatest in its state: one per state (and one for the items with none), a deleted
/// item not counted, an archived one and a draft counted (what the pin's `Issue.objects` finds).
#[tokio::test]
async fn the_greatest_place_in_a_state_is_read_from_the_live_rows_of_that_state() {
    let env = env().await;
    let owner = user_without_login(&env).await;
    let w = workspace_for(owner, "item-places");
    env.db.insert_workspace(&w).await.unwrap();
    let ws = w.workspace.id;
    let new_project = project_for(ws, owner, "Places", "PLC");
    env.db.insert_project(&WorkspaceScope::mint(ws, owner, Role::Admin), &new_project).await.unwrap();
    let project = new_project.project.id;
    let s = Scope::mint(ws, project, owner);
    let (a, b) = (new_project.states[0].id, new_project.states[1].id);
    let make = |state: Option<Uuid>, place: i64, edit: &dyn Fn(&mut WorkItem)| {
        let env = &env;
        let mut w = item(ws, project, Allocation { sequence_id: 0, sort_order: place }, "n", None);
        edit(&mut w);
        let state_attrs = WorkItemAttrs { state_id: state, ..WorkItemAttrs::bare(w.created_at) };
        async move {
            let mut tx = env.db.begin(&s).await.unwrap();
            let alloc = tx.allocate(&s).await.unwrap();
            w.sequence_id = alloc.sequence_id;
            tx.insert_work_item(&s, &w).await.unwrap();
            tx.insert_work_item_attrs(&s, w.id, &state_attrs).await.unwrap();
            tx.commit().await.unwrap();
            w.id
        }
    };
    assert_eq!(env.db.max_work_item_sort_order(&s, Some(a)).await.unwrap(), None, "no item has a place yet");
    make(Some(a), 65_535, &|_| {}).await;
    make(Some(a), 75_535, &|_| {}).await;
    make(Some(b), 65_535, &|_| {}).await;
    make(None, 10_000, &|_| {}).await;
    let gone = make(Some(a), 95_535, &|_| {}).await;
    make(Some(b), 85_535, &|w| w.archived_at = Some(Utc.timestamp_opt(1_800_000_000, 0).unwrap())).await;
    make(Some(b), 95_535, &|w| w.is_draft = true).await;
    let mut tx = env.db.begin(&s).await.unwrap();
    assert!(tx.soft_delete_work_item(&s, gone, Utc.timestamp_opt(1_800_000_001, 0).unwrap()).await.unwrap());
    tx.commit().await.unwrap();
    assert_eq!(
        env.db.max_work_item_sort_order(&s, Some(a)).await.unwrap(),
        Some(75_535),
        "the deleted one is not counted"
    );
    assert_eq!(
        env.db.max_work_item_sort_order(&s, Some(b)).await.unwrap(),
        Some(95_535),
        "an archived one and a draft are"
    );
    assert_eq!(
        env.db.max_work_item_sort_order(&s, None).await.unwrap(),
        Some(10_000),
        "items with no state are their own run"
    );
    assert_eq!(env.db.max_work_item_sort_order(&s, Some(new_id())).await.unwrap(), None);
}

/// `update_profile` is ONE statement: the onboarding page sends several requests at once, each changing different fields of
/// one profile, and none may lose another's change. 32 patches, each setting its own preference, from as many tasks.
#[tokio::test]
async fn concurrent_profile_patches_to_different_fields_all_survive() {
    let env = env().await;
    let user = user_with_profile(&env).await;
    let at = Utc.timestamp_opt(1_800_000_100, 0).unwrap();
    let mut handles = Vec::new();
    for i in 0..32 {
        let db = env.db.clone();
        handles.push(tokio::spawn(async move {
            let mut patch = ProfilePatch::default();
            patch.preferences.insert(format!("k{i}"), serde_json::json!(i));
            if i % 2 == 0 {
                patch.first_name = Some(format!("n{i}"));
            }
            db.update_profile(user, &patch, at).await.unwrap().unwrap();
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let p = env.db.profile(user).await.unwrap().unwrap();
    let keys = p.preferences.as_object().unwrap();
    assert_eq!(keys.len(), 32, "a patch lost another's change: {keys:?}");
    assert!((0..32).all(|i| keys[&format!("k{i}")] == serde_json::json!(i)));
    assert!(p.first_name.starts_with('n'), "a name was written by one of the even patches");
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

    // duplicate live external key -> Conflict; the failed tx writes nothing
    let mut tx = env.db.begin(&s).await.unwrap();
    let a2 = tx.allocate(&s).await.unwrap();
    assert_eq!(a2.sequence_id, 2);
    let err = tx.insert_work_item(&s, &item(ws, p, a2, "dup", Some("1"))).await.unwrap_err();
    assert!(matches!(err, DbError::Conflict(_)), "got {err:?}");
    tx.rollback().await.unwrap();

    // control: nothing leaked. The number is NOT given back: FR-22 (ADR-0043 round 29) allocates in its own short
    // transaction so the counter row is locked for one statement, and a create that fails after its allocation loses the
    // number, which ADR-0043 accepts. Until FR-22 this asserted the opposite ("a rolled-back allocation must not leave a
    // gap", the counter row was transactional); the property was given up on purpose, in the commit that moved the
    // allocation, and `model_postgres_adapter_matches_the_reference_model` carries the same change.
    assert_eq!(every_item_name(&env, &s).await.len(), 1);
    let mut tx = env.db.begin(&s).await.unwrap();
    assert_eq!(tx.allocate(&s).await.unwrap().sequence_id, 3, "a rolled-back create's number stays taken (gap, FR-22)");
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
            // later iterations would see a stale open transaction and a later commit would commit what they left.
            // The abandoned transaction WRITES (an item, so it is inside a transaction when dropped): since FR-22 an
            // allocation alone is its own short transaction and leaves nothing open.
            let mut tx = env.db.begin(&s).await.unwrap();
            let a = tx.allocate(&s).await.unwrap();
            tx.insert_work_item(&s, &item(ws, p, a, "abandoned", None)).await.unwrap();
            drop(tx); // never committed
        }
        // nothing an abandoned transaction wrote is visible, and the numbers it took stay taken (40 of them)
        let mut tx = env.db.begin(&s).await.unwrap();
        let a = tx.allocate(&s).await.unwrap();
        assert_eq!(a.sequence_id, 41, "the numbers of abandoned transactions are not given back (FR-22)");
        tx.insert_work_item(&s, &item(ws, p, a, "kept", None)).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(every_item_name(&env, &s).await, ["kept"], "abandoned transactions must roll back");
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

    assert_eq!(live_item_count(&env, &s).await, 1);
    assert_eq!(every_item_name(&env, &s).await.len(), 2);
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
    env.db
        .mark_outbox_dispatched(&sys, row.envelope.event_id, Utc.timestamp_opt(1_800_000_001, 0).unwrap())
        .await
        .unwrap();
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

// ---------------------------------------------------------------------------------------------------------------------------------
// Issue #44, stage 6b: a work item's history (migration 0014) and the update of a work item. The generated sequences in `model` hold the
// adapter to the reference model; these are the examples a model that sees only the port cannot state: the table's shape and what its
// constraints refuse, the upgrade of a database that has data, the order of rows in the one second they tie, an all-or-nothing batch
// counted in the table itself, deliveries that race, and the rows an update leaves in the join tables.
// ---------------------------------------------------------------------------------------------------------------------------------

/// A workspace and project of its own, an owner who is a user, and `n` live work items of the project: what a history row can name.
async fn tenant_with_items(env: &Env, tag: &str, n: usize) -> (Scope, UserId, Vec<WorkItemId>) {
    let owner = user_without_login(env).await;
    let w = workspace_for(owner, tag);
    env.db.insert_workspace(&w).await.unwrap();
    let ws = w.workspace.id;
    let new_project = project_for(ws, owner, tag, "HIS");
    env.db.insert_project(&WorkspaceScope::mint(ws, owner, Role::Admin), &new_project).await.unwrap();
    let s = Scope::mint(ws, new_project.project.id, owner);
    let mut ids = Vec::new();
    for i in 0..n {
        let mut tx = env.db.begin(&s).await.unwrap();
        let a = tx.allocate(&s).await.unwrap();
        let w = item(ws, s.project_id(), a, &format!("item {i}"), None);
        tx.insert_work_item(&s, &w).await.unwrap();
        tx.commit().await.unwrap();
        ids.push(w.id);
    }
    (s, owner, ids)
}

fn at(secs: i64) -> Timestamp {
    Utc.timestamp_opt(1_800_000_000 + secs, 0).unwrap()
}

/// A history row of the tenant `s` for `item`, in event `event` at place `ordinal`, made `secs` after one time.
fn hrow(s: &Scope, item: WorkItemId, event: Uuid, ordinal: i32, secs: i64, actor: Option<UserId>) -> WorkItemActivity {
    WorkItemActivity {
        id: ActivityId(new_id()),
        event_id: EventId(event),
        ordinal,
        workspace_id: s.workspace_id(),
        project_id: s.project_id(),
        work_item_id: item,
        actor_id: actor,
        verb: "updated".into(),
        field: Some("name".into()),
        old_value: Some("old".into()),
        new_value: Some("new".into()),
        comment: "updated the name to".into(),
        old_identifier: None,
        new_identifier: None,
        epoch: 1_800_000_000 + secs,
        created_at: at(secs),
    }
}

/// How many rows of history the table holds for these work items.
async fn history_rows(env: &Env, items: &[WorkItemId]) -> i64 {
    let raw = env.conn.raw().await.unwrap();
    let ids: Vec<Uuid> = items.iter().map(|i| i.0).collect();
    raw.query_one("SELECT count(*) FROM work_item_activities WHERE work_item_id = ANY($1)", &[&ids])
        .await
        .unwrap()
        .get(0)
}

/// The SQLSTATE and the constraint a statement that fails names.
async fn refusal(
    raw: &tokio_postgres::Client,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> (String, Option<String>) {
    let e = raw.execute(sql, params).await.expect_err("the statement was refused");
    let db = e.as_db_error().expect("a database error");
    (db.code().code().to_string(), db.constraint().map(str::to_string))
}

/// A database created before migration 0014 has work items and no history. `migrate` makes the table, empty, and applying it again
/// changes nothing: the rows written in between are there as they were, in the order they were written, and the table's constraints
/// are not added a second time. A pinned regression for the upgrade path of a database that has data, as 0013's.
#[tokio::test]
async fn migration_0014_adds_the_history_table_to_a_database_that_has_work_items_and_a_second_run_changes_nothing() {
    let container = Postgres::default().start().await.expect("start postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let conn = Arc::new(PoolConnector::from_url(&url, 4).unwrap());
    let raw = conn.raw().await.unwrap();
    for sql in [
        thrust_adapters_postgres::MIGRATION_0001,
        thrust_adapters_postgres::MIGRATION_0002,
        thrust_adapters_postgres::MIGRATION_0003,
        thrust_adapters_postgres::MIGRATION_0004,
        thrust_adapters_postgres::MIGRATION_0005,
        thrust_adapters_postgres::MIGRATION_0006,
        thrust_adapters_postgres::MIGRATION_0007,
        thrust_adapters_postgres::MIGRATION_0008,
        thrust_adapters_postgres::MIGRATION_0009,
        thrust_adapters_postgres::MIGRATION_0010,
        thrust_adapters_postgres::MIGRATION_0011,
        thrust_adapters_postgres::MIGRATION_0012,
        thrust_adapters_postgres::MIGRATION_0013,
    ] {
        raw.batch_execute(sql).await.unwrap();
    }
    let (ws, project, user, item) = (new_id(), new_id(), new_id(), new_id());
    raw.execute("INSERT INTO users (id) VALUES ($1)", &[&user]).await.unwrap();
    raw.execute("INSERT INTO workspaces (id, slug, name) VALUES ($1, 'legacy-history', 'Legacy')", &[&ws])
        .await
        .unwrap();
    raw.execute("INSERT INTO workspace_members (workspace_id, user_id) VALUES ($1, $2)", &[&ws, &user]).await.unwrap();
    raw.execute(
        "INSERT INTO projects (id, workspace_id, name, identifier) VALUES ($1, $2, 'Legacy project', 'LEG')",
        &[&project, &ws],
    )
    .await
    .unwrap();
    raw.execute(
        "INSERT INTO project_members (project_id, workspace_id, user_id, role) VALUES ($1, $2, $3, 'member')",
        &[&project, &ws, &user],
    )
    .await
    .unwrap();
    let made = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    raw.execute(
        "INSERT INTO work_items (id, workspace_id, project_id, sequence_id, sort_order, name, created_by, created_at) VALUES ($1, $2, $3, 1, 10000, 'Legacy item', $4, $5)",
        &[&item, &ws, &project, &user, &made],
    )
    .await
    .unwrap();
    assert!(
        raw.query_one("SELECT to_regclass('work_item_activities')::text", &[])
            .await
            .unwrap()
            .get::<_, Option<String>>(0)
            .is_none(),
        "no history table yet"
    );

    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    async fn rows(raw: &tokio_postgres::Client) -> Vec<String> {
        let rows = raw.query("SELECT seq, id::text, event_id::text, ordinal, created_at::text FROM work_item_activities ORDER BY seq", &[]).await.unwrap();
        rows.iter()
            .map(|r| {
                format!(
                    "{}|{}|{}|{}|{}",
                    r.get::<_, i64>(0),
                    r.get::<_, String>(1),
                    r.get::<_, String>(2),
                    r.get::<_, i32>(3),
                    r.get::<_, String>(4)
                )
            })
            .collect()
    }
    assert_eq!(rows(&raw).await, Vec::<String>::new(), "made empty");
    drop(raw);

    let db = PgDatabase::new(conn.clone());
    let s = Scope::mint(WorkspaceId(ws), ProjectId(project), UserId(user));
    let event = new_id();
    let (first, second) =
        (hrow(&s, WorkItemId(item), event, 0, 7, Some(UserId(user))), hrow(&s, WorkItemId(item), event, 1, 3, None));
    assert_eq!(
        db.insert_work_item_activities(&SystemScope::mint(), &[first.clone(), second.clone()]).await.unwrap(),
        2
    );
    let raw = conn.raw().await.unwrap();
    let written = rows(&raw).await;
    assert_eq!(written.len(), 2);
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    assert_eq!(rows(&raw).await, written, "a second run changed the rows");
    let kinds = raw
        .query(
            "SELECT contype::text, count(*) FROM pg_constraint WHERE conrelid = 'work_item_activities'::regclass AND contype IN ('p', 'u', 'f') GROUP BY contype ORDER BY contype",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        kinds.iter().map(|r| (r.get::<_, String>(0), r.get::<_, i64>(1))).collect::<Vec<_>>(),
        [("f".into(), 3), ("p".into(), 1), ("u".into(), 1)],
        "a second run added a constraint"
    );
    drop(raw);
    // read back through the adapter: the later time is the later row
    assert_eq!(db.work_item_history(&s, WorkItemId(item), None).await.unwrap(), vec![second, first]);
}

/// The history table's columns are written out, so a column added later has to be argued for, and the constraints that keep a row well
/// formed are exercised: a key of event and place that is unique (and an id), a work item, an actor and a project that exist, columns that
/// cannot be missing, and an order of writing that cannot be given.
#[tokio::test]
async fn the_history_table_has_the_columns_it_says_and_the_constraints_that_keep_a_row_well_formed() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let q = |sql: &'static str| {
        let raw = &raw;
        async move {
            raw.query(sql, &[])
                .await
                .unwrap()
                .iter()
                .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
                .collect::<Vec<_>>()
        }
    };
    let pairs = |v: Vec<(&str, &str)>| v.into_iter().map(|(n, t)| (n.to_string(), t.to_string())).collect::<Vec<_>>();
    let (ts, text, uuid, bigint, int) = ("timestamp with time zone", "text", "uuid", "bigint", "integer");
    assert_eq!(
        q("SELECT column_name::text, data_type::text FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'work_item_activities' ORDER BY column_name").await,
        pairs(vec![
            ("actor_id", uuid),
            ("comment", text),
            ("created_at", ts),
            ("epoch", bigint),
            ("event_id", uuid),
            ("field", text),
            ("id", uuid),
            ("new_identifier", uuid),
            ("new_value", text),
            ("old_identifier", uuid),
            ("old_value", text),
            ("ordinal", int),
            ("project_id", uuid),
            ("seq", bigint),
            ("verb", text),
            ("work_item_id", uuid),
            ("workspace_id", uuid),
        ])
    );
    // what may be missing: the actor, what changed and its two values and identifiers; the rest may not
    let mut nullable = q("SELECT column_name::text, is_nullable::text FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'work_item_activities' AND is_nullable = 'YES'").await;
    nullable.sort();
    assert_eq!(
        nullable,
        pairs(vec![
            ("actor_id", "YES"),
            ("field", "YES"),
            ("new_identifier", "YES"),
            ("new_value", "YES"),
            ("old_identifier", "YES"),
            ("old_value", "YES")
        ])
    );
    // the order of writing is the database's: `seq` is generated, and cannot be given
    assert_eq!(
        q("SELECT is_identity::text, identity_generation::text FROM information_schema.columns WHERE table_name = 'work_item_activities' AND column_name = 'seq'").await,
        pairs(vec![("YES", "ALWAYS")])
    );
    let mut constraints = q("SELECT contype::text, pg_get_constraintdef(oid)::text FROM pg_constraint WHERE conrelid = 'work_item_activities'::regclass AND contype IN ('p', 'u', 'f')").await;
    constraints.sort();
    assert_eq!(
        constraints,
        pairs(vec![
            ("f", "FOREIGN KEY (actor_id) REFERENCES users(id)"),
            ("f", "FOREIGN KEY (work_item_id) REFERENCES work_items(id)"),
            ("f", "FOREIGN KEY (workspace_id, project_id) REFERENCES projects(workspace_id, id)"),
            ("p", "PRIMARY KEY (id)"),
            ("u", "UNIQUE (event_id, ordinal)"),
        ])
    );
    let index = q("SELECT indexname::text, indexdef::text FROM pg_indexes WHERE tablename = 'work_item_activities' AND indexname = 'work_item_activities_by_item'").await;
    assert_eq!(index.len(), 1, "the index a history is read by");
    assert!(index[0].1.ends_with("(work_item_id, created_at)"), "{}", index[0].1);

    // what the constraints refuse
    let (s, owner, items) = tenant_with_items(&env, "hist-shape", 1).await;
    let (ws, project, item) = (s.workspace_id().0, s.project_id().0, items[0].0);
    let insert = "INSERT INTO work_item_activities (id, event_id, ordinal, workspace_id, project_id, work_item_id, actor_id, verb, comment, epoch, created_at) \
                  VALUES ($1, $2, $3, $4, $5, $6, $7, 'updated', '', 0, now())";
    let event = new_id();
    raw.execute(insert, &[&new_id(), &event, &0i32, &ws, &project, &item, &None::<Uuid>]).await.unwrap();
    // an event and a place once; the same place in another event and another place in the same event are two rows
    let (code, constraint) =
        refusal(&raw, insert, &[&new_id(), &event, &0i32, &ws, &project, &item, &None::<Uuid>]).await;
    assert_eq!((code.as_str(), constraint.as_deref()), ("23505", Some("work_item_activities_event_key")));
    raw.execute(insert, &[&new_id(), &event, &1i32, &ws, &project, &item, &None::<Uuid>]).await.unwrap();
    raw.execute(insert, &[&new_id(), &new_id(), &0i32, &ws, &project, &item, &Some(owner.0)]).await.unwrap();
    // an id once
    let id = new_id();
    raw.execute(insert, &[&id, &new_id(), &0i32, &ws, &project, &item, &None::<Uuid>]).await.unwrap();
    let (code, constraint) = refusal(&raw, insert, &[&id, &new_id(), &0i32, &ws, &project, &item, &None::<Uuid>]).await;
    assert_eq!((code.as_str(), constraint.as_deref()), ("23505", Some("work_item_activities_pkey")));
    // a work item, an actor and a project that are there
    let (code, constraint) =
        refusal(&raw, insert, &[&new_id(), &new_id(), &0i32, &ws, &project, &new_id(), &None::<Uuid>]).await;
    assert_eq!((code.as_str(), constraint.as_deref()), ("23503", Some("work_item_activities_work_item_id_fkey")));
    let (code, constraint) =
        refusal(&raw, insert, &[&new_id(), &new_id(), &0i32, &ws, &project, &item, &Some(new_id())]).await;
    assert_eq!((code.as_str(), constraint.as_deref()), ("23503", Some("work_item_activities_actor_id_fkey")));
    let (code, constraint) =
        refusal(&raw, insert, &[&new_id(), &new_id(), &0i32, &new_id(), &project, &item, &None::<Uuid>]).await;
    assert_eq!((code.as_str(), constraint.as_deref()), ("23503", Some("work_item_activities_tenant_fkey")));
    let (code, constraint) =
        refusal(&raw, insert, &[&new_id(), &new_id(), &0i32, &ws, &new_id(), &item, &None::<Uuid>]).await;
    assert_eq!((code.as_str(), constraint.as_deref()), ("23503", Some("work_item_activities_tenant_fkey")));
    // the order of writing cannot be given
    let (code, _) = refusal(
        &raw,
        "INSERT INTO work_item_activities (seq, id, event_id, ordinal, workspace_id, project_id, work_item_id, verb, comment, epoch, created_at) VALUES (1, $1, $2, 0, $3, $4, $5, 'updated', '', 0, now())",
        &[&new_id(), &new_id(), &ws, &project, &item],
    )
    .await;
    assert_eq!(code, "428C9");
    // a row of the tenant the work item is not in satisfies the keys the table has (the adapter refuses it: see the test of the batch)
    assert_eq!(history_rows(&env, &items).await, 4);
}

/// Rows of one time are read in the order they were written. The ids, the events and the places of these run backwards, so that a read
/// that ordered by any of them would be reversed; `seq` is what puts them right. A touch moves the time alone: the row keeps its `seq`,
/// every other column, and its place among rows of its new time (the first written is first among those that tie with it).
#[tokio::test]
async fn rows_of_one_time_come_back_in_the_order_they_were_written_and_a_touch_moves_the_time_alone() {
    let env = env().await;
    let (s, owner, items) = tenant_with_items(&env, "hist-ties", 1).await;
    let item = items[0];
    let sys = SystemScope::mint();
    let row = |k: u128, event: u128, ordinal: i32| WorkItemActivity {
        id: ActivityId(Uuid::from_u128(u128::MAX - k)),
        ..hrow(&s, item, Uuid::from_u128(u128::MAX - event), ordinal, 5, Some(owner))
    };
    let batches = vec![vec![row(0, 0, 9), row(1, 0, 8)], vec![row(2, 1, 7)], vec![row(3, 2, 6), row(4, 2, 5)]];
    for b in &batches {
        assert_eq!(env.db.insert_work_item_activities(&sys, b).await.unwrap(), b.len() as u64);
    }
    let written: Vec<WorkItemActivity> = batches.concat();
    assert_eq!(env.db.work_item_history(&s, item, None).await.unwrap(), written);
    assert_eq!(
        env.db.latest_work_item_activity(&sys, s.workspace_id(), s.project_id(), item).await.unwrap().as_ref(),
        written.last()
    );
    let raw = env.conn.raw().await.unwrap();
    let seq_order = || {
        let raw = &raw;
        async move {
            raw.query("SELECT id FROM work_item_activities WHERE work_item_id = $1 ORDER BY seq", &[&item.0])
                .await
                .unwrap()
                .iter()
                .map(|r| ActivityId(r.get(0)))
                .collect::<Vec<_>>()
        }
    };
    let ids: Vec<ActivityId> = written.iter().map(|r| r.id).collect();
    assert_eq!(seq_order().await, ids, "written in this order");

    // the first written, touched to a later time: the newest, whatever was written after it, and the others as they were
    assert!(env.db.touch_work_item_activity(&sys, written[0].id, at(9)).await.unwrap());
    let moved = WorkItemActivity { created_at: at(9), ..written[0].clone() };
    let mut want: Vec<WorkItemActivity> = written[1..].to_vec();
    want.push(moved.clone());
    assert_eq!(env.db.work_item_history(&s, item, None).await.unwrap(), want);
    assert_eq!(
        env.db.latest_work_item_activity(&sys, s.workspace_id(), s.project_id(), item).await.unwrap(),
        Some(moved.clone())
    );
    assert_eq!(seq_order().await, ids, "a touch does not move a row in the order written");
    // the same row moved back to the others' time is the first of them again, and one moved before them all is first, whatever its place
    assert!(env.db.touch_work_item_activity(&sys, written[0].id, at(5)).await.unwrap());
    assert_eq!(env.db.work_item_history(&s, item, None).await.unwrap(), written);
    assert!(env.db.touch_work_item_activity(&sys, written[4].id, at(1)).await.unwrap());
    let early = WorkItemActivity { created_at: at(1), ..written[4].clone() };
    let mut want = vec![early.clone()];
    want.extend(written[..4].iter().cloned());
    assert_eq!(env.db.work_item_history(&s, item, None).await.unwrap(), want);
    assert_eq!(
        env.db.latest_work_item_activity(&sys, s.workspace_id(), s.project_id(), item).await.unwrap().as_ref(),
        written.get(3)
    );
    // after a time is strict: the rows that tie with it are out
    assert_eq!(env.db.work_item_history(&s, item, Some(at(1))).await.unwrap(), written[..4].to_vec());
    assert_eq!(env.db.work_item_history(&s, item, Some(at(5))).await.unwrap(), Vec::<WorkItemActivity>::new());
    assert_eq!(env.db.work_item_history(&s, item, Some(at(4))).await.unwrap(), written[..4].to_vec());
    // a row that is not there is not touched
    assert!(!env.db.touch_work_item_activity(&sys, ActivityId(new_id()), at(7)).await.unwrap());
    assert_eq!(seq_order().await, ids);
}

/// A batch that holds a row the adapter refuses writes none of its rows, whichever place that row has in it: counted in the table, which
/// is where a row that was written and then lost to a rollback would show, and the rows that were valid are new afterwards. A key that is
/// there is refused all the same when the row naming it is wrong.
#[tokio::test]
async fn a_batch_with_a_row_the_adapter_refuses_writes_none_of_its_rows() {
    let env = env().await;
    let sys = SystemScope::mint();
    let (s, owner, items) = tenant_with_items(&env, "hist-refused", 1).await;
    let (other, _other_owner, others) = tenant_with_items(&env, "hist-refused-other", 1).await;
    let (item, other_item) = (items[0], others[0]);
    let mut expected = 0;
    // (what is wrong with the row, the row)
    type Break = Box<dyn Fn(WorkItemActivity) -> WorkItemActivity>;
    let strays: Vec<(&str, Break)> = vec![
        (
            "an item of another tenant, named with this one's workspace and project",
            Box::new(move |r| WorkItemActivity { work_item_id: other_item, ..r }),
        ),
        ("an item that is not there", Box::new(|r| WorkItemActivity { work_item_id: WorkItemId(new_id()), ..r })),
        (
            "this tenant's item, named with the other's workspace and project",
            Box::new(move |r| WorkItemActivity {
                workspace_id: other.workspace_id(),
                project_id: other.project_id(),
                ..r
            }),
        ),
        (
            "this tenant's item, named with the other's workspace",
            Box::new(move |r| WorkItemActivity { workspace_id: other.workspace_id(), ..r }),
        ),
        (
            "this tenant's item, named with the other's project",
            Box::new(move |r| WorkItemActivity { project_id: other.project_id(), ..r }),
        ),
        ("an actor who is no user", Box::new(|r| WorkItemActivity { actor_id: Some(UserId(new_id())), ..r })),
    ];
    for (wrong, make) in &strays {
        for place in 0..3 {
            let event = new_id();
            let valid = [hrow(&s, item, event, 0, 1, Some(owner)), hrow(&s, item, event, 1, 2, None)];
            let mut batch = valid.to_vec();
            batch.insert(place, make(hrow(&s, item, new_id(), 0, 3, Some(owner))));
            let got = env.db.insert_work_item_activities(&sys, &batch).await;
            assert!(matches!(got, Err(DbError::Backend(_))), "{wrong}, at {place}: {got:?}");
            assert_eq!(
                history_rows(&env, &[item, other_item]).await,
                expected,
                "{wrong}, at {place}: a row of the refused batch was written"
            );
            assert_eq!(
                env.db.insert_work_item_activities(&sys, &valid).await.unwrap(),
                2,
                "{wrong}, at {place}: the valid rows were not new"
            );
            expected += 2;
        }
        // a row whose key is there, and which is wrong, is refused as well, and the row that holds the key is as it was
        let held = hrow(&s, item, new_id(), 0, 4, Some(owner));
        assert_eq!(env.db.insert_work_item_activities(&sys, std::slice::from_ref(&held)).await.unwrap(), 1);
        expected += 1;
        let again = make(WorkItemActivity { id: ActivityId(new_id()), ..held.clone() });
        let got = env.db.insert_work_item_activities(&sys, &[again]).await;
        assert!(matches!(got, Err(DbError::Backend(_))), "{wrong}, key held: {got:?}");
        assert_eq!(history_rows(&env, &[item, other_item]).await, expected);
    }
    assert_eq!(
        env.db.work_item_history(&s, item, None).await.unwrap().len() as i64,
        expected,
        "every row written is read back"
    );
    assert_eq!(
        env.db.work_item_history(&other, other_item, None).await.unwrap(),
        Vec::<WorkItemActivity>::new(),
        "nothing was written for the other tenant"
    );
}

/// A history that is delivered at least once is written once: sixteen consumers write overlapping windows of one event's rows, each
/// window in order, with an id of its own for every row, at the same time. Each key is written by exactly one of them, none of them is
/// refused, and no row is lost or doubled.
#[tokio::test]
async fn deliveries_of_the_rows_of_one_event_that_race_write_each_row_once() {
    let env = env().await;
    let (s, owner, items) = tenant_with_items(&env, "hist-race", 1).await;
    let item = items[0];
    let event = new_id();
    let mut handles = Vec::new();
    for i in 0..16 {
        let db = env.db.clone();
        handles.push(tokio::spawn(async move {
            let start = i % 4;
            let rows: Vec<WorkItemActivity> =
                (start..start + 4).map(|o| hrow(&s, item, event, o, 0, Some(owner))).collect();
            tokio::time::timeout(
                std::time::Duration::from_secs(30),
                db.insert_work_item_activities(&SystemScope::mint(), &rows),
            )
            .await
            .expect("no deadlock")
            .unwrap()
        }));
    }
    let mut written = 0;
    for h in handles {
        written += h.await.unwrap();
    }
    // the windows start at 0 to 3 and hold 4 places: places 0 to 6, once each
    assert_eq!(written, 7);
    let history = env.db.work_item_history(&s, item, None).await.unwrap();
    let mut places: Vec<i32> = history.iter().map(|r| r.ordinal).collect();
    places.sort();
    assert_eq!(places, (0..7).collect::<Vec<_>>());
}

/// An update that names people or labels replaces the rows of the item in the join tables, all of them (those of people who are not in
/// the project, which no read shows, too) and no row of another item; one that names none leaves the rows; naming none at all (an empty
/// list) leaves no row.
#[tokio::test]
async fn an_update_replaces_the_join_rows_of_the_people_and_labels_it_names_and_leaves_the_rest() {
    let env = env().await;
    let (s, owner, items) = tenant_with_items(&env, "upd-rows", 2).await;
    let (a, b) = (items[0], items[1]);
    let (u2, u3) = (user_without_login(&env).await, user_without_login(&env).await);
    let (l1, l2) = (new_id(), new_id());
    let raw = env.conn.raw().await.unwrap();
    for l in [l1, l2] {
        raw.execute("INSERT INTO labels (id, workspace_id, project_id, name, created_at, updated_at) VALUES ($1, $2, $3, $4, now(), now())", &[&l, &s.workspace_id().0, &s.project_id().0, &format!("l-{l}")])
            .await
            .unwrap();
    }
    let people = |id: WorkItemId| {
        let raw = &raw;
        async move {
            raw.query("SELECT user_id FROM work_item_assignees WHERE work_item_id = $1 ORDER BY user_id", &[&id.0])
                .await
                .unwrap()
                .iter()
                .map(|r| UserId(r.get(0)))
                .collect::<Vec<_>>()
        }
    };
    let labels = |id: WorkItemId| {
        let raw = &raw;
        async move {
            raw.query("SELECT label_id FROM work_item_labels WHERE work_item_id = $1 ORDER BY label_id", &[&id.0])
                .await
                .unwrap()
                .iter()
                .map(|r| r.get::<_, Uuid>(0))
                .collect::<Vec<_>>()
        }
    };
    let update = |people: Option<Vec<UserId>>, labels: Option<Vec<Uuid>>| WorkItemUpdate {
        name: "named".into(),
        state_id: None,
        priority: Priority::None,
        start_date: None,
        target_date: None,
        completed_at: None,
        updated_at: at(10),
        updated_by: owner,
        description_html: "<p></p>".into(),
        assignee_ids: people,
        label_ids: labels,
        external_source: None,
        external_id: None,
        parent_id: None,
        point: None,
    };
    let run = |id: WorkItemId, u: WorkItemUpdate| {
        let env = &env;
        async move {
            let mut tx = env.db.begin(&s).await.unwrap();
            let found = tx.update_work_item(&s, id, &u).await.unwrap();
            tx.commit().await.unwrap();
            found
        }
    };
    let sorted = |mut v: Vec<UserId>| {
        v.sort();
        v
    };
    let all = vec![owner, u2, u3, u3];
    let mut ls = vec![l1, l2];
    ls.sort();
    for id in [a, b] {
        assert!(run(id, update(Some(all.clone()), Some(ls.clone()))).await);
        assert_eq!(people(id).await, sorted(vec![owner, u2, u3]), "once each, those who are not in the project too");
        assert_eq!(labels(id).await, ls);
    }
    // one item is given other people and labels: every row of it is replaced, and no row of the other
    assert!(run(a, update(Some(vec![u2]), Some(vec![l2]))).await);
    assert_eq!((people(a).await, labels(a).await), (vec![u2], vec![l2]));
    assert_eq!((people(b).await, labels(b).await), (sorted(vec![owner, u2, u3]), ls.clone()), "the other item's rows");
    // none named: the rows stay
    assert!(run(a, update(None, None)).await);
    assert_eq!((people(a).await, labels(a).await), (vec![u2], vec![l2]));
    // an empty list: no row
    assert!(run(a, update(Some(vec![]), Some(vec![]))).await);
    assert_eq!((people(a).await, labels(a).await), (vec![], vec![]));
    assert_eq!((people(b).await.len(), labels(b).await.len()), (3, 2));
}

/// An update sees what its own transaction wrote: an item it inserted can be updated in it, and one it soft-deleted is not there for the
/// update that follows (`false`, nothing written), as the port says of an item that is not live.
#[tokio::test]
async fn an_update_sees_what_its_own_transaction_wrote() {
    let env = env().await;
    let (s, owner, items) = tenant_with_items(&env, "upd-own-tx", 1).await;
    let update = |name: &str| WorkItemUpdate {
        name: name.into(),
        state_id: None,
        priority: Priority::High,
        start_date: None,
        target_date: None,
        completed_at: None,
        updated_at: at(10),
        updated_by: owner,
        description_html: "<p>x</p>".into(),
        assignee_ids: Some(vec![owner]),
        label_ids: None,
        external_source: None,
        external_id: None,
        parent_id: None,
        point: None,
    };
    // inserted and updated in one transaction
    let mut tx = env.db.begin(&s).await.unwrap();
    let a = tx.allocate(&s).await.unwrap();
    let w = item(s.workspace_id(), s.project_id(), a, "inserted", None);
    tx.insert_work_item(&s, &w).await.unwrap();
    assert!(tx.update_work_item(&s, w.id, &update("updated in its own transaction")).await.unwrap());
    tx.commit().await.unwrap();
    let row = env.db.work_item_row(&s, w.id).await.unwrap().expect("committed");
    assert_eq!(
        (row.item.name.as_str(), row.attrs.priority, row.attrs.assignee_ids),
        ("updated in its own transaction", Priority::High, vec![owner])
    );
    // soft-deleted and then updated in one transaction
    let mut tx = env.db.begin(&s).await.unwrap();
    assert!(tx.soft_delete_work_item(&s, items[0], at(11)).await.unwrap());
    assert!(
        !tx.update_work_item(&s, items[0], &update("never written")).await.unwrap(),
        "an item deleted in this transaction is not live"
    );
    tx.commit().await.unwrap();
    let gone =
        env.db.get_work_item(&s, items[0], Visibility::WithDeleted).await.unwrap().expect("soft-deleted rows stay");
    assert_eq!((gone.name.as_str(), gone.deleted_at), ("item 0", Some(at(11))));
}

/// Only a live work item of the scope's own workspace and project is updated: one that is deleted, archived or a draft, one of another
/// project or another workspace, and one that is not there answer `false` and write nothing (not the columns, not the people). The port
/// names these cases; the generated sequences reach them too, and this is the pin that does not depend on the sample.
#[tokio::test]
async fn an_update_finds_a_live_work_item_of_the_scopes_own_project_alone() {
    let env = env().await;
    let (s, owner, _) = tenant_with_items(&env, "upd-live", 0).await;
    let (other, _, _) = tenant_with_items(&env, "upd-live-other", 0).await;
    // another project of the same workspace
    let sibling_project = project_for(s.workspace_id(), owner, "Sibling", "SIB");
    env.db.insert_project(&WorkspaceScope::mint(s.workspace_id(), owner, Role::Admin), &sibling_project).await.unwrap();
    let sibling = Scope::mint(s.workspace_id(), sibling_project.project.id, owner);
    // one item of each kind, inserted as it is (a work item is hidden by its columns, and only a soft delete hides one that exists)
    let make = |scope: Scope, edit: &dyn Fn(&mut WorkItem)| {
        let env = &env;
        let mut w = item(
            scope.workspace_id(),
            scope.project_id(),
            Allocation { sequence_id: 0, sort_order: 0 },
            "before",
            None,
        );
        edit(&mut w);
        async move {
            let mut tx = env.db.begin(&scope).await.unwrap();
            w.sequence_id = tx.allocate(&scope).await.unwrap().sequence_id;
            tx.insert_work_item(&scope, &w).await.unwrap();
            tx.commit().await.unwrap();
            w.id
        }
    };
    let live = make(s, &|_| {}).await;
    let deleted = make(s, &|w| w.deleted_at = Some(at(1))).await;
    let archived = make(s, &|w| w.archived_at = Some(at(1))).await;
    let draft = make(s, &|w| w.is_draft = true).await;
    let of_other = make(other, &|_| {}).await;
    let update = WorkItemUpdate {
        name: "after".into(),
        state_id: None,
        priority: Priority::Urgent,
        start_date: None,
        target_date: None,
        completed_at: None,
        updated_at: at(10),
        updated_by: owner,
        description_html: "<p>after</p>".into(),
        assignee_ids: Some(vec![owner]),
        label_ids: None,
        external_source: None,
        external_id: None,
        parent_id: None,
        point: None,
    };
    let try_update = |scope: Scope, id: WorkItemId| {
        let (env, update) = (&env, &update);
        async move {
            let mut tx = env.db.begin(&scope).await.unwrap();
            let found = tx.update_work_item(&scope, id, update).await.unwrap();
            tx.commit().await.unwrap();
            found
        }
    };
    for (what, scope, id) in [
        ("deleted", s, deleted),
        ("archived", s, archived),
        ("a draft", s, draft),
        ("one that is not there", s, WorkItemId(new_id())),
        ("an item of another workspace", s, of_other),
        ("the live item through another tenant's scope", other, live),
        ("the live item through another project's scope in the same workspace", sibling, live),
    ] {
        assert!(!try_update(scope, id).await, "{what}");
    }
    // nothing of it was written: every item has the name it had, and none has the person
    let raw = env.conn.raw().await.unwrap();
    for id in [live, deleted, archived, draft, of_other] {
        let r = raw.query_one("SELECT name, priority, (SELECT count(*) FROM work_item_assignees a WHERE a.work_item_id = w.id) FROM work_items w WHERE w.id = $1", &[&id.0]).await.unwrap();
        assert_eq!(
            (r.get::<_, String>(0), r.get::<_, String>(1), r.get::<_, i64>(2)),
            ("before".into(), "none".into(), 0),
            "{id:?}"
        );
    }
    // and the live item is found through its own scope, written whole
    assert!(try_update(s, live).await);
    let row = env.db.work_item_row(&s, live).await.unwrap().expect("live");
    assert_eq!(
        (row.item.name.as_str(), row.attrs.priority, row.attrs.assignee_ids),
        ("after", Priority::Urgent, vec![owner])
    );
}

// --- work items of the v1 API (issue #58, migration 0015) ---------------------------------------------------------------------
// The memory adapter is held to the same answers by `tests/work_items_v1.rs` in its own crate.

/// A work item of the scope's project, numbered from its counter, with the attributes `edit` makes of the bare ones, committed.
async fn put_item(
    env: &Env,
    s: Scope,
    name: &str,
    edit: impl FnOnce(&mut WorkItem, &mut WorkItemAttrs),
) -> Result<WorkItemId, DbError> {
    let mut tx = env.db.begin(&s).await.unwrap();
    let a = tx.allocate(&s).await.unwrap();
    let mut w = item(s.workspace_id(), s.project_id(), a, name, None);
    w.created_at = Utc.timestamp_opt(1_800_000_000 + a.sequence_id * 100, 0).unwrap();
    // made by the person the scope is for, as a creation is
    w.created_by = Some(s.user_id());
    let mut attrs = WorkItemAttrs::bare(w.created_at);
    edit(&mut w, &mut attrs);
    let id = w.id;
    tx.insert_work_item(&s, &w).await.unwrap();
    let put = tx.insert_work_item_attrs(&s, id, &attrs).await;
    match put {
        Ok(()) => {
            tx.commit().await.unwrap();
            Ok(id)
        }
        Err(e) => {
            tx.rollback().await.unwrap();
            Err(e)
        }
    }
}

/// The whole update of an item as it stands, with the v1 API's columns set as given.
fn whole(
    r: &WorkItemRow,
    parent: Option<Uuid>,
    point: Option<i32>,
    key: Option<(&str, &str)>,
    by: UserId,
) -> WorkItemUpdate {
    WorkItemUpdate {
        name: r.item.name.clone(),
        state_id: r.attrs.state_id,
        priority: r.attrs.priority,
        start_date: r.attrs.start_date,
        target_date: r.attrs.target_date,
        completed_at: r.attrs.completed_at,
        updated_at: Utc.timestamp_opt(1_800_001_000, 0).unwrap(),
        updated_by: by,
        description_html: r.attrs.description_html.clone(),
        assignee_ids: None,
        label_ids: None,
        external_source: key.map(|k| k.0.to_string()),
        external_id: key.map(|k| k.1.to_string()),
        parent_id: parent,
        point,
    }
}

/// `update_work_item` in a transaction of its own: committed when it answered, rolled back (as the core does) when it refused.
async fn update_item(env: &Env, s: Scope, id: WorkItemId, u: &WorkItemUpdate) -> Result<bool, DbError> {
    let mut tx = env.db.begin(&s).await.unwrap();
    match tx.update_work_item(&s, id, u).await {
        Ok(found) => {
            tx.commit().await.unwrap();
            Ok(found)
        }
        Err(e) => {
            tx.rollback().await.unwrap();
            Err(e)
        }
    }
}

async fn delete_item(env: &Env, s: Scope, id: WorkItemId) -> bool {
    let mut tx = env.db.begin(&s).await.unwrap();
    let found = tx.soft_delete_work_item(&s, id, Utc.timestamp_opt(1_800_002_000, 0).unwrap()).await.unwrap();
    tx.commit().await.unwrap();
    found
}

/// A parent, a point and a key are stored and read back; a maker may be missing; what the constraints refuse (a point outside 0 to
/// 12, a parent that is no work item) is refused and writes nothing; a key that another live item has is a conflict, and one that a
/// deleted item had is free.
#[tokio::test]
async fn a_work_item_has_a_parent_a_point_and_a_key_and_a_maker_that_may_be_missing() {
    let env = env().await;
    let (_, _, owner, s) = project_with_states(&env, "wi-attrs").await;
    let parent = put_item(&env, s, "parent", |_, _| {}).await.unwrap();
    let child = put_item(&env, s, "child", |w, a| {
        a.parent_id = Some(parent.0);
        a.point = Some(12);
        w.external_source = Some("gh".into());
        w.external_id = Some("7".into());
    })
    .await
    .unwrap();
    let r = env.db.work_item_row(&s, child).await.unwrap().unwrap();
    assert_eq!((r.attrs.parent_id, r.attrs.point), (Some(parent.0), Some(12)));
    assert_eq!((r.item.external_source.as_deref(), r.item.external_id.as_deref()), (Some("gh"), Some("7")));
    // nobody made this one: it is stored and read, and it is no one person's for a page asked for by maker
    let nobody = put_item(&env, s, "nobody's", |w, _| w.created_by = None).await.unwrap();
    assert_eq!(env.db.work_item_row(&s, nobody).await.unwrap().unwrap().item.created_by, None);
    let order = WorkItemOrder { field: WorkItemOrderField::SequenceId, descending: false };
    let by_one = WorkItemQuery { order, limit: 10, page: 0, updated_after: None, limited_guest: Some(owner) };
    let page = env.db.work_item_page(&s, &by_one).await.unwrap();
    assert!(page.rows.iter().all(|r| r.item.created_by == Some(owner)) && page.total == 2, "{page:?}");

    // refused, in a creation (the item is not left behind) and in an update
    for (point, parent) in [(Some(13), None), (Some(-1), None), (None, Some(new_id()))] {
        let got = put_item(&env, s, "refused", |_, a| {
            a.point = point;
            a.parent_id = parent;
        })
        .await;
        assert!(got.is_err(), "{point:?} {parent:?}");
        let got =
            update_item(&env, s, child, &whole(&r, parent.or(r.attrs.parent_id), point.or(r.attrs.point), None, owner))
                .await;
        assert!(got.is_err(), "{point:?} {parent:?}");
        assert_eq!(env.db.work_item_row(&s, child).await.unwrap().unwrap(), r, "nothing was written");
    }
    assert_eq!(live_item_count(&env, &s).await, 3);

    // a key that another live item has is a conflict, and a key the item has is its own
    let other = put_item(&env, s, "other", |_, _| {}).await.unwrap();
    let o = env.db.work_item_row(&s, other).await.unwrap().unwrap();
    let got = update_item(&env, s, other, &whole(&o, None, None, Some(("gh", "7")), owner)).await;
    assert!(matches!(got, Err(DbError::Conflict(_))), "{got:?}");
    assert!(update_item(&env, s, child, &whole(&r, Some(parent.0), Some(12), Some(("gh", "7")), owner)).await.unwrap());
    // once the holder is deleted the key is free
    assert!(delete_item(&env, s, child).await);
    assert!(update_item(&env, s, other, &whole(&o, None, None, Some(("gh", "7")), owner)).await.unwrap());
}

/// A limited guest's page is what they made, unless the project shows its guests everything (`guest_view_all_features`, which is
/// Plane's own switch): the page's statement reads the flag (issue #39), and so does the count that stands in for an empty page's
/// total, so the total a guest is told is the count of what they may see on every kind of page. Exhaustive over both values of the
/// flag, and over the four askers and the three kinds of page; the in-memory adapter's own is the core's
/// `a_guest_reads_only_what_they_made_unless_the_project_shows_them_everything`.
#[tokio::test]
async fn a_limited_guests_page_is_what_they_made_unless_the_project_shows_guests_everything() {
    let env = env().await;
    let (_, project, owner, s) = project_with_states(&env, "guest-flag").await;
    let guest = UserId(new_id());
    put_item(&env, s, "made by the guest", |w, _| w.created_by = Some(guest)).await.unwrap();
    put_item(&env, s, "made by the owner", |_, _| {}).await.unwrap();
    put_item(&env, s, "made by nobody", |w, _| w.created_by = None).await.unwrap();
    let order = WorkItemOrder { field: WorkItemOrderField::SequenceId, descending: false };
    let ask = |limited_guest, limit, page| WorkItemQuery { order, limit, page, updated_after: None, limited_guest };
    let raw = env.conn.raw().await.unwrap();
    for guests_see_all in [false, true] {
        raw.execute("UPDATE projects SET guest_view_all_features = $1 WHERE id = $2", &[&guests_see_all, &project.0])
            .await
            .unwrap();
        // who asks, and how many of the three items they may see: everyone sees all; a limited guest sees what they made (the
        // guest one, the owner one, none for a stranger), or all three when the project shows its guests everything
        let all = 3;
        let askers = [
            (None, all),
            (Some(guest), if guests_see_all { all } else { 1 }),
            (Some(owner), if guests_see_all { all } else { 1 }),
            (Some(UserId(new_id())), if guests_see_all { all } else { 0 }),
        ];
        for (who, sees) in askers {
            // a page of them all, one that cuts them, and one past the end (no row carries the total: the count statement does)
            for (limit, page, rows) in [(10, 0, sees), (1, 0, sees.min(1)), (10, 5, 0)] {
                let got = env.db.work_item_page(&s, &ask(who, limit, page)).await.unwrap();
                assert_eq!(
                    (got.rows.len() as u64, got.total),
                    (rows, sees),
                    "guests see all: {guests_see_all}; asker {who:?}; limit {limit} page {page}: {got:?}"
                );
            }
        }
    }
}

/// A work item is found by its number and by its integration key when it is live and of the scope's own project, and by nothing else.
#[tokio::test]
async fn a_work_item_is_found_by_its_number_and_its_key_when_it_is_live_and_of_the_scopes_own_project() {
    let env = env().await;
    let (_, _, _, s) = project_with_states(&env, "wi-lookups").await;
    let key = |source: &'static str, id: &'static str| {
        move |w: &mut WorkItem, _: &mut WorkItemAttrs| {
            w.external_source = Some(source.into());
            w.external_id = Some(id.into());
        }
    };
    let live = put_item(&env, s, "live", key("gh", "1")).await.unwrap();
    let archived = put_item(&env, s, "archived", |w, _| {
        w.archived_at = Some(Utc.timestamp_opt(1_800_000_050, 0).unwrap());
        w.external_source = Some("gh".into());
        w.external_id = Some("2".into());
    })
    .await
    .unwrap();
    let draft = put_item(&env, s, "draft", |w, _| {
        w.is_draft = true;
        w.external_source = Some("gh".into());
        w.external_id = Some("3".into());
    })
    .await
    .unwrap();
    let deleted = put_item(&env, s, "deleted", key("gh", "4")).await.unwrap();
    assert!(delete_item(&env, s, deleted).await);
    let number = |id: WorkItemId| {
        let env = &env;
        async move { env.db.get_work_item(&s, id, Visibility::WithDeleted).await.unwrap().unwrap().sequence_id }
    };
    let by_number = |s: Scope, n: i64| {
        let env = &env;
        async move { env.db.work_item_by_sequence(&s, n).await.unwrap().map(|r| r.item.id) }
    };
    let by_key = |s: Scope, source: &'static str, id: &'static str| {
        let env = &env;
        async move { env.db.work_item_by_external(&s, source, id).await.unwrap().map(|r| r.item.id) }
    };
    assert_eq!(by_number(s, number(live).await).await, Some(live));
    assert_eq!(by_key(s, "gh", "1").await, Some(live));
    for (gone, i) in [(archived, "2"), (draft, "3"), (deleted, "4")] {
        assert_eq!(by_number(s, number(gone).await).await, None, "{gone:?} is not live");
        assert_eq!(by_key(s, "gh", i).await, None, "{gone:?} is not live");
    }
    assert_eq!((by_key(s, "GH", "1").await, by_key(s, "gh", "11").await, by_number(s, 999).await), (None, None, None));
    // another project's scope finds neither
    let (_, _, _, other) = project_with_states(&env, "wi-lookups-2").await;
    assert_eq!((by_number(other, number(live).await).await, by_key(other, "gh", "1").await), (None, None));
}

/// A deleted work item is marked, and not removed, together with the live work items under it (the walk ends on a work item that is
/// its own parent, and on a loop); an archived work item and a draft are there to delete, though a read does not find them; and
/// nothing outside the project is touched.
#[tokio::test]
async fn a_deleted_work_item_takes_the_work_items_under_it_and_ends_on_a_loop() {
    let env = env().await;
    let (_, _, owner, s) = project_with_states(&env, "wi-delete").await;
    let under = |p: WorkItemId| move |_: &mut WorkItem, a: &mut WorkItemAttrs| a.parent_id = Some(p.0);
    let root = put_item(&env, s, "root", |_, _| {}).await.unwrap();
    let child = put_item(&env, s, "child", under(root)).await.unwrap();
    let grandchild = put_item(&env, s, "grandchild", under(child)).await.unwrap();
    let sibling = put_item(&env, s, "sibling", |_, _| {}).await.unwrap();
    let (_, _, _, elsewhere_scope) = project_with_states(&env, "wi-delete-2").await;
    let elsewhere = put_item(&env, elsewhere_scope, "elsewhere", |_, _| {}).await.unwrap();
    let there = |id: WorkItemId, s: Scope| {
        let env = &env;
        async move { env.db.work_item_row(&s, id).await.unwrap().is_some() }
    };

    assert!(delete_item(&env, s, child).await);
    assert!(!there(child, s).await && !there(grandchild, s).await, "the child and what is under it go");
    assert!(there(root, s).await && there(sibling, s).await && there(elsewhere, elsewhere_scope).await);
    // the rows stay, marked, and a second deletion, another project's item and one that is nowhere are not there
    let raw = env.conn.raw().await.unwrap();
    let marked: i64 = raw
        .query_one(
            "SELECT count(*) FROM work_items WHERE project_id = $1 AND deleted_at IS NOT NULL",
            &[&s.project_id().0],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(marked, 2);
    assert!(!delete_item(&env, s, child).await);
    assert!(!delete_item(&env, s, elsewhere).await);
    assert!(!delete_item(&env, s, WorkItemId(new_id())).await);

    // a work item that is its own parent, and two that are each other's: the walk ends, and takes the whole loop
    let (a, b) = (put_item(&env, s, "a", |_, _| {}).await.unwrap(), put_item(&env, s, "b", |_, _| {}).await.unwrap());
    let r = |id: WorkItemId| {
        let env = &env;
        async move { env.db.work_item_row(&s, id).await.unwrap().unwrap() }
    };
    assert!(update_item(&env, s, a, &whole(&r(a).await, Some(a.0), None, None, owner)).await.unwrap());
    assert!(update_item(&env, s, b, &whole(&r(b).await, Some(a.0), None, None, owner)).await.unwrap());
    assert!(update_item(&env, s, a, &whole(&r(a).await, Some(b.0), None, None, owner)).await.unwrap());
    assert!(delete_item(&env, s, b).await);
    assert!(!there(a, s).await && !there(b, s).await, "the loop goes whole, from either end");
    assert!(there(root, s).await);

    // an archived item and a draft are there to delete, though a read does not find them
    let archived =
        put_item(&env, s, "archived", |w, _| w.archived_at = Some(Utc.timestamp_opt(1_800_000_050, 0).unwrap()))
            .await
            .unwrap();
    let draft = put_item(&env, s, "draft", |w, _| w.is_draft = true).await.unwrap();
    assert!(delete_item(&env, s, archived).await && delete_item(&env, s, draft).await);
    assert!(!delete_item(&env, s, archived).await);
}

/// The search finds, among the live work items of the workspace's projects that the caller is in and that are not archived, those
/// whose name or project identifier contains the text in any case or whose number is asked for, newest first, with `%` and `_` as
/// ordinary characters.
#[tokio::test]
async fn the_workspace_search_finds_what_a_name_an_identifier_or_a_number_says_in_the_callers_own_live_projects() {
    let env = env().await;
    let (ws, first, owner, s1) = project_with_states(&env, "wi-search").await;
    let second = project_for(ws, owner, "Second", "SEC");
    env.db.insert_project(&WorkspaceScope::mint(ws, owner, Role::Admin), &second).await.unwrap();
    let s2 = Scope::mint(ws, second.project.id, owner);
    put_item(&env, s1, "Alpha login", |_, _| {}).await.unwrap();
    put_item(&env, s1, "Beta sign-up", |_, _| {}).await.unwrap();
    put_item(&env, s1, "100% done_ish", |_, _| {}).await.unwrap();
    put_item(&env, s1, "Archived login", |w, _| w.archived_at = Some(Utc.timestamp_opt(1_800_000_050, 0).unwrap()))
        .await
        .unwrap();
    put_item(&env, s1, "Draft login", |w, _| w.is_draft = true).await.unwrap();
    let gone = put_item(&env, s1, "Deleted login", |_, _| {}).await.unwrap();
    assert!(delete_item(&env, s1, gone).await);
    // made later than every item of the first project, so that "newest first" has no tie to break
    put_item(&env, s2, "Second login", |w, _| w.created_at = Utc.timestamp_opt(1_800_005_000, 0).unwrap())
        .await
        .unwrap();

    let ws_scope = |user: UserId| WorkspaceScope::mint(ws, user, Role::Member);
    let search = |user: UserId, text: &str, numbers: Vec<i64>, project: Option<ProjectId>, limit: u32| {
        let (env, text) = (&env, text.to_string());
        async move {
            let q = WorkspaceSearch { text, numbers, project, limit };
            env.db.search_workspace_work_items(&ws_scope(user), &q).await.unwrap()
        }
    };
    let names = |hits: Vec<WorkItemHit>| hits.into_iter().map(|h| h.name).collect::<Vec<_>>();
    // a name, in any case; the second project's item was made later, so it is first
    assert_eq!(names(search(owner, "LOGIN", vec![], None, 10).await), ["Second login", "Alpha login"]);
    // a project narrows it, and one the caller is not in has no hits (a stranger is in no project)
    assert_eq!(names(search(owner, "login", vec![], Some(first), 10).await), ["Alpha login"]);
    let stranger = user_without_login(&env).await;
    assert!(search(stranger, "login", vec![], None, 10).await.is_empty());
    // a number is a work item's number in any project the caller is in
    assert_eq!(names(search(owner, "no such text", vec![1], None, 10).await), ["Second login", "Alpha login"]);
    // `%` and `_` are ordinary characters
    assert_eq!(names(search(owner, "100%", vec![], None, 10).await), ["100% done_ish"]);
    assert_eq!(names(search(owner, "done_ish", vec![], None, 10).await), ["100% done_ish"]);
    assert!(search(owner, "a_b", vec![], None, 10).await.is_empty());
    assert_eq!(search(owner, "login", vec![], None, 1).await.len(), 1);
    // the identifier of a project is what a hit says, and what a search finds
    let hits = search(owner, "sta", vec![], None, 10).await;
    assert_eq!(hits.len(), 3, "every live item of the first project, whatever it is called");
    let slug = w1_slug(&env, ws).await;
    assert!(hits.iter().all(|h| h.project_identifier == "STA" && h.project_id == first && h.workspace_slug == slug));
    // an archived project is not searched
    let admin = WorkspaceScope::mint(ws, owner, Role::Admin);
    let mut p = env.db.project_detail(&admin, second.project.id).await.unwrap().unwrap().listing.project;
    p.archived_at = Some(Utc.timestamp_opt(1_800_000_900, 0).unwrap());
    assert!(env.db.update_project(&admin, &p).await.unwrap());
    assert_eq!(names(search(owner, "login", vec![], None, 10).await), ["Alpha login"]);
}

/// The slug a workspace was made with (a hit says it).
async fn w1_slug(env: &Env, ws: WorkspaceId) -> WorkspaceSlug {
    let raw = env.conn.raw().await.unwrap();
    WorkspaceSlug::new(
        raw.query_one("SELECT slug FROM workspaces WHERE id = $1", &[&ws.0]).await.unwrap().get::<_, String>(0),
    )
}

// --- labels (issue #58, migration 0015) ---------------------------------------------------------------------------------------
// The memory adapter is held to the same answers by `tests/projects.rs` in its own crate.

fn label_for(ws: WorkspaceId, project: ProjectId, owner: UserId, name: &str, sort_order: f64) -> LabelRow {
    let now = Utc.timestamp_opt(1_800_000_700, 0).unwrap();
    LabelRow {
        id: new_id(),
        workspace_id: ws,
        project_id: project,
        parent_id: None,
        name: name.into(),
        description: String::new(),
        color: "#abcdef".into(),
        sort_order,
        created_by: Some(owner),
        created_at: now,
        updated_at: now,
        external_source: None,
        external_id: None,
        updated_by: None,
    }
}

async fn label_names(env: &Env, s: &Scope) -> Vec<String> {
    env.db.project_labels(s).await.unwrap().into_iter().map(|l| l.name).collect()
}

/// A new label goes after the greatest live one by 10000, whatever it asked for (the first keeps its place); a label's name is unique
/// among the live labels of its project (another case is another name, another project's labels do not count, a deleted label's name
/// is free); and a change replaces what it names, keeps who made the label and when, and finds only a live label of the project.
#[tokio::test]
async fn a_new_label_is_placed_as_plane_places_it_and_its_name_is_unique_among_the_live_labels_of_its_project() {
    let env = env().await;
    let (ws, project, owner, s) = project_with_states(&env, "label-place").await;
    let new = |name: &str, sort_order: f64| label_for(ws, project, owner, name, sort_order);
    let at = Utc.timestamp_opt(1_800_000_800, 0).unwrap();

    let first = env.db.insert_label(&s, &new("bug", 65535.0)).await.unwrap();
    assert_eq!(first.sort_order, 65535.0);
    assert_eq!(env.db.project_labels(&s).await.unwrap(), vec![first.clone()], "what is returned is what is stored");
    assert_eq!(env.db.insert_label(&s, &new("feature", 1.0)).await.unwrap().sort_order, 75535.0);
    let last = env.db.insert_label(&s, &new("docs", 9e9)).await.unwrap();
    assert_eq!(last.sort_order, 85535.0);
    // a deleted label is not counted
    assert!(env.db.delete_label(&s, last.id, at).await.unwrap());
    assert_eq!(env.db.insert_label(&s, &new("ops", 0.0)).await.unwrap().sort_order, 85535.0);

    // the name
    let taken = |r: Result<LabelRow, DbError>| matches!(r, Err(DbError::Conflict(c)) if c.contains("labels_project_id_name_key"));
    assert!(taken(env.db.insert_label(&s, &new("bug", 1.0)).await));
    env.db.insert_label(&s, &new("Bug", 1.0)).await.unwrap();
    let (ws2, project2, owner2, s2) = project_with_states(&env, "label-place-2").await;
    env.db.insert_label(&s2, &label_for(ws2, project2, owner2, "bug", 1.0)).await.unwrap();
    assert!(env.db.delete_label(&s, first.id, at).await.unwrap());
    env.db.insert_label(&s, &new("bug", 1.0)).await.unwrap();

    // a change
    let feature = env.db.project_labels(&s).await.unwrap().into_iter().find(|l| l.name == "feature").unwrap();
    let mut asked = LabelRow {
        parent_id: Some(feature.id),
        name: "defect".into(),
        description: "d".into(),
        color: "#000".into(),
        sort_order: 1.5,
        external_source: Some("gh".into()),
        external_id: Some("7".into()),
        updated_by: Some(owner),
        updated_at: at,
        ..env.db.project_labels(&s).await.unwrap().into_iter().find(|l| l.name == "bug").unwrap()
    };
    let stored_before = env.db.project_labels(&s).await.unwrap().into_iter().find(|l| l.id == asked.id).unwrap();
    asked.created_at = at;
    asked.created_by = None;
    assert!(env.db.update_label(&s, &asked).await.unwrap());
    let stored = env.db.project_labels(&s).await.unwrap().into_iter().find(|l| l.id == asked.id).unwrap();
    assert_eq!(
        stored,
        LabelRow { created_at: stored_before.created_at, created_by: stored_before.created_by, ..asked.clone() }
    );
    let onto = LabelRow { name: "Bug".into(), ..stored.clone() };
    let got = env.db.update_label(&s, &onto).await;
    assert!(matches!(got, Err(DbError::Conflict(_))), "{got:?}");
    assert!(env.db.update_label(&s, &stored).await.unwrap(), "and its own name is not another's");
    // a label that is nowhere, one of another project and a deleted one are not there to change
    assert!(!env.db.update_label(&s, &LabelRow { id: new_id(), ..stored.clone() }).await.unwrap());
    let other = env.db.project_labels(&s2).await.unwrap().remove(0);
    assert!(!env.db.update_label(&s, &other).await.unwrap());
    assert!(env.db.delete_label(&s, stored.id, at).await.unwrap());
    assert!(!env.db.update_label(&s, &stored).await.unwrap());
    // a parent that is no label is refused by the foreign key, and changes nothing
    let live = env.db.project_labels(&s).await.unwrap().remove(0);
    let orphan = LabelRow { parent_id: Some(new_id()), ..live.clone() };
    assert!(env.db.update_label(&s, &orphan).await.is_err());
    assert_eq!(env.db.project_labels(&s).await.unwrap().remove(0), live);
}

/// A deleted label is marked and not removed, together with the live labels under it (the walk ends on a label that is its own parent,
/// and on a loop), and the work items lose their hold on every one of them: no join row of theirs is left. A label of another project
/// is not touched, and a second deletion, another project's label and a label that is nowhere are not there.
#[tokio::test]
async fn a_deleted_label_takes_the_labels_under_it_and_the_work_items_hold_on_them() {
    let env = env().await;
    let (ws, project, owner, s) = project_with_states(&env, "label-delete").await;
    let at = Utc.timestamp_opt(1_800_000_800, 0).unwrap();
    let make =
        |name: &str, parent: Option<Uuid>| LabelRow { parent_id: parent, ..label_for(ws, project, owner, name, 1.0) };
    let parent = env.db.insert_label(&s, &make("parent", None)).await.unwrap();
    let child = env.db.insert_label(&s, &make("child", Some(parent.id))).await.unwrap();
    let grandchild = env.db.insert_label(&s, &make("grandchild", Some(child.id))).await.unwrap();
    let sibling = env.db.insert_label(&s, &make("sibling", None)).await.unwrap();
    let hold = |labels: Vec<Uuid>| async {
        let attrs = WorkItemAttrs { label_ids: labels, ..WorkItemAttrs::bare(at) };
        let mut tx = env.db.begin(&s).await.unwrap();
        let a = tx.allocate(&s).await.unwrap();
        let id = insert_with_attrs(&mut tx, s, a, &attrs).await.unwrap();
        tx.commit().await.unwrap();
        id
    };
    let wanted = |labels: &[&LabelRow]| {
        let mut ids: Vec<Uuid> = labels.iter().map(|l| l.id).collect();
        ids.sort();
        ids
    };
    let (both, deep, kept) = (
        hold(wanted(&[&parent, &sibling])).await,
        hold(wanted(&[&grandchild, &sibling])).await,
        hold(wanted(&[&sibling])).await,
    );
    let held = |id: WorkItemId| {
        let (env, s) = (&env, s);
        async move { env.db.work_item_row(&s, id).await.unwrap().unwrap().attrs.label_ids }
    };

    // another project's label of the same name
    let (ws2, project2, owner2, s2) = project_with_states(&env, "label-delete-2").await;
    let other = env.db.insert_label(&s2, &label_for(ws2, project2, owner2, "child", 1.0)).await.unwrap();

    // the child, and so the grandchild; the parent and the sibling stay
    assert!(env.db.delete_label(&s, child.id, at).await.unwrap());
    assert_eq!(label_names(&env, &s).await, ["parent", "sibling"]);
    assert_eq!(held(deep).await, [sibling.id]);
    assert_eq!(held(both).await, wanted(&[&parent, &sibling]));
    // the rows are marked, not removed, and the join rows of the three are gone
    let raw = env.conn.raw().await.unwrap();
    let marked: i64 = raw
        .query_one("SELECT count(*) FROM labels WHERE project_id = $1 AND deleted_at IS NOT NULL", &[&project.0])
        .await
        .unwrap()
        .get(0);
    assert_eq!(marked, 2);
    let joins: i64 = raw
        .query_one("SELECT count(*) FROM work_item_labels WHERE label_id = ANY($1)", &[&vec![child.id, grandchild.id]])
        .await
        .unwrap()
        .get(0);
    assert_eq!(joins, 0);
    // the parent, then; and the sibling's items keep the sibling
    assert!(env.db.delete_label(&s, parent.id, at).await.unwrap());
    assert_eq!(held(both).await, [sibling.id]);
    assert_eq!(held(kept).await, [sibling.id]);
    // another project's label is not touched; a second deletion, a label of another project and one that is nowhere are not there
    assert_eq!(env.db.project_labels(&s2).await.unwrap(), vec![other.clone()]);
    assert!(!env.db.delete_label(&s, parent.id, at).await.unwrap());
    assert!(!env.db.delete_label(&s, other.id, at).await.unwrap());
    assert!(!env.db.delete_label(&s, new_id(), at).await.unwrap());
    // the names are free
    env.db.insert_label(&s, &make("parent", None)).await.unwrap();
    env.db.insert_label(&s, &make("child", None)).await.unwrap();

    // a label that is its own parent, and two that are each other's: the walk ends, and takes the whole loop
    let a = env.db.insert_label(&s, &make("a", None)).await.unwrap();
    let b = env.db.insert_label(&s, &make("b", None)).await.unwrap();
    let c = env.db.insert_label(&s, &make("c", None)).await.unwrap();
    assert!(env.db.update_label(&s, &LabelRow { parent_id: Some(a.id), ..a.clone() }).await.unwrap());
    assert!(env.db.update_label(&s, &LabelRow { parent_id: Some(c.id), ..b.clone() }).await.unwrap());
    assert!(env.db.update_label(&s, &LabelRow { parent_id: Some(b.id), ..c.clone() }).await.unwrap());
    let all = hold(wanted(&[&a, &b, &c, &sibling])).await;
    assert!(env.db.delete_label(&s, a.id, at).await.unwrap());
    assert!(!label_names(&env, &s).await.contains(&"a".to_string()));
    assert!(env.db.delete_label(&s, c.id, at).await.unwrap());
    let names = label_names(&env, &s).await;
    assert!(!names.contains(&"b".to_string()) && !names.contains(&"c".to_string()), "{names:?}");
    assert_eq!(held(all).await, [sibling.id]);
}

/// Issue #58, migration 0015: the name of a label is unique among the live ones by an index that took the place of 0012's table
/// constraint, and a start does it once (see the same test for states).
#[tokio::test]
async fn migration_0015_replaces_the_label_name_constraint_once_and_a_restart_keeps_the_index() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let name = "labels_project_id_name_key";
    let index_def = || {
        let raw = &raw;
        async move {
            raw.query_one("SELECT indexdef FROM pg_indexes WHERE schemaname = 'public' AND tablename = 'labels' AND indexname = $1", &[&name])
                .await
                .unwrap()
                .get::<_, String>(0)
        }
    };
    let constraints = || {
        let raw = &raw;
        async move {
            raw.query_one("SELECT count(*) FROM pg_constraint WHERE conname = $1", &[&name])
                .await
                .unwrap()
                .get::<_, i64>(0)
        }
    };
    let oid = || {
        let raw = &raw;
        async move {
            raw.query_one("SELECT oid::int8 FROM pg_class WHERE relname = $1", &[&name]).await.unwrap().get::<_, i64>(0)
        }
    };
    assert!(index_def().await.contains("deleted_at IS NULL"));
    assert_eq!(constraints().await, 0, "after migrating there is the index and no constraint");

    // a database from before 0015: the table constraint 0012 made
    raw.batch_execute(&format!(
        "DROP INDEX {name}; ALTER TABLE labels ADD CONSTRAINT {name} UNIQUE (project_id, name);"
    ))
    .await
    .unwrap();
    assert_eq!(constraints().await, 1);
    assert!(!index_def().await.contains("deleted_at"));
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    assert!(index_def().await.contains("deleted_at IS NULL"), "the constraint was not replaced");
    assert_eq!(constraints().await, 0);

    // a restart makes nothing again
    let before = oid().await;
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    raw.batch_execute(thrust_adapters_postgres::MIGRATION_0012).await.unwrap();
    assert_eq!(oid().await, before, "a restart made the index again");
    assert_eq!(constraints().await, 0, "a restart made the constraint again");
}

// --- the oracle's seed and `reset` (issue #58) --------------------------------------------------------------------------------

/// The oracle's seed (`thrust-oracle-fixture`), made at a fixed instant: two projects, their states and labels, eleven work items
/// with people, labels, a parent and dates, and the numbers and places the oracle's Plane gave them.
fn oracle_fixture() -> SeedFixture {
    thrust_oracle_fixture::oracle_seed(Utc.timestamp_opt(1_800_000_000, 0).unwrap()).fixture
}

/// Each table of the `public` schema and how many rows it has, by name: the schema as the migrations made it, so that a table a later
/// migration adds is counted without this test being told.
async fn row_counts(raw: &tokio_postgres::Client) -> Vec<(String, i64)> {
    let tables =
        raw.query("SELECT tablename FROM pg_tables WHERE schemaname = 'public' ORDER BY tablename", &[]).await.unwrap();
    let mut out = Vec::new();
    for t in tables {
        let name: String = t.get(0);
        let n: i64 = raw.query_one(&format!("SELECT count(*) FROM \"{name}\""), &[]).await.unwrap().get(0);
        out.push((name, n));
    }
    out
}

fn sorted_ids<T: Ord + Clone>(v: &[T]) -> Vec<T> {
    let mut out = v.to_vec();
    out.sort();
    out.dedup();
    out
}

/// A database turns the oracle's fixture into the rows it says: each work item with its number, place, text, time, state, priority,
/// dates, people, labels and parent; each project with its key, description and default state; and each project's counters carry on
/// from the largest number and place of its items (the project's next number is 10 and the next place after the largest, as the
/// oracle's own next create was).
#[tokio::test]
async fn the_oracles_seed_is_stored_as_it_was_given() {
    let env = env().await;
    let f = oracle_fixture();
    env.db.seed(&SystemScope::mint(), &f).await.unwrap();
    for w in &f.work_items {
        let s = scope(w.workspace_id, w.project_id);
        let row = env.db.work_item_row(&s, w.id).await.unwrap().unwrap_or_else(|| panic!("{} is not there", w.name));
        let which = &w.name;
        assert_eq!(row.item.sequence_id, w.sequence_id, "{which}: number");
        assert_eq!(row.item.sort_order, w.sort_order, "{which}: place");
        assert_eq!(row.item.name, w.name);
        assert_eq!(row.item.description, w.description, "{which}: description");
        assert_eq!(row.item.created_by, w.created_by, "{which}: maker");
        assert_eq!(row.item.created_at, w.created_at, "{which}: made at");
        assert_eq!(row.attrs.state_id, w.state_id, "{which}: state");
        assert_eq!(row.attrs.priority, w.priority, "{which}: priority");
        assert_eq!(row.attrs.start_date, w.start_date, "{which}: start");
        assert_eq!(row.attrs.target_date, w.target_date, "{which}: target");
        assert_eq!(row.attrs.completed_at, w.completed_at, "{which}: completed at");
        assert_eq!(row.attrs.description_html, w.description_html, "{which}: description_html");
        assert_eq!(row.attrs.parent_id, w.parent_id, "{which}: parent");
        assert_eq!(row.attrs.assignee_ids, sorted_ids(&w.assignee_ids), "{which}: people");
        assert_eq!(row.attrs.label_ids, sorted_ids(&w.label_ids), "{which}: labels");
        let group = w.state_id.and_then(|st| f.states.iter().find(|x| x.id == st)).map(|x| x.group);
        assert_eq!(row.state_group, group, "{which}: the group of its state");
    }

    // the projects' own columns
    let raw = env.conn.raw().await.unwrap();
    for d in &f.project_details {
        let r = raw
            .query_one("SELECT description, default_state, identifier FROM projects WHERE id = $1", &[&d.project_id.0])
            .await
            .unwrap();
        assert_eq!(r.get::<_, String>(0), d.description);
        assert_eq!(r.get::<_, Option<Uuid>>(1), d.default_state);
        assert_eq!(Some(r.get::<_, String>(2)), d.identifier);
    }

    // the counters: a project's next number is one past its largest, and the next place ten thousand past
    for p in &f.projects {
        let ours: Vec<&SeedIssue> = f.work_items.iter().filter(|w| w.project_id == p.id).collect();
        let s = scope(p.workspace_id, p.id);
        let mut tx = env.db.begin(&s).await.unwrap();
        let next = tx.allocate(&s).await.unwrap();
        assert_eq!(next.sequence_id, ours.iter().map(|w| w.sequence_id).max().unwrap() + 1, "{}: next number", p.name);
        assert_eq!(
            next.sort_order,
            ours.iter().map(|w| w.sort_order).max().unwrap() + 10_000,
            "{}: next place",
            p.name
        );
        tx.rollback().await.unwrap();
        // the largest place among the items of one state, which is what the pin puts a new item's place after
        let backlog = f.states.iter().find(|x| x.project_id == p.id && x.is_default).unwrap().id;
        let largest = ours.iter().filter(|w| w.state_id == Some(backlog)).map(|w| w.sort_order).max();
        assert_eq!(env.db.max_work_item_sort_order(&s, Some(backlog)).await.unwrap(), largest);
    }
}

/// `reset` empties every table of the database (counted from the schema, so a table that a later migration adds is counted too),
/// leaves the tables, and what it emptied loads again to the same rows. The control is in the middle: before the reset the tables
/// are not empty, so that "none has a row" is a result and not what a database nobody wrote to says.
#[tokio::test]
async fn a_reset_empties_every_table_and_the_same_fixture_loads_again() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let sys = SystemScope::mint();
    let fresh = row_counts(&raw).await;
    assert!(fresh.len() > 20, "the schema has its tables: {fresh:?}");
    assert!(fresh.iter().all(|(_, n)| *n == 0), "a database that was just made has no rows: {fresh:?}");

    let f = oracle_fixture();
    env.db.seed(&sys, &f).await.unwrap();
    let loaded = row_counts(&raw).await;
    let filled = loaded.iter().filter(|(_, n)| *n > 0).count();
    assert!(
        filled >= 10,
        "the control: the oracle's fixture fills at least ten tables, and filled {filled}: {loaded:?}"
    );

    env.db.reset(&sys).await.unwrap();
    let emptied = row_counts(&raw).await;
    assert_eq!(
        emptied.iter().map(|(t, _)| t).collect::<Vec<_>>(),
        loaded.iter().map(|(t, _)| t).collect::<Vec<_>>(),
        "the tables are all still there"
    );
    assert!(emptied.iter().all(|(_, n)| *n == 0), "a table still has rows: {emptied:?}");

    // the same fixture, into what is left: no clash, the same rows, and the counters start from where the first load put them
    env.db.seed(&sys, &f).await.unwrap();
    assert_eq!(row_counts(&raw).await, loaded);
    let p = &f.projects[0];
    let s = scope(p.workspace_id, p.id);
    let mut tx = env.db.begin(&s).await.unwrap();
    let next = tx.allocate(&s).await.unwrap();
    let largest = f.work_items.iter().filter(|w| w.project_id == p.id).map(|w| w.sequence_id).max().unwrap();
    assert_eq!(next.sequence_id, largest + 1, "a reset left a counter behind, or took one");
    tx.rollback().await.unwrap();

    // a reset of what is already empty is no error
    env.db.reset(&sys).await.unwrap();
    env.db.reset(&sys).await.unwrap();
}

/// What a fixture can get wrong, one way at a time, is refused as a request would be, and a seed is all or nothing: after each
/// refusal not one row of the fixture is left (the rows before the bad one were rolled back). Each case starts from a reset.
#[tokio::test]
async fn a_fixture_the_database_refuses_leaves_no_row_of_it() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let sys = SystemScope::mint();
    let nobody = Uuid::from_u128(0xDEAD_BEEF);
    // each is: its name, what it does to the fixture, and whether the refusal is a unique key's (`Conflict`) or another constraint's
    #[allow(clippy::type_complexity)]
    let cases: Vec<(&str, Box<dyn Fn(&mut SeedFixture)>, bool)> = vec![
        ("a number a project has", Box::new(|f| f.work_items[1].sequence_id = f.work_items[0].sequence_id), true),
        ("a state that is not there", Box::new(move |f| f.work_items[0].state_id = Some(nobody)), false),
        ("a person who is not there", Box::new(move |f| f.work_items[1].assignee_ids.push(UserId(nobody))), false),
        ("a label that is not there", Box::new(move |f| f.work_items[1].label_ids.push(nobody)), false),
        ("a parent that is not there", Box::new(move |f| f.work_items[0].parent_id = Some(nobody)), false),
        ("a parent that comes after its child", Box::new(|f| f.work_items.swap(2, 5)), false),
        (
            "a start after the target",
            Box::new(|f| {
                let target = f.work_items[1].target_date.unwrap();
                f.work_items[1].start_date = Some(target + thrust_contracts::chrono::Duration::days(1));
            }),
            false,
        ),
        (
            "a project of another workspace",
            Box::new(move |f| f.work_items[0].workspace_id = WorkspaceId(nobody)),
            false,
        ),
        // The adapter's own refusal, not Postgres's: the transaction is not aborted when it is raised, so only a ROLLBACK undoes the
        // rows the seed wrote before it. (When Postgres refuses, the transaction is aborted and a COMMIT would roll back by itself,
        // so the cases above cannot tell a seed that rolls back from one that commits.)
        (
            "a sort order for a project that is not there",
            Box::new(move |f| {
                let user_id = f.users[0];
                f.project_sort_orders.push(SeedProjectSortOrder {
                    project_id: ProjectId(nobody),
                    user_id,
                    sort_order: 1.0,
                });
            }),
            false,
        ),
    ];
    for (name, corrupt, conflict) in cases {
        env.db.reset(&sys).await.unwrap();
        let mut f = oracle_fixture();
        corrupt(&mut f);
        let got = env.db.seed(&sys, &f).await;
        match (&got, conflict) {
            (Err(DbError::Conflict(_)), true) | (Err(DbError::Backend(_)), false) => {}
            _ => panic!("{name}: expected the refusal of a constraint, got {got:?}"),
        }
        let left = row_counts(&raw).await;
        assert!(left.iter().all(|(_, n)| *n == 0), "{name}: rows are left of a refused seed: {left:?}");
    }
}
