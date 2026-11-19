//! Retention (issue #59, F-D10, deviation D-001) against a REAL Postgres: `purgeable_work_items`, `purge_work_items`, `purge_outbox` and
//! `work_item_deleted_at`. The memory adapter is held to the same rules by `crates/adapters/memory/tests/retention.rs`, which states them;
//! this file adds what only the tables can show: the foreign keys that are all NO ACTION (so a purge has to take a work item, what hangs
//! on it and the name its children give it in one transaction), the rows that are left, read with SQL on the table, and a cutoff bound as
//! a typed timestamp, to the microsecond.
//!
//! The cases are generated against one container, each in a tenant of its own (as `assets` does): the work items are written with SQL, with
//! the parents and the deletion times the case says, so that a state the API cannot make is here too (a deleted work item with a live
//! one under it, which only a race makes).

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, TestCaseError, TestRunner};

use super::*;

/// The cutoff, in seconds from the base time of `at`.
const CUTOFF: i64 = 100_000;

fn cutoff() -> Timestamp {
    at(CUTOFF)
}

fn config() -> Config {
    // each case makes a tenant, up to eight work items with their history and assignees, and reads the tables back: tens of cases
    let cases = std::env::var("THRUST_MODEL_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(48);
    Config { cases, max_shrink_iters: 256, max_shrink_time: 60_000, ..Config::default() }
}

/// A workspace and a project of their own, with an owner who is a user, and no work item.
async fn tenant(env: &Env, tag: &str) -> (Scope, UserId) {
    let (s, owner, _) = tenant_with_items(env, tag, 0).await;
    (s, owner)
}

/// Work items of the tenant's project written with SQL, as the case says: item `i` was deleted `times[i]` seconds from the cutoff (`None`:
/// it is live) and is under item `parents[i]` (any item: itself and a loop included).
async fn raw_items(
    raw: &tokio_postgres::Client,
    s: &Scope,
    parents: &[Option<usize>],
    times: &[Option<i64>],
) -> Vec<WorkItemId> {
    let ids: Vec<WorkItemId> = parents.iter().map(|_| WorkItemId(new_id())).collect();
    for (i, id) in ids.iter().enumerate() {
        let deleted: Option<Timestamp> = times[i].map(|secs| at(CUTOFF + secs));
        let number = i as i64 + 1;
        raw.execute(
            "INSERT INTO work_items (id, workspace_id, project_id, sequence_id, sort_order, name, created_at, deleted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[&id.0, &s.workspace_id().0, &s.project_id().0, &number, &number, &format!("item {i}"), &at(0), &deleted],
        )
        .await
        .unwrap();
    }
    for (i, parent) in parents.iter().enumerate() {
        if let Some(p) = parent {
            raw.execute("UPDATE work_items SET parent_id = $2 WHERE id = $1", &[&ids[i].0, &ids[*p].0]).await.unwrap();
        }
    }
    ids
}

async fn assign(raw: &tokio_postgres::Client, s: &Scope, item: WorkItemId, user: UserId) {
    raw.execute(
        "INSERT INTO work_item_assignees (work_item_id, workspace_id, project_id, user_id, created_at) VALUES ($1, $2, $3, $4, $5)",
        &[&item.0, &s.workspace_id().0, &s.project_id().0, &user.0, &at(0)],
    )
    .await
    .unwrap();
}

/// How many rows of the table name one of these work items.
async fn rows_of(raw: &tokio_postgres::Client, table: &str, items: &[WorkItemId]) -> i64 {
    let ids: Vec<Uuid> = items.iter().map(|i| i.0).collect();
    raw.query_one(&format!("SELECT count(*) FROM {table} WHERE work_item_id = ANY($1)"), &[&ids]).await.unwrap().get(0)
}

/// The work items of a project as the table has them: id to parent.
async fn left(raw: &tokio_postgres::Client, s: &Scope) -> BTreeMap<Uuid, Option<Uuid>> {
    raw.query("SELECT id, parent_id FROM work_items WHERE project_id = $1", &[&s.project_id().0])
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get("id"), r.get("parent_id")))
        .collect()
}

