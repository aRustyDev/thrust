//! The migration ledger (issue #195) against a real Postgres: a database without one gets every file once and records them, a second
//! run applies nothing, a newer file runs once in a transaction with its row, and what the ledger refuses (an edited file, a build from
//! another branch, a file older than the newest applied, a contract file when they are not allowed) applies nothing. The arithmetic
//! alone, over generated inputs, is `crates/adapters/postgres/tests/ledger.rs`.
//!
//! The extra files these tests carry are made up for them (`9000_ledger_probe.sql` and so on, numbered after every real file, however many there are: the ledger orders by name, and one that sorted before a real file would be out of order): each leaves a mark that a test can
//! count, so that "applied once" is a count of one and not the absence of an error.

use thrust_adapters_postgres::ledger::{Plan, LEDGER_TABLE};
use thrust_adapters_postgres::{
    carried, migrate, migrate_atomic_with, migrate_guarded_with, migrate_preview_with, migrate_with, plan_migrations,
    MigrateError, MigrateOptions, MigrateOutcome, Migration, MIGRATIONS,
};

use super::*;

const ALLOW_CONTRACT: MigrateOptions = MigrateOptions { allow_contract: true };
const NO_CONTRACT: MigrateOptions = MigrateOptions { allow_contract: false };

/// A Postgres with nothing applied.
struct Fresh {
    conn: Arc<PoolConnector>,
    _container: ContainerAsync<Postgres>,
}

async fn fresh() -> Fresh {
    let container = Postgres::default().start().await.expect("start postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    Fresh { conn: Arc::new(PoolConnector::from_url(&url, 4).unwrap()), _container: container }
}

/// The adapter's files and these made-up ones after them.
fn with(extra: &[(&'static str, &'static str)]) -> Vec<Migration<'static>> {
    MIGRATIONS.iter().copied().chain(extra.iter().map(|&(file, sql)| Migration { file, sql })).collect()
}

/// A file that inserts a row each time it runs, into a table it makes once: run once, the table has one row.
const PROBE: (&str, &str) =
    ("9000_ledger_probe.sql", "CREATE TABLE IF NOT EXISTS ledger_probe (n int); INSERT INTO ledger_probe VALUES (19);");

/// The ledger's rows, by file: (file, sha256, applied_at as text).
async fn ledger_rows(raw: &tokio_postgres::Client) -> Vec<(String, String, String)> {
    raw.query(&format!("SELECT file, sha256, applied_at::text FROM {LEDGER_TABLE} ORDER BY file"), &[])
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect()
}

async fn ledger_files(raw: &tokio_postgres::Client) -> Vec<String> {
    ledger_rows(raw).await.into_iter().map(|(f, _, _)| f).collect()
}

async fn table_exists(raw: &tokio_postgres::Client, name: &str) -> bool {
    raw.query_one("SELECT to_regclass($1) IS NOT NULL", &[&name]).await.unwrap().get(0)
}

async fn probe_rows(raw: &tokio_postgres::Client) -> i64 {
    raw.query_one("SELECT count(*) FROM ledger_probe", &[]).await.unwrap().get(0)
}

