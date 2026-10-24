//! The ENGINE pass (`scripts/contract.sh <target> engine`): the checks that decide the database engine (spike 0002,
//! issue #37, ADR-0040) rather than the slice's behaviour. They live in their own test binary, not in `contract.rs`, so
//! the main pass and its expected count (`EXPECT_MAIN`) are untouched, and every test is named `engine_`.
//!
//! The pass starts the target with a very high create rate limit (the main pass's 30 per user per window would refuse
//! the creates below), and runs against every target that has an engine: `native-pg`, `worker-cf`, and the memory sets as
//! the reference. (`native-sqlite` and `worker-do` ran it too: that code is archived on the tag `spike/0002-worker-do`.)

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;
use thrust_contract_tests::*;
use thrust_contracts::*;

/// R-F2, ADR-0040 S1: "10k creates from at least 50 parallel writers in one project". The issue and the spike say 10k
/// in total (not 50 x 10k): 200 creates per writer.
const WRITERS: usize = 50;
const PER_WRITER: usize = 200;
/// The second round, after the deletes: 10 more per writer.
const PER_WRITER_AFTER: usize = 10;

async fn make_writers(t: &Target, f: &Fixture) -> Arc<Vec<User>> {
    let mut v = Vec::with_capacity(WRITERS);
    for _ in 0..WRITERS {
        v.push(f.extra_member(t).await);
    }
    Arc::new(v)
}

/// `WRITERS` bounded tasks, each making `per_writer` creates one after the other in project A. Stops every writer at the
/// first create that is not a 201 (a broken engine would otherwise spend the whole budget on errors), and reports it.
async fn write_in_parallel(t: &Arc<Target>, f: &Arc<Fixture>, users: &Arc<Vec<User>>, per_writer: usize, tag: &str) -> Vec<WorkItem> {
    let stop = Arc::new(AtomicBool::new(false));
    let tasks: Vec<_> = (0..WRITERS)
        .map(|w| {
            let (t, f, users, stop, tag) = (t.clone(), f.clone(), users.clone(), stop.clone(), tag.to_string());
            tokio::spawn(async move {
                let mut made = Vec::with_capacity(per_writer);
                for i in 0..per_writer {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let r = create(&t, &users[w], &f.a, json!({ "name": format!("{tag}-w{w}-{i}") })).await;
                    let status = r.status();
                    if status != 201 {
                        stop.store(true, Ordering::Relaxed);
                        return Err(format!("writer {w}, create {i}: HTTP {status}: {}", r.text().await.unwrap_or_default()));
                    }
                    match r.json::<WorkItem>().await {
                        Ok(item) => made.push(item),
                        Err(e) => {
                            stop.store(true, Ordering::Relaxed);
                            return Err(format!("writer {w}, create {i}: unreadable body: {e}"));
                        }
                    }
                }
                Ok(made)
            })
        })
        .collect();
    let mut items = Vec::with_capacity(WRITERS * per_writer);
    let mut errors = Vec::new();
    for task in tasks {
        match task.await.expect("a writer task panicked") {
            Ok(mut made) => items.append(&mut made),
            Err(e) => errors.push(e),
        }
    }
    assert!(errors.is_empty(), "{} of {WRITERS} writers failed; the first: {}", errors.len(), errors[0]);
    items
}

/// Sequence numbers that were handed out twice, and the first of them.
fn duplicates(seqs: &[i64]) -> Vec<i64> {
    let mut seen = BTreeSet::new();
    seqs.iter().copied().filter(|s| !seen.insert(*s)).collect()
}