// --- the work items -------------------------------------------------------------------------------------------------------------

/// A forest (each work item's parent is any work item, itself and a loop included, or none), the time each was deleted, as seconds from
/// the cutoff (`None`: live; a few seconds either side of it, or long before), and a page size.
type Forest = (Vec<Option<usize>>, Vec<Option<i64>>, u32);

fn forest() -> impl Strategy<Value = Forest> {
    (1usize..9).prop_flat_map(|n| {
        (
            prop::collection::vec(prop::option::of(0..n), n),
            prop::collection::vec(
                prop_oneof![2 => Just(None), 3 => (-2i64..=2).prop_map(Some), 1 => Just(Some(-50_000i64))],
                n,
            ),
            1u32..4,
        )
    })
}

/// The rule, computed from the case: a work item is purgeable when it was deleted before the cutoff and so was every work item under
/// it. (One that is its own parent is old, so it never keeps itself.)
fn expected(parents: &[Option<usize>], times: &[Option<i64>]) -> BTreeSet<usize> {
    let old = |t: Option<i64>| t.is_some_and(|secs| secs < 0);
    (0..parents.len())
        .filter(|&i| old(times[i]) && (0..parents.len()).filter(|&c| parents[c] == Some(i)).all(|c| old(times[c])))
        .collect()
}

async fn forest_case(env: &Env, (parents, times, page): Forest) -> Result<(), TestCaseError> {
    let (s, owner) = tenant(env, "forest").await;
    let raw = env.conn.raw().await.unwrap();
    let sys = SystemScope::mint();
    let ids = raw_items(&raw, &s, &parents, &times).await;
    for id in &ids {
        env.db.insert_work_item_activities(&sys, &[hrow(&s, *id, new_id(), 0, 0, Some(owner))]).await.unwrap();
        assign(&raw, &s, *id, owner).await;
    }
    let want = expected(&parents, &times);

    // what is listed: the work items of the rule, in the order of the key, whole or a page at a time
    let all = env.db.purgeable_work_items(&sys, cutoff(), None, 100).await.unwrap();
    let listed: BTreeSet<usize> = all.iter().map(|c| ids.iter().position(|id| *id == c.id).unwrap()).collect();
    prop_assert_eq!(&listed, &want, "the work items deleted before the cutoff whose children were too");
    let keys: Vec<(Timestamp, WorkItemId)> = all.iter().map(|c| (c.deleted_at, c.id)).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    prop_assert_eq!(&keys, &sorted, "in the order of (deleted_at, id)");
    for c in &all {
        prop_assert_eq!((c.workspace_id, c.project_id), (s.workspace_id(), s.project_id()));
    }
    let mut paged = Vec::new();
    let mut after = None;
    loop {
        let got = env.db.purgeable_work_items(&sys, cutoff(), after, page).await.unwrap();
        prop_assert!(got.len() <= page as usize);
        let Some(last) = got.last() else { break };
        after = Some((last.deleted_at, last.id));
        paged.extend(got);
    }
    prop_assert_eq!(&paged, &all);

    // what is purged: those, with their history and assignees, and no other; a work item that stays loses the name of a parent that goes
    prop_assert_eq!(env.db.purge_work_items(&sys, &ids, cutoff()).await.unwrap(), want.len() as u64);
    let rows = left(&raw, &s).await;
    for (i, id) in ids.iter().enumerate() {
        if want.contains(&i) {
            prop_assert!(!rows.contains_key(&id.0), "item {} is gone", i);
        } else {
            let parent = parents[i].filter(|p| !want.contains(p)).map(|p| ids[p].0);
            prop_assert_eq!(rows.get(&id.0), Some(&parent), "item {} stays, with the parent that stays", i);
        }
    }
    let gone: Vec<WorkItemId> = want.iter().map(|&i| ids[i]).collect();
    let kept: Vec<WorkItemId> = (0..ids.len()).filter(|i| !want.contains(i)).map(|i| ids[i]).collect();
    for table in ["work_item_activities", "work_item_assignees"] {
        prop_assert_eq!(rows_of(&raw, table, &gone).await, 0, "{} of what went", table);
        prop_assert_eq!(rows_of(&raw, table, &kept).await, kept.len() as i64, "{} of what stays", table);
    }
    prop_assert_eq!(env.db.purge_work_items(&sys, &ids, cutoff()).await.unwrap(), 0, "a second run has nothing to do");
    prop_assert!(env.db.purgeable_work_items(&sys, cutoff(), None, 100).await.unwrap().is_empty());
    Ok(())
}