fn names(files: &[Migration<'_>]) -> Vec<String> {
    files.iter().map(|m| m.file.to_string()).collect()
}

/// PostgreSQL must reject transaction control from untrusted migration data while permitting the
/// real baseline's multi-statement SQL and nested DO blocks.
#[tokio::test]
async fn model_guarded_migrations_refuse_transaction_escapes() {
    let db = fresh().await;
    let raw = db.conn.raw().await.unwrap();
    assert_eq!(
        migrate_guarded_with(&raw, &MIGRATIONS, NO_CONTRACT).await.unwrap(),
        MigrateOutcome::Baseline { applied: MIGRATIONS.len() }
    );
    let attacks = [
        "COMMIT;",
        "ROLLBACK;",
        "END;",
        "SAVEPOINT hostile;",
        "DO $$ BEGIN COMMIT; END $$;",
        "DO $$ BEGIN ROLLBACK; END $$;",
        "CREATE PROCEDURE hostile() LANGUAGE plpgsql AS $$ BEGIN COMMIT; END $$; CALL hostile();",
    ];
    for attack in attacks {
        let sql = format!(
            "CREATE TABLE migration_escape_probe (n int); {attack} CREATE TABLE migration_escape_after (n int);"
        );
        let mut files: Vec<Migration<'_>> = MIGRATIONS.to_vec();
        files.push(Migration { file: "9000_escape_probe.sql", sql: &sql });
        let result = migrate_guarded_with(&raw, &files, NO_CONTRACT).await;
        assert!(matches!(result, Err(MigrateError::Db(_))), "escape accepted: {attack}: {result:?}");
        assert!(!table_exists(&raw, "migration_escape_probe").await, "escape committed: {attack}");
        assert!(!table_exists(&raw, "migration_escape_after").await, "escape continued: {attack}");
        assert_eq!(ledger_files(&raw).await, names(&MIGRATIONS));
    }
    let sql = "SELECT '$thrust_migration_guard$; COMMIT;'; CREATE TABLE guard_literal (n int);";
    let mut files = MIGRATIONS.to_vec();
    files.push(Migration { file: "9000_guard_literal.sql", sql });
    assert!(matches!(migrate_guarded_with(&raw, &files, NO_CONTRACT).await, Ok(MigrateOutcome::Applied { .. })));
    assert!(table_exists(&raw, "guard_literal").await);
}

/// Each restricted PR role can migrate its own schema and cannot create a table in public.
#[tokio::test]
async fn model_guarded_migrations_run_as_the_preview_role() {
    let db = fresh().await;
    let raw = db.conn.raw().await.unwrap();
    raw.batch_execute(
        "CREATE EXTENSION IF NOT EXISTS pg_trgm WITH SCHEMA public; REVOKE CREATE ON SCHEMA public FROM PUBLIC;",
    )
    .await
    .unwrap();
    for number in 1..=4 {
        let role = format!("pr_{number}");
        raw.batch_execute(&format!("CREATE ROLE {role} NOLOGIN; CREATE SCHEMA {role} AUTHORIZATION {role}; SET ROLE {role}; SET search_path = {role};"))
            .await
            .unwrap();
        let current: String = raw.query_one("SELECT current_user", &[]).await.unwrap().get(0);
        assert_eq!(current, role);
        if number == 4 {
            // Pins the collision case: the scoped catalog view must never replace an existing object.
            raw.batch_execute("CREATE VIEW pg_constraint AS SELECT 42 AS marker").await.unwrap();
            assert!(migrate_guarded_with(&raw, &MIGRATIONS, NO_CONTRACT).await.is_err());
            let marker: i32 =
                raw.query_one(&format!("SELECT marker FROM {role}.pg_constraint"), &[]).await.unwrap().get(0);
            assert_eq!(marker, 42);
            assert!(!table_exists(&raw, "thrust_schema_migrations").await);
            raw.batch_execute(&format!("DROP VIEW {role}.pg_constraint")).await.unwrap();
        }
        assert_eq!(
            migrate_guarded_with(&raw, &MIGRATIONS, NO_CONTRACT).await.unwrap(),
            MigrateOutcome::Baseline { applied: MIGRATIONS.len() }
        );
        let view: Option<String> = raw
            .query_one("SELECT pg_catalog.to_regclass($1)::text", &[&format!("{role}.pg_constraint")])
            .await
            .unwrap()
            .get(0);
        assert!(view.is_none(), "temporary catalog view survived migration");
        let entries = thrust_adapters_postgres::carried(&MIGRATIONS);
        for entry in entries {
            let checksum: String = raw
                .query_one("SELECT sha256 FROM thrust_schema_migrations WHERE file = $1", &[&entry.file])
                .await
                .unwrap()
                .get(0);
            assert_eq!(checksum, entry.sha256);
        }
        let bad = format!("CREATE TABLE public.preview_forbidden_{number} (n int);");
        let mut files = MIGRATIONS.to_vec();
        files.push(Migration { file: "9000_public_forbidden.sql", sql: &bad });
        let failure = migrate_guarded_with(&raw, &files, NO_CONTRACT).await.unwrap_err();
        assert!(
            matches!(&failure, MigrateError::Db(error) if error.code() == Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)),
            "{failure:?}"
        );
        assert!(!table_exists(&raw, &format!("public.preview_forbidden_{number}")).await);
        assert_eq!(ledger_files(&raw).await, names(&MIGRATIONS));
        raw.batch_execute("RESET ROLE; RESET search_path;").await.unwrap();
    }
}

