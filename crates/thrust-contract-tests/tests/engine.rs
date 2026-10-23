//! The ENGINE pass (`scripts/contract.sh <target> engine`): the checks that decide the database engine (spike 0002,
//! issue #37, ADR-0040) rather than the slice's behaviour. They live in their own test binary, not in `contract.rs`, so
//! the main pass and its expected count (`EXPECT_MAIN`) are untouched, and every test is named `engine_`.
//!
//! The pass starts the target with a very high create rate limit (the main pass's 30 per user per window would refuse
//! the creates below), and runs against every target that has an engine: `native-pg`, `native-sqlite`, `worker-cf`,
//! `worker-do`, and the memory sets as the reference.

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
/// Control: `scripts/mutants.sh` runs this pass against `mutant_dup_sequence` (memory and SQLite adapters) and, on the
/// SQLite adapter, `mutant_no_gate` (transactions not serialised): each must fail it.
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