/// Spike 0002 exit criterion 1, step 2 (R-F2, ADR-0040 S1, ADR-0043 B1): 10,000 creates from 50 parallel writers in one
/// project give no duplicate `sequence_id`, contiguous numbers, `sort_order` allocated with them; then deletes and
/// further creates show no number is reused. The contract suite's own 20-writer test (`contract.rs`) is the small
/// version of this one.
///
/// Control: `scripts/mutants.sh` runs this pass against the memory adapter's `mutant_dup_sequence`: it must fail it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn engine_fifty_writers_ten_thousand_creates_never_duplicate_a_sequence_id() {
    let t = Arc::new(Target::from_env());
    let f = Arc::new(Fixture::seed(&t).await);
    let users = make_writers(&t, &f).await;
    let total = WRITERS * PER_WRITER;

    // 1. the race itself
    let items = write_in_parallel(&t, &f, &users, PER_WRITER, "a").await;
    assert_eq!(items.len(), total);
    let mut seqs: Vec<i64> = items.iter().map(|w| w.sequence_id).collect();
    let dups = duplicates(&seqs);
    assert!(dups.is_empty(), "{} duplicate sequence ids in {total} creates, the first: {}", dups.len(), dups[0]);
    seqs.sort_unstable();
    assert_eq!(seqs, (1..=total as i64).collect::<Vec<_>>(), "sequence ids must be contiguous from 1");
    let mut by_seq: Vec<&WorkItem> = items.iter().collect();
    by_seq.sort_by_key(|w| w.sequence_id);
    assert!(
        by_seq.windows(2).all(|p| p[0].sort_order < p[1].sort_order),
        "sort_order must be unique and increasing with sequence_id (allocated in the same critical section)"
    );
    // every id is a distinct row, not one row answered twice
    assert_eq!(items.iter().map(|w| w.id).collect::<BTreeSet<_>>().len(), total);

    // 2. soft-delete every 7th number, from 50 tasks at once
    let doomed: Vec<WorkItem> = items.iter().filter(|w| w.sequence_id % 7 == 0).cloned().collect();
    let deleted_numbers: BTreeSet<i64> = doomed.iter().map(|w| w.sequence_id).collect();
    let chunks: Vec<Vec<WorkItem>> = (0..WRITERS).map(|w| doomed.iter().skip(w).step_by(WRITERS).cloned().collect()).collect();
    let tasks: Vec<_> = chunks
        .into_iter()
        .enumerate()
        .map(|(w, chunk)| {
            let (t, f, users) = (t.clone(), f.clone(), users.clone());
            tokio::spawn(async move {
                for item in chunk {
                    let r = t
                        .http
                        .delete(t.url(&format!("{}/{}", wi_path(&f.a), item.id)))
                        .bearer_auth(&users[w].token)
                        .send()
                        .await
                        .unwrap();
                    assert_eq!(r.status(), 204, "delete of #{}", item.sequence_id);
                }
            })
        })
        .collect();
    for task in tasks {
        task.await.expect("a deleter task panicked");
    }

    // 3. more creates: numbers continue after the last one and none of the deleted numbers comes back
    let after = write_in_parallel(&t, &f, &users, PER_WRITER_AFTER, "b").await;
    let mut seqs_after: Vec<i64> = after.iter().map(|w| w.sequence_id).collect();
    let dups_after = duplicates(&seqs_after);
    assert!(dups_after.is_empty(), "duplicate sequence ids after the deletes: {dups_after:?}");
    seqs_after.sort_unstable();
    let first = total as i64 + 1;
    assert_eq!(
        seqs_after,
        (first..first + (WRITERS * PER_WRITER_AFTER) as i64).collect::<Vec<_>>(),
        "after the deletes the next numbers must continue after {total}"
    );
    assert!(seqs_after.iter().all(|s| !deleted_numbers.contains(s)), "a deleted number was handed out again");
    assert!(
        after.iter().all(|w| w.sort_order > by_seq.last().unwrap().sort_order),
        "sort_order did not keep increasing after the deletes"
    );

    // 4. the whole project as an admin sees it: every number once, tombstones included, in sequence order
    let all: Vec<WorkItem> = list(&t, &f.admin_a, &f.a, Some("with_deleted")).await.json().await.unwrap();
    let grand_total = total + WRITERS * PER_WRITER_AFTER;
    assert_eq!(all.len(), grand_total);
    assert_eq!(all.iter().map(|w| w.sequence_id).collect::<Vec<_>>(), (1..=grand_total as i64).collect::<Vec<_>>());
    assert!(all.windows(2).all(|p| p[0].sort_order < p[1].sort_order), "sort_order is not increasing in sequence order");
    let tombstones: BTreeSet<i64> = all.iter().filter(|w| w.deleted_at.is_some()).map(|w| w.sequence_id).collect();
    assert_eq!(tombstones, deleted_numbers, "the deleted rows are not exactly the ones deleted");
    let visible: Vec<WorkItem> = list(&t, &f.guest_a, &f.a, None).await.json().await.unwrap();
    assert_eq!(visible.len(), grand_total - deleted_numbers.len());
}