/// Each preview identity is checked inside the migration transaction before any file runs.
#[tokio::test]
async fn model_preview_migration_rejects_wrong_connection_and_privileges() {
    use thrust_adapters_postgres::preview_artifact::Artifact;
    let db = fresh().await;
    let admin = db.conn.raw().await.unwrap();
    admin
        .batch_execute("REVOKE CREATE ON SCHEMA public FROM PUBLIC; CREATE ROLE preview_extra NOLOGIN;")
        .await
        .unwrap();
    let port = db._container.get_host_port_ipv4(5432).await.unwrap();
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let sql = "CREATE TABLE identity_probe (n int);";
    let data = serde_json::to_vec(&serde_json::json!({"version":1,"commit":commit,"files":[{
        "name":"0001_identity.sql","sql":sql,"sha256":thrust_adapters_postgres::ledger::sha256_hex(sql)
    }]}))
    .unwrap();
    let artifact = Artifact::parse(&data, commit).unwrap();
    for number in 1..=3 {
        let role = format!("pr_{number}");
        admin.batch_execute(&format!("CREATE ROLE {role} LOGIN PASSWORD 'testpass'; CREATE SCHEMA {role} AUTHORIZATION {role}; ALTER ROLE {role} IN DATABASE postgres SET search_path = {role};")).await.unwrap();
        let conn =
            PoolConnector::from_url(&format!("postgres://{role}:testpass@127.0.0.1:{port}/postgres"), 1).unwrap();
        let client = conn.raw().await.unwrap();
        for (requested, database) in [(number + 1, "postgres"), (number, "other"), (0, "postgres")] {
            assert!(matches!(
                migrate_preview_with(&client, requested, database, &artifact).await,
                Err(MigrateError::PreviewIdentityRefused)
            ));
            assert!(!table_exists(&client, "thrust_schema_migrations").await);
        }
        for (enable, undo) in [
            (format!("GRANT CREATE ON SCHEMA public TO {role}"), format!("REVOKE CREATE ON SCHEMA public FROM {role}")),
            (
                format!("GRANT CREATE ON DATABASE postgres TO {role}"),
                format!("REVOKE CREATE ON DATABASE postgres FROM {role}"),
            ),
            (format!("GRANT preview_extra TO {role}"), format!("REVOKE preview_extra FROM {role}")),
            (format!("ALTER SCHEMA {role} OWNER TO postgres"), format!("ALTER SCHEMA {role} OWNER TO {role}")),
        ] {
            admin.batch_execute(&enable).await.unwrap();
            assert!(
                matches!(
                    migrate_preview_with(&client, number, "postgres", &artifact).await,
                    Err(MigrateError::PreviewIdentityRefused)
                ),
                "{enable}"
            );
            admin.batch_execute(&undo).await.unwrap();
            assert!(!table_exists(&client, "thrust_schema_migrations").await);
        }
        for path in ["public".to_owned(), format!("{role}, public")] {
            client.batch_execute(&format!("SET search_path = {path}")).await.unwrap();
            assert!(matches!(
                migrate_preview_with(&client, number, "postgres", &artifact).await,
                Err(MigrateError::PreviewIdentityRefused)
            ));
        }
        client.batch_execute(&format!("SET search_path = {role}")).await.unwrap();
        admin.batch_execute(&format!("SET ROLE {role}; SET search_path = {role}")).await.unwrap();
        assert!(matches!(
            migrate_preview_with(&admin, number, "postgres", &artifact).await,
            Err(MigrateError::PreviewIdentityRefused)
        ));
        admin.batch_execute("RESET ROLE; RESET search_path").await.unwrap();
        // Control: without the preview identity gate the same connection can migrate even
        // when a caller claims the wrong PR; its SQL and ledger must actually appear.
        assert!(matches!(
            migrate_guarded_with(&client, &artifact.migrations(), NO_CONTRACT).await,
            Ok(MigrateOutcome::Baseline { .. })
        ));
        assert!(table_exists(&client, "identity_probe").await);
        assert!(table_exists(&client, "thrust_schema_migrations").await);
        client.batch_execute("DROP TABLE identity_probe, thrust_schema_migrations").await.unwrap();
        assert_eq!(
            migrate_preview_with(&client, number, "postgres", &artifact).await.unwrap(),
            MigrateOutcome::Baseline { applied: 1 }
        );
        assert!(table_exists(&client, "identity_probe").await);
        assert_eq!(
            migrate_preview_with(&client, number, "postgres", &artifact).await.unwrap(),
            MigrateOutcome::Current
        );
        let contract_sql = "DROP TABLE identity_probe";
        let contract_data = serde_json::to_vec(&serde_json::json!({"version":1,"commit":commit,"files":[
            {"name":"0001_identity.sql","sql":sql,"sha256":thrust_adapters_postgres::ledger::sha256_hex(sql)},
            {"name":"0002_contract_drop.sql","sql":contract_sql,"sha256":thrust_adapters_postgres::ledger::sha256_hex(contract_sql)}
        ]})).unwrap();
        let contract_artifact = Artifact::parse(&contract_data, commit).unwrap();
        assert!(matches!(
            migrate_preview_with(&client, number, "postgres", &contract_artifact).await,
            Err(MigrateError::ContractRefused(_))
        ));
        assert!(table_exists(&client, "identity_probe").await);
    }
}