#[test]
fn prop_the_postgres_purge_takes_exactly_what_the_rule_says() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let env = rt.block_on(env());
    let config = config();
    assert!(config.cases > 0, "THRUST_MODEL_CASES=0 would check nothing");
    let mut runner = TestRunner::new(config);
    let result = runner.run(&forest(), |f| rt.block_on(forest_case(&env, f)));
    rt.block_on(async move { drop(env) });
    if let Err(e) = result {
        panic!("{e}");
    }
}

/// Rule 8's control for the generator: a forest that never has a self-parent, a loop, a work item on the cutoff, a deleted work item with
/// a child that is not old, or a work item that stays under one that goes would pass on a purge that got those wrong. Each is drawn.
#[test]
fn prop_the_forest_strategy_reaches_every_class() {
    let mut runner = TestRunner::deterministic();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for _ in 0..400 {
        let (parents, times, _) = forest().new_tree(&mut runner).unwrap().current();
        let want = expected(&parents, &times);
        for (i, parent) in parents.iter().enumerate() {
            let mut hit = |class| *seen.entry(class).or_insert(0usize) += 1;
            if *parent == Some(i) {
                hit("a work item under itself");
            }
            if parent.is_some_and(|p| p != i && parents[p] == Some(i)) {
                hit("a loop of two");
            }
            if times[i] == Some(0) {
                hit("a work item deleted on the cutoff");
            }
            if times[i].is_some_and(|s| s < 0) && !want.contains(&i) {
                hit("a work item deleted long ago that stays for a child that is not old");
            }
            if !want.contains(&i) && parent.is_some_and(|p| want.contains(&p)) {
                hit("a work item that stays under one that goes");
            }
            if times[i].is_none() {
                hit("a live work item");
            }
        }
    }
    assert_eq!(seen.len(), 6, "{seen:?}");
}

/// Spec examples the pin names: a work item may be its own parent, and two may be each other's. These, and a work item that stays under
/// ones that go, are deleted in one batch (the foreign key is checked after the statement, so none of them has to go first); a work item
/// with a live work item under it does not go.
#[tokio::test]
async fn a_work_item_under_itself_a_loop_and_a_chain_are_deleted_in_one_batch() {
    let env = env().await;
    let (s, _) = tenant(&env, "shapes").await;
    let raw = env.conn.raw().await.unwrap();
    let (old, live) = (Some(-5), None);
    // 0 is under itself; 1 and 2 are under each other; 3 is under 4 which is under 5 (so 5 is the root of a chain); 6 is old but has
    // the live 7 under it, and is itself under the old 8
    let parents = [Some(0), Some(2), Some(1), Some(4), Some(5), None, Some(8), Some(6), None];
    let times = [old, old, old, old, old, old, old, live, old];
    let ids = raw_items(&raw, &s, &parents, &times).await;
    let sys = SystemScope::mint();

    let got = env.db.purge_work_items(&sys, &ids, cutoff()).await.unwrap();
    assert_eq!(got, 7, "0 to 5 and 8 go");
    let rows = left(&raw, &s).await;
    assert_eq!(rows.keys().copied().collect::<BTreeSet<_>>(), BTreeSet::from([ids[6].0, ids[7].0]), "6 and 7 stay");
    assert_eq!(rows[&ids[6].0], None, "6 loses the parent that went");
    assert_eq!(rows[&ids[7].0], Some(ids[6].0), "7 keeps the one that stays");
}