// ---------------------------------------------------------------------------------------------------------------------
// Search (spike 0002 exit criterion 2, ADR-0083): substring queries return what Plane's `icontains` returns.
// ---------------------------------------------------------------------------------------------------------------------

/// The route returns at most this many work items, in sequence order (`thrust_core::SEARCH_LIMIT`).
const SEARCH_LIMIT: usize = 100;

/// Plane's `icontains`, written out: the oracle every engine is compared with. A work item matches when its name or its
/// description contains the query, ignoring case.
fn icontains(item: &(String, Option<String>), q: &str) -> bool {
    let q = q.to_lowercase();
    item.0.to_lowercase().contains(&q) || item.1.as_deref().is_some_and(|d| d.to_lowercase().contains(&q))
}

/// Words that collide on purpose: shared stems and case variants, plus the characters a LIKE pattern or a trigram
/// phrase would mistake for syntax (`%`, `_`, `\`, quotes). ASCII only: the engines fold case through different
/// locale rules beyond it, and that is reported separately below, not folded into the parity claim.
const WORDS: &[&str] = &[
    "fix", "Fix", "FIX", "login", "Login", "logout", "bug", "BUG", "debug", "timeout", "time", "Time", "queue", "Queue",
    "100%", "a_b", "a%b", "back\\slash", "it's", "say\"hi\"", "x", "ab", "abc", "ABC", "api", "API", "v2", "retry-after",
];

fn words() -> impl Strategy<Value = String> {
    (prop::collection::vec(prop::sample::select(WORDS), 1..4), prop::sample::select(&[" ", "-", "_", "", "/"][..]))
        .prop_map(|(w, sep)| w.join(sep))
}

fn corpus() -> impl Strategy<Value = Vec<(String, Option<String>)>> {
    prop::collection::vec((words(), prop::option::weighted(0.6, words())), 300..=300)
}

/// A query: usually a piece of an item (so there are hits, case flipped half the time), sometimes arbitrary (so
/// there are misses and syntax characters), 1 to 8 characters (1 and 2 cannot use a trigram index).
fn queries(items: Vec<(String, Option<String>)>) -> impl Strategy<Value = String> {
    let texts: Vec<String> = items.iter().flat_map(|(n, d)| std::iter::once(n.clone()).chain(d.clone())).collect();
    let from_item = (0..texts.len(), 0..40usize, 1..=8usize, any::<bool>()).prop_map(move |(i, at, len, flip)| {
        let chars: Vec<char> = texts[i].chars().collect();
        let start = at % chars.len();
        let piece: String = chars[start..(start + len).min(chars.len())].iter().collect();
        if flip { piece.chars().map(|c| if c.is_lowercase() { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }).collect() } else { piece }
    });
    let arbitrary = prop::collection::vec(prop::sample::select(&['a', 'b', 'c', 'x', '%', '_', '\\', '\'', '"', ' ', '-', '1', '0'][..]), 1..=5)
        .prop_map(|c| c.into_iter().collect::<String>());
    // a blank query is refused (400), so the generator makes none; a leading or trailing space is a real query
    prop_oneof![7 => from_item, 3 => arbitrary].prop_filter("not blank", |q| !q.trim().is_empty())
}

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;