/// Every position of a failing file rolls the complete transaction back, and a good retry applies once.
#[tokio::test]
async fn model_atomic_migrations_roll_back_at_each_failure_position() {
    let db = fresh().await;
    let raw = db.conn.raw().await.unwrap();
    migrate(&raw).await.unwrap();
    let file_names: Vec<String> = (0..4).map(|n| format!("900{n}_atomic.sql")).collect();
    for failing in 0..4 {
        let sources: Vec<String> =
            (0..4)
                .map(|n| {
                    if n == failing {
                        "SELECT 1 / 0;".to_string()
                    } else {
                        format!("CREATE TABLE atomic_{n} (n int);")
                    }
                })
                .collect();
        let mut files: Vec<Migration<'_>> = MIGRATIONS.to_vec();
        files.extend(file_names.iter().zip(&sources).map(|(file, sql)| Migration { file, sql }));
        assert!(matches!(migrate_atomic_with(&raw, &files, NO_CONTRACT).await, Err(MigrateError::Db(_))));
        for n in 0..4 {
            assert!(!table_exists(&raw, &format!("atomic_{n}")).await);
        }
        assert_eq!(ledger_files(&raw).await, names(&MIGRATIONS));
    }
    let sources: Vec<String> = (0..4)
        .map(|n| {
            format!("CREATE TABLE atomic_{n} (n int); DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_locks WHERE locktype = 'advisory' AND pid = pg_backend_pid() AND granted) THEN RAISE EXCEPTION 'migration lock missing'; END IF; END $$;")
        })
        .collect();
    let mut files: Vec<Migration<'_>> = MIGRATIONS.to_vec();
    files.extend(file_names.iter().zip(&sources).map(|(file, sql)| Migration { file, sql }));
    assert_eq!(
        migrate_atomic_with(&raw, &files, NO_CONTRACT).await.unwrap(),
        MigrateOutcome::Applied { files: file_names.clone() }
    );
    assert_eq!(migrate_atomic_with(&raw, &files, NO_CONTRACT).await.unwrap(), MigrateOutcome::Current);
    let held: i64 = raw.query_one("SELECT count(*) FROM pg_catalog.pg_locks WHERE locktype = 'advisory' AND pid = pg_backend_pid() AND granted", &[]).await.unwrap().get(0);
    assert_eq!(held, 0, "transaction lock must be released at commit");
    files.push(Migration { file: "9004_contract_forbidden.sql", sql: "CREATE TABLE atomic_contract (n int);" });
    assert!(matches!(migrate_atomic_with(&raw, &files, NO_CONTRACT).await, Err(MigrateError::ContractRefused(_))));
    assert!(!table_exists(&raw, "atomic_contract").await);
}