/// Spec example, deviation D-043 (where Plane keeps the history): a purged work item takes its assignees, its labels and its history, and
/// the rows of the work item that stays are as they were. What the work item leaves is the `activity` projection (no foreign key to it)
/// and the project's counter (ADR-0043 B1: a number is not handed out again). The three answers of `work_item_deleted_at` are seen on the way.
#[tokio::test]
async fn d043_a_purged_work_item_takes_what_hangs_on_it_and_leaves_its_number_and_its_projection() {
    let env = env().await;
    let (s, owner, items) = tenant_with_items(&env, "join", 2).await;
    let (gone, kept) = (items[0], items[1]);
    let raw = env.conn.raw().await.unwrap();
    let sys = SystemScope::mint();
    let label = new_id();
    raw.execute(
        "INSERT INTO labels (id, workspace_id, project_id, parent_id, name, created_at, updated_at) VALUES ($1, $2, $3, NULL, 'bug', now(), now())",
        &[&label, &s.workspace_id().0, &s.project_id().0],
    )
    .await
    .unwrap();
    for item in [gone, kept] {
        assign(&raw, &s, item, owner).await;
        raw.execute(
            "INSERT INTO work_item_labels (work_item_id, workspace_id, project_id, label_id, created_at) VALUES ($1, $2, $3, $4, $5)",
            &[&item.0, &s.workspace_id().0, &s.project_id().0, &label, &at(0)],
        )
        .await
        .unwrap();
        env.db.insert_work_item_activities(&sys, &[hrow(&s, item, new_id(), 0, 0, Some(owner))]).await.unwrap();
    }
    let projection = Activity {
        id: ActivityId(new_id()),
        event_id: EventId(new_id()),
        workspace_id: s.workspace_id(),
        project_id: s.project_id(),
        entity_id: gone.0,
        verb: "created".into(),
        actor_id: Some(owner),
        created_at: at(0),
    };
    assert!(env.db.insert_activity(&sys, &projection).await.unwrap());

    assert_eq!(env.db.work_item_deleted_at(&sys, gone).await.unwrap(), Some(None), "live");
    let mut tx = env.db.begin(&s).await.unwrap();
    assert!(tx.soft_delete_work_item(&s, gone, at(CUTOFF - 1)).await.unwrap());
    tx.commit().await.unwrap();
    assert_eq!(env.db.work_item_deleted_at(&sys, gone).await.unwrap(), Some(Some(at(CUTOFF - 1))), "deleted");
    assert_eq!(env.db.purge_work_items(&sys, &[gone, kept], cutoff()).await.unwrap(), 1, "the live one stays");
    assert_eq!(env.db.work_item_deleted_at(&sys, gone).await.unwrap(), None, "gone");

    for table in ["work_item_assignees", "work_item_labels", "work_item_activities"] {
        assert_eq!(rows_of(&raw, table, &[gone]).await, 0, "{table} of the work item that went");
        assert_eq!(rows_of(&raw, table, &[kept]).await, 1, "{table} of the work item that stays");
    }
    assert_eq!(env.db.list_activity(&s, Some(gone)).await.unwrap(), vec![projection]);
    let last: i64 = raw
        .query_one("SELECT last_sequence_id FROM project_sequences WHERE project_id = $1", &[&s.project_id().0])
        .await
        .unwrap()
        .get(0);
    assert_eq!(last, 2, "the counter is not touched");
    let mut tx = env.db.begin(&s).await.unwrap();
    assert_eq!(tx.allocate(&s).await.unwrap().sequence_id, 3, "the number that went is not made again");
    tx.rollback().await.unwrap();
}