/// Sequence ids a search answered with.
async fn search_ids(t: &Target, who: &User, at: &At, q: &str) -> Vec<i64> {
    let r = search(t, who, at, q).await;
    let status = r.status();
    assert_eq!(status, 200, "search for {q:?}: HTTP {status}: {:?}", r.text().await);
    r.json::<Vec<WorkItem>>().await.unwrap().iter().map(|w| w.sequence_id).collect()
}

/// Spike 0002 exit criterion 2: on a generated seed, substring queries answer exactly what `icontains` answers, as the
/// first `SEARCH_LIMIT` matches in sequence order, soft-deleted items excluded. 300 items seeded through the bulk route,
/// 20 of them deleted through the API, 400 generated queries. The case a trigram index cannot answer (1 and 2
/// characters) and the characters that are syntax in LIKE and in an FTS phrase are in the generator on purpose.
///
/// Rule 8, the control: the oracle is checked against a deliberately wrong one (case-sensitive, and prefix-only) and must
/// disagree with it on this very corpus, so a corpus that every matcher answers alike cannot pass here.
#[tokio::test]
async fn engine_search_matches_icontains_on_a_generated_seed() {
    let t = Target::from_env();
    let f = Fixture::seed(&t).await;
    let mut runner = TestRunner::deterministic();
    let seed = corpus().new_tree(&mut runner).unwrap().current();
    let report = seed_work_items(&t, &f, &f.a, f.admin_a.id, &seed).await;
    assert_eq!((report.first_sequence_id, report.count), (1, seed.len() as u64));

    // the project as the API shows it: the seeded rows, in sequence order, with their ids
    let mut rows: Vec<WorkItem> = list(&t, &f.guest_a, &f.a, None).await.json().await.unwrap();
    assert_eq!(rows.iter().map(|w| (w.name.clone(), w.description.clone())).collect::<Vec<_>>(), seed, "the seed route changed what it was given");
    // delete 20 of them: their text must stop matching
    let mut alive: Vec<(i64, (String, Option<String>))> = rows.iter().map(|w| (w.sequence_id, (w.name.clone(), w.description.clone()))).collect();
    for n in (0..seed.len()).step_by(15).take(20) {
        let r = t.http.delete(t.url(&format!("{}/{}", wi_path(&f.a), rows[n].id))).bearer_auth(&f.member_a.token).send().await.unwrap();
        assert_eq!(r.status(), 204);
        alive.retain(|(seq, _)| *seq != rows[n].sequence_id);
    }
    rows.clear();

    let mut hits_total = 0usize;
    let mut empty = 0usize;
    let mut capped = 0usize;
    let mut differ_from_case_sensitive = 0usize;
    let mut differ_from_prefix = 0usize;
    let strategy = queries(seed.clone());
    for _ in 0..400 {
        let q = strategy.new_tree(&mut runner).unwrap().current();
        let want: Vec<i64> = alive.iter().filter(|(_, item)| icontains(item, &q)).map(|(seq, _)| *seq).take(SEARCH_LIMIT).collect();
        let got = search_ids(&t, &f.guest_a, &f.a, &q).await;
        assert_eq!(got, want, "search for {q:?} disagrees with icontains");
        hits_total += want.len();
        empty += want.is_empty() as usize;
        capped += (want.len() == SEARCH_LIMIT) as usize;
        let wrong_case: Vec<i64> = alive.iter().filter(|(_, (n, d))| n.contains(&q) || d.as_deref().is_some_and(|d| d.contains(&q))).map(|(s, _)| *s).take(SEARCH_LIMIT).collect();
        let wrong_prefix: Vec<i64> = alive.iter().filter(|(_, (n, d))| n.to_lowercase().starts_with(&q.to_lowercase()) || d.as_deref().is_some_and(|d| d.to_lowercase().starts_with(&q.to_lowercase()))).map(|(s, _)| *s).take(SEARCH_LIMIT).collect();
        differ_from_case_sensitive += (wrong_case != want) as usize;
        differ_from_prefix += (wrong_prefix != want) as usize;
    }
    // the corpus and the queries exercised the engine: hits, misses, the cap, and both wrong oracles told apart from the right one
    assert!(hits_total > 1000, "the queries found almost nothing: {hits_total} hits");
    assert!(empty > 10 && empty < 300, "{empty} of 400 queries had no match");
    assert!(capped > 0, "no query reached the {SEARCH_LIMIT}-result cap, so the cap was not exercised");
    assert!(differ_from_case_sensitive > 20, "a case-sensitive matcher differed from icontains on only {differ_from_case_sensitive} queries: the corpus does not test case folding");
    assert!(differ_from_prefix > 20, "a prefix-only matcher differed from icontains on only {differ_from_prefix} queries: the corpus does not test substring matching");
    eprintln!("SEARCH-OBSERVATION: 400 queries, {hits_total} hits, {empty} empty, {capped} capped");
}