/// Two callers on separate connections serialize on the transaction lock and apply the baseline once.
#[tokio::test]
async fn model_atomic_migrations_serialize_concurrent_baselines() {
    let db = fresh().await;
    let first = db.conn.raw().await.unwrap();
    let second = db.conn.raw().await.unwrap();
    let (a, b) = tokio::join!(
        migrate_atomic_with(&first, &MIGRATIONS, NO_CONTRACT),
        migrate_atomic_with(&second, &MIGRATIONS, NO_CONTRACT)
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert!(matches!(
        (&a, &b),
        (MigrateOutcome::Baseline { .. }, MigrateOutcome::Current)
            | (MigrateOutcome::Current, MigrateOutcome::Baseline { .. })
    ));
    assert_eq!(ledger_files(&first).await, names(&MIGRATIONS));
}

/// A new database: `--plan` says baseline and writes nothing, not even the ledger table; `migrate` runs every file and records each
/// with its checksum; a second `migrate` applies nothing and leaves every row as it was.
#[tokio::test]
async fn a_new_database_gets_the_baseline_and_a_second_run_applies_nothing() {
    let db = fresh().await;
    let raw = db.conn.raw().await.unwrap();

    let (present, planned) = plan_migrations(&raw, &MIGRATIONS).await.unwrap();
    assert!(!present);
    assert_eq!(planned, Plan::Apply { pending: names(&MIGRATIONS), contract: vec![] });
    assert_eq!(planned.to_json(present)["outcome"], "baseline");
    assert!(!table_exists(&raw, LEDGER_TABLE).await, "--plan wrote the ledger table");

    assert_eq!(migrate(&raw).await.unwrap(), MigrateOutcome::Baseline { applied: MIGRATIONS.len() });
    let rows = ledger_rows(&raw).await;
    let recorded: Vec<(String, String)> = rows.iter().map(|(f, s, _)| (f.clone(), s.clone())).collect();
    let want: Vec<(String, String)> = carried(&MIGRATIONS).into_iter().map(|e| (e.file, e.sha256)).collect();
    assert_eq!(recorded, want, "every file is recorded with the checksum of its SQL");

    assert_eq!(migrate(&raw).await.unwrap(), MigrateOutcome::Current);
    assert_eq!(ledger_rows(&raw).await, rows, "a second run touched the ledger");
    let (present, planned) = plan_migrations(&raw, &MIGRATIONS).await.unwrap();
    assert!(present);
    assert_eq!(planned, Plan::Apply { pending: vec![], contract: vec![] });
}

/// The baseline is one transaction: when a file of it fails, the database is left as it was, with no ledger and none of the tables the
/// files before it made, so the next run is the baseline again and not a half-recorded schema.
#[tokio::test]
async fn a_baseline_that_fails_leaves_the_database_as_it_was() {
    let db = fresh().await;
    let raw = db.conn.raw().await.unwrap();
    let got = migrate_with(&raw, &with(&[("9000_ledger_fails.sql", "SELECT 1 / 0;")]), ALLOW_CONTRACT).await;
    assert!(matches!(got, Err(MigrateError::Db(_))), "{got:?}");
    assert!(!table_exists(&raw, LEDGER_TABLE).await, "a failed baseline left a ledger");
    assert!(
        !table_exists(&raw, "work_items").await,
        "a failed baseline left the tables of the files before the one that failed"
    );
    // the control: the same database takes the baseline once the failing file is gone
    assert_eq!(migrate(&raw).await.unwrap(), MigrateOutcome::Baseline { applied: MIGRATIONS.len() });
    assert!(table_exists(&raw, "work_items").await);
}

/// A database from before the ledger (every file applied by hand, as `migrate` did until #195), with a row in it: the baseline
/// records it without losing the row, and the next run applies nothing.
#[tokio::test]
async fn a_database_from_before_the_ledger_is_recorded_and_keeps_its_rows() {
    let db = fresh().await;
    let raw = db.conn.raw().await.unwrap();
    for m in &MIGRATIONS {
        raw.batch_execute(m.sql).await.unwrap();
    }
    let user = UserId(new_id());
    raw.execute("INSERT INTO users (id, email) VALUES ($1, 'before-the-ledger@example.test')", &[&user.0])
        .await
        .unwrap();
    assert!(!table_exists(&raw, LEDGER_TABLE).await);

    assert_eq!(migrate(&raw).await.unwrap(), MigrateOutcome::Baseline { applied: MIGRATIONS.len() });
    assert_eq!(ledger_files(&raw).await, names(&MIGRATIONS));
    let kept: i64 = raw.query_one("SELECT count(*) FROM users WHERE id = $1", &[&user.0]).await.unwrap().get(0);
    assert_eq!(kept, 1);
    assert_eq!(migrate(&raw).await.unwrap(), MigrateOutcome::Current);
}

/// A newer file runs once: its mark is one row, and a second run leaves one row (a file run again would insert another). A file that
/// fails leaves neither its table nor its ledger row, and the file applied before it in the same run stays applied and recorded.
#[tokio::test]
async fn a_newer_file_runs_once_in_a_transaction_with_its_row() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let files = with(&[PROBE]);
    assert_eq!(
        migrate_with(&raw, &files, ALLOW_CONTRACT).await.unwrap(),
        MigrateOutcome::Applied { files: vec![PROBE.0.to_string()] }
    );
    assert_eq!(probe_rows(&raw).await, 1);
    assert_eq!(migrate_with(&raw, &files, ALLOW_CONTRACT).await.unwrap(), MigrateOutcome::Current);
    assert_eq!(probe_rows(&raw).await, 1, "the file ran again");

    // two more files in one run: the first applies, the second fails half way
    let files = with(&[
        PROBE,
        ("9001_ledger_second.sql", "CREATE TABLE ledger_second (n int);"),
        ("9002_ledger_fails.sql", "CREATE TABLE ledger_half_done (n int); SELECT 1 / 0;"),
    ]);
    let got = migrate_with(&raw, &files, ALLOW_CONTRACT).await;
    assert!(matches!(got, Err(MigrateError::Db(_))), "{got:?}");
    assert!(table_exists(&raw, "ledger_second").await);
    assert!(!table_exists(&raw, "ledger_half_done").await, "the failed file's table was kept");
    let recorded = ledger_files(&raw).await;
    assert!(recorded.contains(&"9001_ledger_second.sql".to_string()));
    assert!(!recorded.contains(&"9002_ledger_fails.sql".to_string()), "the failed file was recorded");
}

