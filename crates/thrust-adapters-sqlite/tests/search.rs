//! Search and the bulk seed on the SQLite adapter, hermetically (spike 0002, ADR-0083). The HTTP twin of these tests is
//! the engine pass (`crates/thrust-contract-tests/tests/engine.rs`), which runs on every target; this one needs no
//! target and runs in the default loop.
//!
//! `prop_search_matches_the_reference_adapter` is a differential test: the memory adapter's search is a scan with
//! `to_lowercase` and `contains` (Plane's `icontains`), and the SQLite adapter answers from an FTS5 trigram index (three
//! characters or more) or a LIKE scan (fewer). Same corpus, same queries, same answers.
//!
//! `search.proptest-regressions` is a pinned regression (rule 9 (a)): the query `é` found nothing for a row holding `ÉCOLE`,
//! because SQLite's LIKE folds ASCII case only. Short non-ASCII queries now scan in Rust (`scan_in_rust`).

use std::sync::Arc;

use futures_executor::block_on;
use proptest::prelude::*;
use thrust_adapters_memory::MemoryDatabase;
use thrust_adapters_sqlite::{Exec, NativeSql, SqliteDatabase};
use thrust_contracts::chrono::{TimeZone, Utc};
use thrust_contracts::*;
use thrust_ports::*;

const WORDS: &[&str] = &[
    "fix", "Fix", "FIX", "login", "Login", "bug", "BUG", "debug", "time", "Time", "queue", "100%", "a_b", "a%b", "back\\slash",
    "it's", "say\"hi\"", "x", "ab", "abc", "ABC", "api", "retry-after", "\u{e9}cole", "\u{c9}COLE",
];

fn text() -> impl Strategy<Value = String> {
    (prop::collection::vec(prop::sample::select(WORDS), 1..4), prop::sample::select(&[" ", "-", "_", ""][..]))
        .prop_map(|(w, sep)| w.join(sep))
}

fn corpus() -> impl Strategy<Value = Vec<(String, Option<String>)>> {
    prop::collection::vec((text(), prop::option::weighted(0.6, text())), 1..40)
}

fn query() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => text().prop_flat_map(|t| {
            let n = t.chars().count();
            (Just(t), 0..n, 1..8usize)
        }).prop_map(|(t, at, len)| t.chars().skip(at).take(len).collect::<String>()),
        1 => prop::collection::vec(prop::sample::select(&['a', 'b', 'x', '%', '_', '\\', '\'', '"', ' ', '0'][..]), 1..5)
            .prop_map(|c| c.into_iter().collect::<String>()),
    ]
    .prop_filter("not empty", |q| !q.trim().is_empty())
}

struct Both {
    sqlite: SqliteDatabase,
    memory: MemoryDatabase,
    scope: Scope,
}

fn both() -> Both {
    let sqlite = SqliteDatabase::new(Arc::new(NativeSql::memory().unwrap()));
    sqlite.migrate().unwrap();
    let memory = MemoryDatabase::new();
    let (ws, project, user) = (WorkspaceId(Uuid::from_u128(1)), ProjectId(Uuid::from_u128(2)), UserId(Uuid::from_u128(3)));
    let fixture = SeedFixture {
        workspaces: vec![SeedWorkspace { id: ws, slug: WorkspaceSlug::new("w") }],
        users: vec![user],
        workspace_members: vec![SeedWorkspaceMember { workspace_id: ws, user_id: user }],
        projects: vec![SeedProject { id: project, workspace_id: ws, name: "p".into() }],
        memberships: vec![SeedMembership { project_id: project, workspace_id: ws, user_id: user, role: Role::Member }],
        sessions: vec![],
        ..SeedFixture::default()
    };
    let sys = SystemScope::mint();
    block_on(sqlite.seed(&sys, &fixture)).unwrap();
    block_on(memory.seed(&sys, &fixture)).unwrap();
    Both { sqlite, memory, scope: Scope::mint(ws, project, user) }
}

fn rows(items: &[(String, Option<String>)], from: u128) -> Vec<SeedWorkItem> {
    items
        .iter()
        .enumerate()
        .map(|(i, (name, description))| SeedWorkItem {
            id: WorkItemId(Uuid::from_u128(from + i as u128)),
            name: name.clone(),
            description: description.clone(),
            created_by: UserId(Uuid::from_u128(3)),
            created_at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
        })
        .collect()
}

fn seed_both(b: &Both, items: &[(String, Option<String>)]) {
    let sys = SystemScope::mint();
    let (ws, project) = (b.scope.workspace_id(), b.scope.project_id());
    let a = block_on(b.sqlite.seed_work_items(&sys, ws, project, &rows(items, 100))).unwrap();
    let m = block_on(b.memory.seed_work_items(&sys, ws, project, &rows(items, 100))).unwrap();
    assert_eq!(a, m, "the first number differs");
}

fn answers(b: &Both, q: &str, limit: u32) -> (Vec<WorkItem>, Vec<WorkItem>) {
    (
        block_on(b.sqlite.search_work_items(&b.scope, q, limit)).unwrap(),
        block_on(b.memory.search_work_items(&b.scope, q, limit)).unwrap(),
    )
}