/// Freshness and scope (exit criterion 2: "1-minute freshness"; ADR-0083: tenant-aware results). A created item is found
/// by the very next search, a deleted one is not, `%` `_` and `\` are literal, and nothing crosses a tenant or a role
/// line. Every allowed case has its neighbouring rejected case.
#[tokio::test]
async fn engine_search_is_fresh_literal_and_scoped() {
    let t = Target::from_env();
    let f = Fixture::seed(&t).await;
    let needle = format!("zq{}x", uuid::Uuid::new_v4().simple());

    // fresh: found at once after the create (the bound is 60 s; nothing here waits), gone at once after the delete
    assert!(search_ids(&t, &f.guest_a, &f.a, &needle).await.is_empty(), "found before it exists");
    let item = create(&t, &f.member_a, &f.a, json!({ "name": format!("A {needle} b"), "description": "plain" })).await.json::<WorkItem>().await.unwrap();
    assert_eq!(search_ids(&t, &f.guest_a, &f.a, &needle).await, [item.sequence_id], "not searchable right after the create");
    assert_eq!(search_ids(&t, &f.guest_a, &f.a, &needle.to_uppercase()).await, [item.sequence_id], "case matters");
    let r = t.http.delete(t.url(&format!("{}/{}", wi_path(&f.a), item.id))).bearer_auth(&f.member_a.token).send().await.unwrap();
    assert_eq!(r.status(), 204);
    assert!(search_ids(&t, &f.guest_a, &f.a, &needle).await.is_empty(), "a deleted item is still searchable");

    // a description is searched too, and a name and a description of one item are one hit
    let both = create(&t, &f.member_a, &f.a, json!({ "name": format!("n-{needle}"), "description": format!("d-{needle}") })).await.json::<WorkItem>().await.unwrap();
    assert_eq!(search_ids(&t, &f.guest_a, &f.a, &needle).await, [both.sequence_id]);
    let only_desc = create(&t, &f.member_a, &f.a, json!({ "name": "no match here", "description": format!("see {needle}2") })).await.json::<WorkItem>().await.unwrap();
    assert_eq!(search_ids(&t, &f.guest_a, &f.a, &format!("{needle}2")).await, [only_desc.sequence_id]);

    // `%`, `_` and `\` are literal characters, not pattern syntax
    for name in ["100% done", "100 done", "a_b", "axb", "c\\d", "cd"] {
        create_ok(&t, &f.member_a, &f.a, name).await;
    }
    let names = |hits: Vec<WorkItem>| hits.into_iter().map(|w| w.name).collect::<Vec<_>>();
    for (q, want) in [("0% d", vec!["100% done"]), ("a_b", vec!["a_b"]), ("c\\d", vec!["c\\d"])] {
        let hits: Vec<WorkItem> = search(&t, &f.guest_a, &f.a, q).await.json().await.unwrap();
        assert_eq!(names(hits), want, "{q:?} was read as pattern syntax");
    }

    // tenant and role lines: B has an item with the same text, and A's search must not return it
    let other = create(&t, &f.admin_b, &f.b, json!({ "name": format!("tenant b {needle}") })).await.json::<WorkItem>().await.unwrap();
    assert_eq!(
        search_ids(&t, &f.guest_a, &f.a, &needle).await,
        [both.sequence_id, only_desc.sequence_id],
        "another tenant's item was returned (or one of A's went missing)"
    );
    assert_eq!(search_ids(&t, &f.admin_b, &f.b, &needle).await, [other.sequence_id], "B does not find its own item");
    assert_eq!(search(&t, &f.admin_b, &f.a, &needle).await.status(), 404, "cross-tenant search");
    assert_eq!(search(&t, &f.outsider_a, &f.a, &needle).await.status(), 403, "a workspace member who is not in the project");
    let r = t.http.get(t.url(&search_path(&f.a))).query(&[("q", needle.as_str())]).send().await.unwrap();
    assert_eq!(r.status(), 401, "unauthenticated search");
    for bad in ["", "   ", &"x".repeat(101)] {
        assert_eq!(search(&t, &f.guest_a, &f.a, bad).await.status(), 400, "q = {bad:?}");
    }
    let r = t.http.get(t.url(&search_path(&f.a))).bearer_auth(&f.guest_a.token).send().await.unwrap();
    assert_eq!(r.status(), 400, "no q at all");
}