/// A file and its ledger row are one transaction: when the row cannot be written, the file's changes are undone too. The fault is
/// made by the file itself, which writes the row that `migrate` then fails to write (a unique key), so the only way the file's table
/// can survive is a file that was applied outside the transaction of its row.
#[tokio::test]
async fn a_file_whose_row_cannot_be_written_leaves_nothing() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let file = "9000_ledger_row_taken.sql";
    let sql = "CREATE TABLE ledger_row_taken (n int); \
               INSERT INTO thrust_schema_migrations (file, sha256) VALUES ('9000_ledger_row_taken.sql', 'not this file');";
    let got = migrate_with(&raw, &with(&[(file, sql)]), ALLOW_CONTRACT).await;
    assert!(matches!(got, Err(MigrateError::Db(_))), "{got:?}");
    assert!(!table_exists(&raw, "ledger_row_taken").await, "the file's change outlived its row");
    assert!(!ledger_files(&raw).await.contains(&file.to_string()));
}

/// The control the ledger exists for: a file that was edited after it ran is refused, by its name, and nothing runs, not even a file
/// that was pending. The same refusal when it is the ledger's checksum that is not the file's.
#[tokio::test]
async fn an_edited_file_is_refused_and_nothing_runs() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let before = ledger_rows(&raw).await;
    let last = MIGRATIONS[MIGRATIONS.len() - 1];
    let edited_sql: &'static str = Box::leak(format!("{}\n-- edited after it ran\n", last.sql).into_boxed_str());
    let mut files: Vec<Migration<'_>> = MIGRATIONS.to_vec();
    *files.last_mut().unwrap() = Migration { file: last.file, sql: edited_sql };
    files.push(Migration { file: PROBE.0, sql: PROBE.1 });

    let got = migrate_with(&raw, &files, ALLOW_CONTRACT).await;
    match &got {
        Err(MigrateError::Refused(Plan::Changed { files })) => assert_eq!(files, &vec![last.file.to_string()]),
        other => panic!("expected the edited file to be refused, got {other:?}"),
    }
    assert!(got.unwrap_err().to_string().contains(last.file), "the refusal names the file");
    assert!(!table_exists(&raw, "ledger_probe").await, "a pending file ran after a refusal");
    assert_eq!(ledger_rows(&raw).await, before);

    // the database's side: the ledger says another checksum than the file's
    raw.execute(&format!("UPDATE {LEDGER_TABLE} SET sha256 = repeat('0', 64) WHERE file = $1"), &[&MIGRATIONS[4].file])
        .await
        .unwrap();
    let got = migrate(&raw).await;
    assert!(
        matches!(&got, Err(MigrateError::Refused(Plan::Changed { files })) if files == &vec![MIGRATIONS[4].file.to_string()]),
        "{got:?}"
    );
}