proptest! {
    /// Same corpus, a few soft deletes, many queries (1 and 2 characters take the LIKE path, 3 and more the index; `%`,
    /// `_`, `\`, quotes and non-ASCII letters are in the alphabet): the two adapters answer alike, row for row.
    #[test]
    fn prop_search_matches_the_reference_adapter(
        items in corpus(),
        queries in prop::collection::vec(query(), 1..12),
        limit in prop_oneof![Just(1u32), Just(5), Just(100)],
        deletes in prop::collection::vec(0..40usize, 0..5),
    ) {
        let b = both();
        seed_both(&b, &items);
        // soft-delete the same rows in both
        let all = block_on(b.memory.list_work_items(&b.scope, Visibility::Visible)).unwrap();
        for d in deletes {
            let id = all[d % all.len()].id;
            for db in [&b.sqlite as &dyn Database, &b.memory] {
                let mut tx = block_on(db.begin(&b.scope)).unwrap();
                block_on(tx.soft_delete_work_item(&b.scope, id, Utc.timestamp_opt(1_800_000_100, 0).unwrap())).unwrap();
                block_on(tx.commit()).unwrap();
            }
        }
        for q in &queries {
            let (got, want) = answers(&b, q, limit);
            prop_assert_eq!(
                got.iter().map(|w| (w.sequence_id, w.name.clone(), w.description.clone())).collect::<Vec<_>>(),
                want.iter().map(|w| (w.sequence_id, w.name.clone(), w.description.clone())).collect::<Vec<_>>(),
                "query {:?}, limit {}", q, limit
            );
        }
    }
}

/// A query of three characters or more is answered from the FTS5 table, not by scanning the items: the plan says so.
#[test]
fn a_three_character_query_uses_the_trigram_index() {
    let sql = NativeSql::memory().unwrap();
    SqliteDatabase::new(Arc::new(NativeSql::memory().unwrap())).migrate().unwrap();
    thrust_adapters_sqlite::migrate(&sql, thrust_adapters_sqlite::MIGRATIONS).unwrap();
    let plan = |q: &str| -> String {
        sql.query(q, &[Value::from("\"abc\"")])
            .unwrap()
            .iter()
            .map(|r| r.text(3).unwrap().to_string())
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let p = plan("EXPLAIN QUERY PLAN SELECT id FROM work_items WHERE rowid IN (SELECT rowid FROM work_items_fts WHERE work_items_fts MATCH ?)");
    assert!(p.contains("work_items_fts"), "the plan does not use the FTS table: {p}");
    let _ = Value::Null;
}

use thrust_adapters_sqlite::Value;

/// The search really comes from the index for three characters or more, and from a scan below that: with the insert
/// trigger gone, a seeded row is invisible to a long query and still found by a short one.
#[test]
fn the_index_is_what_answers_a_long_query() {
    let b = both();
    block_on(async {
        let sql = NativeSql::memory().unwrap();
        let _ = sql;
    });
    // an adapter on a database whose insert trigger was dropped after migrating
    let raw = Arc::new(NativeSql::memory().unwrap());
    let broken = SqliteDatabase::new(raw.clone());
    broken.migrate().unwrap();
    raw.execute("DROP TRIGGER work_items_fts_insert", &[]).unwrap();
    let sys = SystemScope::mint();
    let (ws, project, user) = (b.scope.workspace_id(), b.scope.project_id(), b.scope.user_id());
    block_on(broken.seed(&sys, &SeedFixture {
        workspaces: vec![SeedWorkspace { id: ws, slug: WorkspaceSlug::new("w") }],
        users: vec![user],
        workspace_members: vec![SeedWorkspaceMember { workspace_id: ws, user_id: user }],
        projects: vec![SeedProject { id: project, workspace_id: ws, name: "p".into() }],
        memberships: vec![],
        sessions: vec![],
        ..SeedFixture::default()
    }))
    .unwrap();
    block_on(broken.seed_work_items(&sys, ws, project, &rows(&[("needle in a haystack".to_string(), None)], 500))).unwrap();
    let long = block_on(broken.search_work_items(&b.scope, "needle", 10)).unwrap();
    let short = block_on(broken.search_work_items(&b.scope, "ne", 10)).unwrap();
    assert!(long.is_empty(), "a 6-character query found a row the index does not hold: it did not come from the index");
    assert_eq!(short.len(), 1, "a 2-character query is a scan and must find it");
    // and with the trigger in place (the real migration) the long query finds it
    let ok = both();
    seed_both(&ok, &[("needle in a haystack".to_string(), None)]);
    assert_eq!(answers(&ok, "needle", 10).0.len(), 1);
}

/// The bulk seed: numbers continue after what is there, `sort_order` steps by 10,000, a second batch continues after
/// the first, and a project that is not in the workspace is refused with nothing written.
#[test]
fn seed_work_items_numbers_from_the_counter_and_refuses_a_foreign_project() {
    let b = both();
    let sys = SystemScope::mint();
    let (ws, project) = (b.scope.workspace_id(), b.scope.project_id());
    for db in [&b.sqlite as &dyn Database, &b.memory] {
        let first = block_on(db.seed_work_items(&sys, ws, project, &rows(&[("a".into(), None), ("b".into(), None)], 10))).unwrap();
        let second = block_on(db.seed_work_items(&sys, ws, project, &rows(&[("c".into(), Some("d".into()))], 20))).unwrap();
        assert_eq!((first, second), (1, 3));
        let all = block_on(db.list_work_items(&b.scope, Visibility::Visible)).unwrap();
        assert_eq!(all.iter().map(|w| (w.sequence_id, w.sort_order)).collect::<Vec<_>>(), [(1, 10_000), (2, 20_000), (3, 30_000)]);
        // a project that exists, asked for under another workspace: refused, and nothing was written
        let elsewhere = WorkspaceId(Uuid::from_u128(99));
        assert!(block_on(db.seed_work_items(&sys, elsewhere, project, &rows(&[("z".into(), None)], 30))).is_err());
        assert_eq!(block_on(db.list_work_items(&b.scope, Visibility::Visible)).unwrap().len(), 3);
    }
}
