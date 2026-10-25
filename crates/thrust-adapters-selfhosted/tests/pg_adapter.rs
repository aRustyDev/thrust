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
                None => raw
                    .execute(
                        "INSERT INTO api_keys (id, user_id, token_hash, prefix, label, created_at) \
                         VALUES ($1, $2, $3, 'thrk_x', '', now())",
                        &[&new_id(), &user, &vec![digest; 32]],
                    )
                    .await,
                Some(v) => raw
                    .execute(
                        "INSERT INTO api_keys (id, user_id, token_hash, hash_version, prefix, label, created_at) \
                         VALUES ($1, $2, $3, $4, 'thrk_x', '', now())",
                        &[&new_id(), &user, &vec![digest; 32], &v],
                    )
                    .await,
            }
        }
    };
    insert(10, None).await.expect("the default");
    assert_eq!(
        raw.query_one("SELECT hash_version FROM api_keys WHERE token_hash = $1", &[&vec![10u8; 32]]).await.unwrap().get::<_, i16>(0),
        1
    );
    insert(11, Some(i16::MAX)).await.expect("the largest version");
    assert!(insert(12, Some(0)).await.is_err(), "zero is not a version");
    assert!(insert(13, Some(-1)).await.is_err(), "a negative one is not either");
    assert!(
        raw.execute("UPDATE api_keys SET hash_version = NULL WHERE id = $1", &[&a]).await.is_err(),
        "NOT NULL"
    );
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
        .insert_user(&UserRecord { id: user, email: Some(format!("{}@example.test", user.0.simple())), password_hash: None, is_active: true })
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
    for (column, bad) in [("onboarding_step", "[]"), ("onboarding_step", "1"), ("preferences", "[]"), ("preferences", "\"x\"")] {
        let sql = format!("UPDATE user_profiles SET {column} = $2::jsonb WHERE user_id = $1");
        let refused = raw.execute(&sql, &[&user.0, &bad]).await;
        assert!(refused.is_err(), "{column} = {bad}");
    }
    // control: an object is accepted
    raw.execute("UPDATE user_profiles SET preferences = '{\"k\": 1}'::jsonb WHERE user_id = $1", &[&user.0]).await.unwrap();
}