/// An older build next to a newer schema: the database has a file it does not know, and every file it knows. It applies nothing and
/// succeeds (an older release runs on the newer, expanded schema).
#[tokio::test]
async fn an_older_build_applies_nothing_and_succeeds() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    migrate_with(&raw, &with(&[PROBE]), ALLOW_CONTRACT).await.unwrap();
    let before = ledger_rows(&raw).await;
    assert_eq!(migrate(&raw).await.unwrap(), MigrateOutcome::Older { unknown: vec![PROBE.0.to_string()] });
    assert_eq!(ledger_rows(&raw).await, before);
    assert_eq!(probe_rows(&raw).await, 1);
}

/// Two builds that each have a file the other lacks: refused, and the second build's file does not run.
#[tokio::test]
async fn a_build_from_another_branch_is_refused() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    migrate_with(&raw, &with(&[PROBE]), ALLOW_CONTRACT).await.unwrap();
    let other = with(&[("9000_other_branch.sql", "CREATE TABLE ledger_other_branch (n int);")]);
    let got = migrate_with(&raw, &other, ALLOW_CONTRACT).await;
    assert!(matches!(&got, Err(MigrateError::Refused(Plan::Diverged { .. }))), "{got:?}");
    assert!(!table_exists(&raw, "ledger_other_branch").await);
}

