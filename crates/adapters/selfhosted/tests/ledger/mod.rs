//! The migration ledger (issue #195) against a real Postgres: a database without one gets every file once and records them, a second
//! run applies nothing, a newer file runs once in a transaction with its row, and what the ledger refuses (an edited file, a build from
//! another branch, a file older than the newest applied, a contract file when they are not allowed) applies nothing. The arithmetic
//! alone, over generated inputs, is `crates/adapters/postgres/tests/ledger.rs`.
//!
//! The extra files these tests carry are made up for them (`0019_ledger_probe.sql` and so on): each leaves a mark that a test can
//! count, so that "applied once" is a count of one and not the absence of an error.

use thrust_adapters_postgres::ledger::{Plan, LEDGER_TABLE};
use thrust_adapters_postgres::{
    carried, migrate, migrate_with, plan_migrations, MigrateError, MigrateOptions, MigrateOutcome, Migration,
    MIGRATIONS,
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
fn with(extra: &[(&'static str, &'static str)]) -> Vec<Migration> {
    MIGRATIONS.iter().copied().chain(extra.iter().map(|&(file, sql)| Migration { file, sql })).collect()
}

/// A file that inserts a row each time it runs, into a table it makes once: run once, the table has one row.
const PROBE: (&str, &str) =
    ("0019_ledger_probe.sql", "CREATE TABLE IF NOT EXISTS ledger_probe (n int); INSERT INTO ledger_probe VALUES (19);");

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

fn names(files: &[Migration]) -> Vec<String> {
    files.iter().map(|m| m.file.to_string()).collect()
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
    let got = migrate_with(&raw, &with(&[("0019_ledger_fails.sql", "SELECT 1 / 0;")]), ALLOW_CONTRACT).await;
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
        ("0020_ledger_second.sql", "CREATE TABLE ledger_second (n int);"),
        ("0021_ledger_fails.sql", "CREATE TABLE ledger_half_done (n int); SELECT 1 / 0;"),
    ]);
    let got = migrate_with(&raw, &files, ALLOW_CONTRACT).await;
    assert!(matches!(got, Err(MigrateError::Db(_))), "{got:?}");
    assert!(table_exists(&raw, "ledger_second").await);
    assert!(!table_exists(&raw, "ledger_half_done").await, "the failed file's table was kept");
    let recorded = ledger_files(&raw).await;
    assert!(recorded.contains(&"0020_ledger_second.sql".to_string()));
    assert!(!recorded.contains(&"0021_ledger_fails.sql".to_string()), "the failed file was recorded");
}

/// A file and its ledger row are one transaction: when the row cannot be written, the file's changes are undone too. The fault is
/// made by the file itself, which writes the row that `migrate` then fails to write (a unique key), so the only way the file's table
/// can survive is a file that was applied outside the transaction of its row.
#[tokio::test]
async fn a_file_whose_row_cannot_be_written_leaves_nothing() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let file = "0019_ledger_row_taken.sql";
    let sql = "CREATE TABLE ledger_row_taken (n int); \
               INSERT INTO thrust_schema_migrations (file, sha256) VALUES ('0019_ledger_row_taken.sql', 'not this file');";
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
    let mut files: Vec<Migration> = MIGRATIONS.to_vec();
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
    let other = with(&[("0019_other_branch.sql", "CREATE TABLE ledger_other_branch (n int);")]);
    let got = migrate_with(&raw, &other, ALLOW_CONTRACT).await;
    assert!(matches!(&got, Err(MigrateError::Refused(Plan::Diverged { .. }))), "{got:?}");
    assert!(!table_exists(&raw, "ledger_other_branch").await);
}

/// A file numbered before the newest applied one: refused, and it does not run.
#[tokio::test]
async fn a_file_older_than_the_newest_applied_is_refused() {
    let env = env().await;
    let raw = env.conn.raw().await.unwrap();
    let newer = ("0020_ledger_newer.sql", "CREATE TABLE ledger_newer (n int);");
    migrate_with(&raw, &with(&[newer]), ALLOW_CONTRACT).await.unwrap();
    let got = migrate_with(&raw, &with(&[PROBE, newer]), ALLOW_CONTRACT).await;
    assert!(matches!(&got, Err(MigrateError::Refused(Plan::OutOfOrder { .. }))), "{got:?}");
    assert!(!table_exists(&raw, "ledger_probe").await);
}

/// A contract file to apply: refused when contract migrations are not allowed (nothing runs, nothing is recorded), applied when they
/// are. On a database without the ledger the refusal comes before the baseline, so not even the ledger table is made.
#[tokio::test]
async fn a_contract_file_is_refused_on_request_and_applied_otherwise() {
    let contract = ("0019_contract_ledger_probe.sql", "CREATE TABLE ledger_contract_ran (n int);");

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

    let got = migrate_with(&raw, &with(&[PROBE, ("0020_contract_ledger_probe.sql", contract.1)]), ALLOW_CONTRACT).await;
    assert_eq!(got.unwrap(), MigrateOutcome::Applied { files: vec!["0020_contract_ledger_probe.sql".to_string()] });
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