#[tokio::test]
async fn migration_0008_is_idempotent_and_keeps_its_rows() {
    let env = env().await;
    let user = user_with_profile(&env).await;
    env.db
        .update_profile(user, &ProfilePatch { first_name: Some("Kept".into()), ..ProfilePatch::default() }, Utc.timestamp_opt(1_800_000_100, 0).unwrap())
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
    for (set, why) in [("role = 'owner'", "role"), ("props = '[]'::jsonb", "props array"), ("props = '1'::jsonb", "props scalar")] {
        let refused = raw.execute(&format!("UPDATE workspace_members SET {set} WHERE workspace_id = $1"), &[&ws]).await;
        assert!(refused.is_err(), "{why}");
    }
    // control: a role from the three and an object are accepted
    raw.execute("UPDATE workspace_members SET role = 'guest', props = '{\"k\": 1}'::jsonb WHERE workspace_id = $1", &[&ws]).await.unwrap();
    assert_eq!(env.db.workspace_member(&WorkspaceScope::mint(w.workspace.id, owner, Role::Guest)).await.unwrap().unwrap().role, Role::Guest);
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
    raw.execute("INSERT INTO workspace_members (workspace_id, user_id) VALUES ($1, $2), ($1, $3)", &[&ws, &u1, &u2]).await.unwrap();
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    let first = raw.query("SELECT user_id, id FROM workspace_members ORDER BY user_id", &[]).await.unwrap();
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    let again = raw.query("SELECT user_id, id FROM workspace_members ORDER BY user_id", &[]).await.unwrap();
    let ids = |rows: &[tokio_postgres::Row]| rows.iter().map(|r| (r.get::<_, Uuid>(0), r.get::<_, Uuid>(1))).collect::<Vec<_>>();
    assert_eq!(ids(&first), ids(&again), "a second run gave the members new ids");
    assert_ne!(ids(&first)[0].1, ids(&first)[1].1, "two members share an id");

    drop(raw);
    let db = PgDatabase::new(conn);
    let listed = db.workspaces_of(UserId(u1)).await.unwrap();
    assert_eq!(listed.len(), 1);
    let (w, role, total) = (&listed[0].workspace, listed[0].role, listed[0].total_members);
    assert_eq!((w.id, w.slug.as_str(), w.name.as_str(), w.owner_id), (WorkspaceId(ws), "legacy-space", "legacy-space", None));
    assert_eq!((w.timezone.as_str(), w.background_color.as_str(), w.organization_size.clone()), ("UTC", "#808080", None));
    assert_eq!((role, total), (Role::Member, 2));
    let m = db.workspace_member(&WorkspaceScope::mint(WorkspaceId(ws), UserId(u2), Role::Member)).await.unwrap().unwrap();
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
    raw.execute("INSERT INTO projects (id, workspace_id, name, identifier) VALUES ($1, $2, 'p', 'P')", &[&project, &ws]).await.unwrap();
    // a network outside the two words, and a logo that is not an object, are refused by the database itself
    for (set, why) in [("network = 'private'", "network"), ("logo_props = '[]'::jsonb", "logo array"), ("logo_props = '1'::jsonb", "logo scalar")] {
        assert!(raw.execute(&format!("UPDATE projects SET {set} WHERE id = $1"), &[&project]).await.is_err(), "{why}");
    }
    // a project has an identifier
    assert!(raw.execute("UPDATE projects SET identifier = NULL WHERE id = $1", &[&project]).await.is_err());
    // control: the two networks and an object are accepted
    raw.execute("UPDATE projects SET network = 'secret', logo_props = '{\"in_use\": \"emoji\"}'::jsonb WHERE id = $1", &[&project]).await.unwrap();

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
    assert!(raw.execute(home, &[&ws, &owner.0, &serde_json::json!([])]).await.is_err(), "a widget config that is not an object");
    raw.execute(home, &[&ws, &owner.0, &serde_json::json!({"a": 1})]).await.unwrap();
    let props = "INSERT INTO workspace_user_properties (id, workspace_id, user_id, filters, display_filters, display_properties, \
                   rich_filters, navigation_project_limit, navigation_control_preference, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, '{}', '{}', '{}', 10, $5, now(), now())";
    let object = serde_json::json!({});
    assert!(raw.execute(props, &[&new_id(), &ws, &owner.0, &serde_json::json!([]), &"ACCORDION"]).await.is_err(), "filters that are no object");
    assert!(raw.execute(props, &[&new_id(), &ws, &owner.0, &object, &"SIDEWAYS"]).await.is_err(), "a navigation control of another word");
    raw.execute(props, &[&new_id(), &ws, &owner.0, &object, &"TABBED"]).await.unwrap();
    assert!(raw.execute(props, &[&new_id(), &ws, &owner.0, &object, &"TABBED"]).await.is_err(), "two settings rows for one member");
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
    raw.execute("INSERT INTO workspaces (id, slug, name) VALUES ($1, 'legacy-projects', 'Legacy')", &[&ws]).await.unwrap();
    raw.execute("INSERT INTO workspace_members (workspace_id, user_id) VALUES ($1, $2)", &[&ws, &user]).await.unwrap();
    for p in projects {
        raw.execute("INSERT INTO projects (id, workspace_id, name) VALUES ($1, $2, $3)", &[&p, &ws, &format!("legacy {p}")]).await.unwrap();
        raw.execute("INSERT INTO project_members (project_id, workspace_id, user_id, role) VALUES ($1, $2, $3, 'member')", &[&p, &ws, &user])
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
    let listed = db.workspace_projects(&WorkspaceScope::mint(WorkspaceId(ws), UserId(user), Role::Member)).await.unwrap();
    assert_eq!(listed.len(), 3);
    for l in &listed {
        let p = &l.project;
        assert_eq!((p.network, p.archived_at, p.logo_props.clone(), p.project_lead, p.created_by), (ProjectNetwork::Public, None, serde_json::json!({}), None, None));
        assert_eq!((p.cycle_view, p.issue_views_view, p.module_view, p.page_view, p.intake_view, p.guest_view_all_features), (false, false, false, true, false, false));
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
    let sidebar: Vec<SidebarPreference> =
        (0..7).map(|i| SidebarPreference { key: format!("key{i}"), is_pinned: i % 2 == 0, sort_order: 65535.0 + 10000.0 * f64::from(i) }).collect();
    let home: Vec<HomePreference> = (0..3)
        .map(|i| HomePreference { key: format!("widget{i}"), is_enabled: true, config: serde_json::json!({}), sort_order: 999.0 - f64::from(i), created_at: now })
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
    for (table, want) in [("workspace_user_preferences", 7i64), ("workspace_home_preferences", 3), ("workspace_user_properties", 1)] {
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
        states: vec![state("Backlog", 15000.0, StateGroup::Backlog, true), state("Triage", 65000.0, StateGroup::Triage, false)],
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
            ("description", text),
            ("id", uuid),
            ("is_default", boolean),
            ("name", text),
            ("project_id", uuid),
            ("sequence", double),
            ("state_group", text),
            ("updated_at", ts),
            ("workspace_id", uuid),
        ])
    );
    assert_eq!(
        columns("labels").await,
        pairs(vec![
            ("color", text),
            ("created_at", ts),
            ("created_by", uuid),
            ("description", text),
            ("id", uuid),
            ("name", text),
            ("parent_id", uuid),
            ("project_id", uuid),
            ("sort_order", double),
            ("updated_at", ts),
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
    for (set, ok) in [("archive_in = 12", true), ("archive_in = 0", true), ("archive_in = 13", false), ("close_in = -1", false)] {
        assert_eq!(raw.execute(&format!("UPDATE projects SET {set} WHERE id = $1"), &[&project]).await.is_ok(), ok, "{set}");
    }
    // a name and a key are each unique in their workspace, and free in another
    let clash = |name: &str, identifier: &str| project_for(w1.workspace.id, owner, name, identifier);
    assert!(matches!(env.db.insert_project(&scope(&w1), &clash("First", "OTH")).await, Err(DbError::Conflict(_))), "the name");
    assert!(matches!(env.db.insert_project(&scope(&w1), &clash("Other", "FST")).await, Err(DbError::Conflict(_))), "the key");
    env.db.insert_project(&scope(&w2), &project_for(w2.workspace.id, owner, "First", "FST")).await.unwrap();
    // the users a project names are users
    let nobody = new_id();
    for column in ["created_by", "updated_by", "project_lead", "default_assignee"] {
        assert!(raw.execute(&format!("UPDATE projects SET {column} = $2 WHERE id = $1"), &[&project, &nobody]).await.is_err(), "{column}");
    }
    // settings are objects
    for set in ["view_props = '[]'::jsonb", "default_props = '1'::jsonb", "preferences = 'null'::jsonb"] {
        assert!(raw.execute(&format!("UPDATE project_members SET {set} WHERE project_id = $1"), &[&project]).await.is_err(), "{set}");
    }
    for set in ["filters = '[]'::jsonb", "display_filters = '1'::jsonb", "display_properties = '\"x\"'::jsonb", "rich_filters = '[]'::jsonb", "preferences = '1'::jsonb"] {
        assert!(raw.execute(&format!("UPDATE project_user_properties SET {set} WHERE project_id = $1"), &[&project]).await.is_err(), "{set}");
    }
    // control: objects are accepted
    raw.execute("UPDATE project_members SET view_props = '{\"a\": 1}'::jsonb WHERE project_id = $1", &[&project]).await.unwrap();
    // a state: a group from the six, a name once per project, one default, a project of its own workspace
    let state = "INSERT INTO states (id, workspace_id, project_id, name, color, sequence, state_group, is_default, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, '#fff', 1, $5, $6, now(), now())";
    let ws1 = w1.workspace.id.0;
    assert!(raw.execute(state, &[&new_id(), &ws1, &project, &"Review", &"reviewing", &false]).await.is_err(), "a group of its own");
    assert!(raw.execute(state, &[&new_id(), &ws1, &project, &"Backlog", &"backlog", &false]).await.is_err(), "a name twice");
    assert!(raw.execute(state, &[&new_id(), &ws1, &project, &"Second default", &"started", &true]).await.is_err(), "two default states");
    assert!(raw.execute(state, &[&new_id(), &w2.workspace.id.0, &project, &"Elsewhere", &"started", &false]).await.is_err(), "a state in another workspace than its project's");
    raw.execute(state, &[&new_id(), &ws1, &project, &"Review", &"started", &false]).await.unwrap();
    // a label: a name once per project, a parent that exists, a project of its own workspace
    let label = "INSERT INTO labels (id, workspace_id, project_id, parent_id, name, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, now(), now())";
    let parent = new_id();
    raw.execute(label, &[&parent, &ws1, &project, &None::<Uuid>, &"bug"]).await.unwrap();
    assert!(raw.execute(label, &[&new_id(), &ws1, &project, &None::<Uuid>, &"bug"]).await.is_err(), "a label name twice");
    assert!(raw.execute(label, &[&new_id(), &ws1, &project, &Some(new_id()), &"child"]).await.is_err(), "a parent that is no label");
    assert!(raw.execute(label, &[&new_id(), &w2.workspace.id.0, &project, &None::<Uuid>, &"x"]).await.is_err(), "a label in another workspace than its project's");
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
    raw.execute("INSERT INTO workspaces (id, slug, name) VALUES ($1, 'legacy-members', 'Legacy')", &[&ws]).await.unwrap();
    raw.execute("INSERT INTO workspace_members (workspace_id, user_id) VALUES ($1, $2), ($1, $3)", &[&ws, &u1, &u2]).await.unwrap();
    raw.execute("INSERT INTO projects (id, workspace_id, name, identifier) VALUES ($1, $2, 'Legacy project', 'LEG')", &[&project, &ws]).await.unwrap();
    for u in [u1, u2] {
        raw.execute("INSERT INTO project_members (project_id, workspace_id, user_id, role) VALUES ($1, $2, $3, 'member')", &[&project, &ws, &u]).await.unwrap();
    }
    raw.execute("INSERT INTO project_user_properties (project_id, user_id, sort_order) VALUES ($1, $2, 7.5)", &[&project, &u1]).await.unwrap();

    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    async fn state(raw: &tokio_postgres::Client) -> (Vec<(Uuid, Uuid)>, Vec<(Uuid, Uuid, f64, serde_json::Value)>) {
        let members = raw.query("SELECT user_id, id FROM project_members ORDER BY user_id", &[]).await.unwrap();
        let props = raw.query("SELECT user_id, workspace_id, sort_order, filters FROM project_user_properties ORDER BY user_id", &[]).await.unwrap();
        (
            members.iter().map(|r| (r.get(0), r.get(1))).collect(),
            props.iter().map(|r| (r.get(0), r.get(1), r.get(2), r.get(3))).collect(),
        )
    }
    let first = state(&raw).await;
    thrust_adapters_postgres::migrate(&raw).await.unwrap();
    assert_eq!(first, state(&raw).await, "a second run changed the members or their settings");
    assert_ne!(first.0[0].1, first.0[1].1, "two members share an id");
    assert_eq!(first.1, vec![(u1, ws, 7.5, serde_json::json!({}))], "a settings row has the workspace of its project and the empty object");

    drop(raw);
    let db = PgDatabase::new(conn);
    let scope = WorkspaceScope::mint(WorkspaceId(ws), UserId(u1), Role::Member);
    let d = db.project_detail(&scope, ProjectId(project)).await.unwrap().unwrap();
    assert_eq!((d.listing.role, d.listing.sort_order, d.members.len(), d.next_sequence), (Some(Role::Member), Some(7.5), 2, 1));
    assert_eq!((d.listing.project.description.as_str(), d.listing.project.timezone.as_str(), d.listing.project.archive_in), ("", "UTC", 0));
    // a member who has no settings row yet gets one on their first ask, and the one that was there is kept
    let m = db.project_member(&scope, ProjectId(project)).await.unwrap().unwrap();
    assert_eq!((m.member.role, m.member.sort_order, m.member.view_props.clone()), (Role::Member, 65535.0, serde_json::json!({})));
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
    for (project, draft, archived, deleted) in [(p1, false, false, false), (p1, true, false, false), (p1, false, false, false), (p1, false, false, true), (p2, false, true, false)] {
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
    let rows = env.db.list_work_items(&s, Visibility::WithDeleted).await.unwrap();
    assert_eq!(rows.len(), 1);
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
        let rows = env.db.list_work_items(&s, Visibility::WithDeleted).await.unwrap();
        assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["kept"], "abandoned transactions must roll back");
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