/// A file numbered before the newest applied one: refused, and it does not run.
#[tokio::test]
async fn a_file_older_than_the_newest_applied_is_refused() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let newer = ("9001_ledger_newer.sql", "CREATE TABLE ledger_newer (n int);");
    migrate_with(&raw, &with(&[newer]), ALLOW_CONTRACT).await.unwrap();
    let got = migrate_with(&raw, &with(&[PROBE, newer]), ALLOW_CONTRACT).await;
    assert!(matches!(&got, Err(MigrateError::Refused(Plan::OutOfOrder { .. }))), "{got:?}");
    assert!(!table_exists(&raw, "ledger_probe").await);
}

/// A contract file to apply: refused when contract migrations are not allowed (nothing runs, nothing is recorded), applied when they
/// are. On a database without the ledger the refusal comes before the baseline, so not even the ledger table is made.
#[tokio::test]
async fn a_contract_file_is_refused_on_request_and_applied_otherwise() {
    let contract = ("9000_contract_ledger_probe.sql", "CREATE TABLE ledger_contract_ran (n int);");

    let db = fresh().await;
    let raw = db.conn.raw().await.unwrap();
    let got = migrate_with(&raw, &with(&[contract]), NO_CONTRACT).await;
    assert!(matches!(&got, Err(MigrateError::ContractRefused(f)) if f == &vec![contract.0.to_string()]), "{got:?}");
    assert!(!table_exists(&raw, LEDGER_TABLE).await, "the baseline ran before the refusal");

    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let got = migrate_with(&raw, &with(&[contract]), NO_CONTRACT).await;
    assert!(matches!(&got, Err(MigrateError::ContractRefused(_))), "{got:?}");
    assert!(!table_exists(&raw, "ledger_contract_ran").await);
    assert!(!ledger_files(&raw).await.contains(&contract.0.to_string()));
    // and an expand file is not refused by the same option
    assert!(migrate_with(&raw, &with(&[PROBE]), NO_CONTRACT).await.is_ok());

    let got = migrate_with(&raw, &with(&[PROBE, ("9001_contract_ledger_probe.sql", contract.1)]), ALLOW_CONTRACT).await;
    assert_eq!(got.unwrap(), MigrateOutcome::Applied { files: vec!["9001_contract_ledger_probe.sql".to_string()] });
    assert!(table_exists(&raw, "ledger_contract_ran").await);
}

/// `reset` empties the data and not the ledger: after it, the next `migrate` applies nothing. (Emptied, the ledger would make the
/// next run apply every file again.) The control: the reset did empty a table that had a row.
#[tokio::test]
async fn a_reset_keeps_the_ledger_and_the_next_migrate_applies_nothing() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let before = ledger_rows(&raw).await;
    raw.execute("INSERT INTO users (id, email) VALUES ($1, 'reset@example.test')", &[&new_id()]).await.unwrap();
    env.db.reset(&SystemScope::mint()).await.unwrap();
    let users: i64 = raw.query_one("SELECT count(*) FROM users", &[]).await.unwrap().get(0);
    assert_eq!(users, 0, "the control: reset empties the data");
    assert_eq!(ledger_rows(&raw).await, before);
    assert_eq!(migrate(&raw).await.unwrap(), MigrateOutcome::Current);
}

/// Two runs at once on a new database (two deploys): they take turns on the advisory lock, so one makes the baseline and the other
/// finds it done, and each file is recorded once.
#[tokio::test]
async fn two_migrations_at_once_take_turns() {
    let db = fresh().await;
    let (a, b) = (db.conn.raw().await.unwrap(), db.conn.raw().await.unwrap());
    let (x, y) = tokio::join!(migrate(&a), migrate(&b));
    let mut outcomes = vec![format!("{:?}", x.unwrap()), format!("{:?}", y.unwrap())];
    outcomes.sort();
    assert_eq!(
        outcomes,
        vec![format!("{:?}", MigrateOutcome::Baseline { applied: MIGRATIONS.len() }), "Current".to_string()]
    );
    assert_eq!(ledger_files(&a).await, names(&MIGRATIONS));
}