/// The bulk-seed route (`/internal/seed-work-items`, the way the benches get to 100k rows): numbers come from the
/// project's own counter, so they continue after the work items created through the API and the next API create
/// continues after them; `sort_order` keeps increasing; the rows are ordinary work items.
#[tokio::test]
async fn engine_seed_work_items_numbers_from_the_projects_counter() {
    let t = Target::from_env();
    let f = Fixture::seed(&t).await;
    for n in ["one", "two", "three"] {
        create_ok(&t, &f.member_a, &f.a, n).await;
    }
    let batch: Vec<(String, Option<String>)> = (0..50).map(|i| (format!("seeded {i}"), (i % 2 == 0).then(|| format!("desc {i}")))).collect();
    let report = seed_work_items(&t, &f, &f.a, f.admin_a.id, &batch).await;
    assert_eq!((report.first_sequence_id, report.count), (4, 50));
    // a second batch continues after the first
    let more = seed_work_items(&t, &f, &f.a, f.admin_a.id, &batch[..5]).await;
    assert_eq!((more.first_sequence_id, more.count), (54, 5));
    let next = create_ok(&t, &f.member_a, &f.a, "after the seed").await;
    assert_eq!(next.sequence_id, 59, "the API create did not continue after the seeded numbers");
    let rows: Vec<WorkItem> = list(&t, &f.guest_a, &f.a, None).await.json().await.unwrap();
    assert_eq!(rows.len(), 59);
    assert_eq!(rows.iter().map(|w| w.sequence_id).collect::<Vec<_>>(), (1..=59).collect::<Vec<i64>>());
    assert!(rows.windows(2).all(|p| p[0].sort_order < p[1].sort_order));
    assert_eq!(rows[3].name, "seeded 0");
    assert_eq!(rows[3].description.as_deref(), Some("desc 0"));
    assert_eq!(rows[4].description, None);
    assert!(rows[3..53].iter().all(|w| w.created_by == f.admin_a.id && w.workspace_id == f.ws_a && w.deleted_at.is_none() && !w.is_draft));
    // the other project is untouched: its counter is its own
    assert_eq!(create_ok(&t, &f.admin_b, &f.b, "b").await.sequence_id, 1);
    // control: the route is a test route, closed without the admin token
    let r = t.http.post(t.url("/internal/seed-work-items")).json(&json!({})).send().await.unwrap();
    assert_eq!(r.status(), 401);
}