/// The cutoff is bound as a timestamp, not as text or a date, so it compares to the microsecond: a work item deleted one microsecond before
/// it goes, one deleted on it and one a microsecond after it stay. (An exhaustive matrix over the finite domain of the three.)
#[tokio::test]
async fn a_cutoff_is_compared_to_the_microsecond() {
    let env = env().await;
    let (s, _) = tenant(&env, "micros").await;
    let raw = env.conn.raw().await.unwrap();
    let sys = SystemScope::mint();
    let ids = raw_items(&raw, &s, &[None, None, None], &[Some(0), Some(0), Some(0)]).await;
    for (id, micros) in ids.iter().zip([-1i64, 0, 1]) {
        let when = cutoff() + thrust_contracts::chrono::Duration::microseconds(micros);
        raw.execute("UPDATE work_items SET deleted_at = $2 WHERE id = $1", &[&id.0, &when]).await.unwrap();
    }
    let got = env.db.purgeable_work_items(&sys, cutoff(), None, 10).await.unwrap();
    assert_eq!(got.iter().map(|c| c.id).collect::<Vec<_>>(), [ids[0]], "only the one deleted a microsecond before");
    assert_eq!(env.db.purge_work_items(&sys, &ids, cutoff()).await.unwrap(), 1);
    assert_eq!(left(&raw, &s).await.len(), 2);
}

// --- the outbox -----------------------------------------------------------------------------------------------------------------

/// Rows made in pairs of one time (so that the event breaks the tie), the cutoff on a pair's time or between two, some dispatched, written in
/// any order, and a limit: `(dispatched, insertion order, the first pair that is not old, limit)`.
type Rows = (Vec<bool>, Vec<usize>, usize, u32);

fn rows() -> impl Strategy<Value = Rows> {
    (0usize..12).prop_flat_map(|n| {
        (
            prop::collection::vec(any::<bool>(), n),
            Just((0..n).collect::<Vec<_>>()).prop_shuffle(),
            0..=n.div_ceil(2),
            0u32..14,
        )
    })
}

async fn outbox_case(env: &Env, (dispatched, order, boundary, limit): Rows) -> Result<(), TestCaseError> {
    let raw = env.conn.raw().await.unwrap();
    raw.execute("DELETE FROM outbox", &[]).await.unwrap();
    let ws = new_id();
    // row `i` was made `i / 2 - boundary` seconds from the cutoff, so the pairs before `boundary` are old, `boundary` is on it
    let created = |i: usize| at(CUTOFF + (i / 2) as i64 - boundary as i64);
    let event = |i: usize| Uuid::from_u128(0xB000 + i as u128);
    for i in order {
        let done = dispatched[i].then(|| created(i) + thrust_contracts::chrono::Duration::seconds(1));
        raw.execute(
            "INSERT INTO outbox (event_id, workspace_id, envelope, created_at, dispatched_at) VALUES ($1, $2, '{}'::jsonb, $3, $4)",
            &[&event(i), &ws, &created(i), &done],
        )
        .await
        .unwrap();
    }
    let sys = SystemScope::mint();
    let old: Vec<usize> = (0..dispatched.len()).filter(|&i| dispatched[i] && i / 2 < boundary).collect();
    let taken = old.len().min(limit as usize);
    prop_assert_eq!(
        env.db.purge_outbox(&sys, cutoff(), limit).await.unwrap(),
        taken as u64,
        "the outbox purge: the oldest dispatched rows made before the cutoff, at most the limit"
    );
    let rows: BTreeSet<Uuid> =
        raw.query("SELECT event_id FROM outbox", &[]).await.unwrap().iter().map(|r| r.get("event_id")).collect();
    let want: BTreeSet<Uuid> = (0..dispatched.len()).filter(|i| !old[..taken].contains(i)).map(event).collect();
    prop_assert_eq!(
        rows,
        want,
        "the outbox purge: the rows that are left, which are all but the oldest dispatched ones"
    );
    Ok(())
}

#[test]
fn prop_the_postgres_outbox_purge_takes_the_oldest_dispatched_rows_before_the_cutoff() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let env = rt.block_on(env());
    let config = config();
    assert!(config.cases > 0, "THRUST_MODEL_CASES=0 would check nothing");
    let mut runner = TestRunner::new(config);
    let result = runner.run(&rows(), |r| rt.block_on(outbox_case(&env, r)));
    rt.block_on(async move { drop(env) });
    if let Err(e) = result {
        panic!("{e}");
    }
}
