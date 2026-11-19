//! The daily hard delete (issue #59, F-D10, deviation D-001) through the use-cases: `run_retention_with` and its windows, over the memory
//! adapters (the adapters' own rules are in `thrust-adapters-memory/tests/retention.rs` and, on Postgres, `pg_adapter.rs`).
//!
//! The rules, written out. Every window is counted back from the run's `now`, and a row is old when its time is strictly before that:
//!
//! * A soft-deleted work item goes when it was deleted more than `hard_delete_after_days` ago, in either tenant, with its attachments;
//!   one deleted exactly that long ago, one deleted since, and a live one stay. The next work item made in its project gets the next
//!   number (a number is not handed out again, ADR-0043 B1).
//! * A dispatched outbox row goes when it was made more than `outbox_retention_days` ago. One that was not dispatched stays, however old.
//! * A run does at most `work_item_batches` batches of `work_item_batch` work items and `outbox_batches` of `outbox_batch` rows, and
//!   says it is `complete` only when it left nothing it could have taken. A work item whose blob will not be deleted is left, and
//!   counted in `failed`, for the next run.
//! * A window is a whole number of days, 0 or more: anything else is an error that names the variable, and an unset or empty one is the
//!   default (30 days for a work item, 90 for the outbox).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

const BASE: i64 = 1_760_000_000;
const DAY: i64 = 86_400;
/// The time the job runs, in seconds from `BASE`: far enough from the start for any age a case asks for.
const NOW: i64 = 1_000 * DAY;

fn ts(secs: i64) -> Timestamp {
    DateTime::from_timestamp(BASE + secs, 0).unwrap()
}

/// A clock the test moves, in seconds from `BASE`.
struct Clockwork(AtomicI64);
impl Clockwork {
    fn set(&self, secs: i64) {
        self.0.store(secs, Ordering::SeqCst);
    }
}
impl Clock for Clockwork {
    fn now(&self) -> Timestamp {
        ts(self.0.load(Ordering::SeqCst))
    }
}

/// A world with the windows a case chooses, a clock it moves and a recording queue (the sweeper needs somewhere to publish to). The fast
/// path is off, so that an outbox row waits for the sweeper, which the cases that need a row dispatched run themselves.
fn world_in(hard_delete_after_days: u32, outbox_retention_days: u32) -> (World, Arc<Clockwork>) {
    let clock = Arc::new(Clockwork(AtomicI64::new(0)));
    let cfg =
        CoreConfig { outbox_fastpath: false, hard_delete_after_days, outbox_retention_days, ..CoreConfig::default() };
    let w = world_built(cfg, Arc::new(RecordingQueue::default()), Arc::new(NoBackground), clock.clone());
    (w, clock)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tenant {
    A,
    B,
}

const BOTH: [Tenant; 2] = [Tenant::A, Tenant::B];

fn actor(t: Tenant) -> Actor {
    match t {
        Tenant::A => Actor::MemberA,
        Tenant::B => Actor::MemberB,
    }
}

fn slug(w: &World, t: Tenant) -> WorkspaceSlug {
    match t {
        Tenant::A => w.slug_a.clone(),
        Tenant::B => w.slug_b.clone(),
    }
}

fn project(w: &World, t: Tenant) -> ProjectId {
    match t {
        Tenant::A => w.project_a,
        Tenant::B => w.project_b,
    }
}

fn workspace(w: &World, t: Tenant) -> WorkspaceId {
    match t {
        Tenant::A => w.ws_a,
        Tenant::B => w.ws_b,
    }
}

/// The access `/api/v1` gives a member who makes and deletes work items.
fn access(w: &World, t: Tenant) -> IssueAccess {
    block_on(v1_issue_writer(&w.app, &w.principal(actor(t)), &slug(w, t), project(w, t))).unwrap()
}

/// A live work item of the tenant, made at the clock's time, as `/api/v1` makes it.
fn make(w: &World, t: Tenant) -> WorkItemRow {
    block_on(create_issue(&w.app, &access(w, t), IssueDraft { name: "item".into(), ..IssueDraft::default() })).unwrap()
}

/// Delete the work item at the time the clock says.
fn remove(w: &World, t: Tenant, id: WorkItemId) {
    block_on(delete_issue(&w.app, &access(w, t), id)).unwrap();
}

fn run(w: &World, now: i64) -> RetentionReport {
    block_on(run_retention_at(&w.app, ts(now))).unwrap()
}

fn run_with(w: &World, now: i64, limits: RetentionLimits) -> RetentionReport {
    block_on(run_retention_with(&w.app, ts(now), limits)).unwrap()
}

fn deleted_at(w: &World, id: WorkItemId) -> Option<Option<Timestamp>> {
    block_on(work_item_deleted_at(&w.app, id)).unwrap()
}

// --- the work items -------------------------------------------------------------------------------------------------------------

/// A window in days, and work items of either tenant, each live or deleted `over` seconds before the cutoff (so it goes exactly when
/// `over` is above 0: `-2` to `2` are the seconds either side of the cutoff, and the far values are a lifetime).
type Cases = (u32, Vec<(Tenant, Option<i64>)>);

fn cases() -> impl Strategy<Value = Cases> {
    let tenant = prop_oneof![Just(Tenant::A), Just(Tenant::B)];
    let over =
        prop_oneof![2 => Just(None), 3 => (-2i64..=2).prop_map(Some), 1 => (-300_000i64..300_000).prop_map(Some)];
    (0u32..120, prop::collection::vec((tenant, over), 0..7))
}

proptest! {
    #![proptest_config(proptest_config(64))]

    /// Over any window and any work items: a work item is purged exactly when it was deleted strictly more than the window ago, with
    /// its attachments; a live one never is; the next work item of a project is numbered after every one it ever had; and a second run
    /// has nothing to do. The model is the rule, with the cutoff counted here.
    #[test]
    fn prop_a_work_item_goes_exactly_when_it_was_deleted_more_than_the_window_ago((days, cases) in cases()) {
        let (w, clock) = world_in(days, 90);
        let cutoff = NOW - i64::from(days) * DAY;
        let mut made = Vec::new();
        for (t, over) in &cases {
            clock.set(0);
            let item = make(&w, *t);
            block_on(put_attachment(
                &w.app, &w.principal(actor(*t)), &slug(&w, *t), project(&w, *t), item.item.id, "a.txt", b"x".to_vec(), None,
            ))
            .unwrap();
            if let Some(over) = over {
                clock.set(cutoff - over);
                remove(&w, *t, item.item.id);
            }
            made.push((*t, item.item.id, *over));
        }
        clock.set(NOW);

        let report = run(&w, NOW);
        let goes = |over: &Option<i64>| over.is_some_and(|o| o > 0);
        let want = made.iter().filter(|(_, _, over)| goes(over)).count() as u64;
        prop_assert_eq!(report.work_items, want, "the work items deleted more than {} days ago", days);
        prop_assert_eq!(report.blobs, want, "one attachment each");
        prop_assert_eq!(report.work_item_cutoff, Some(ts(cutoff)));
        prop_assert!(report.complete);
        for (t, id, over) in &made {
            let prefix = format!("{}/{}/{}/", workspace(&w, *t), project(&w, *t), id);
            let blobs = block_on(w.app.ports.blobs.list(&prefix)).unwrap();
            if goes(over) {
                prop_assert_eq!(deleted_at(&w, *id), None, "gone");
                prop_assert!(blobs.is_empty(), "its attachments are gone with it");
            } else {
                prop_assert_ne!(deleted_at(&w, *id), None, "stays");
                prop_assert_eq!(blobs.len(), 1, "its attachment stays");
            }
        }

        for t in BOTH {
            let had = made.iter().filter(|(m, _, _)| *m == t).count() as i64;
            let next = make(&w, t);
            prop_assert_eq!(next.item.sequence_id, had + 1, "numbered after every work item the project ever had");
        }
        let again = run(&w, NOW);
        prop_assert_eq!(again.work_items, 0, "a second run has nothing to do");
        prop_assert_eq!(again.blobs, 0);
    }
}

/// Spec example, the acceptance criteria of #59 (30-day window, in both tenants): work items deleted 31 days and 30 days plus a second
/// ago are gone; one deleted exactly 30 days ago, one 29 days ago and a live one remain.
#[rstest]
fn a_work_item_goes_only_when_it_was_deleted_more_than_thirty_days_ago(
    #[values(31 * DAY, 30 * DAY + 1, 30 * DAY, 29 * DAY)] age: i64,
    #[values(Tenant::A, Tenant::B)] t: Tenant,
) {
    let (w, clock) = world_in(30, 90);
    let item = make(&w, t).item.id;
    let live = make(&w, t).item.id;
    clock.set(NOW - age);
    remove(&w, t, item);
    let report = run(&w, NOW);
    let goes = age > 30 * DAY;
    assert_eq!(report.work_items, u64::from(goes), "a work item deleted {age} s ago");
    assert_eq!(deleted_at(&w, item).is_none(), goes, "a work item deleted {age} s ago");
    assert_eq!(deleted_at(&w, live), Some(None), "a live work item stays");
}

/// `run_retention` counts from the app's own clock, which is what the schedule runs.
#[test]
fn run_retention_counts_from_the_apps_clock() {
    let (w, clock) = world_in(30, 90);
    let item = make(&w, Tenant::A).item.id;
    clock.set(NOW - 31 * DAY);
    remove(&w, Tenant::A, item);
    clock.set(NOW);
    let report = block_on(run_retention(&w.app)).unwrap();
    assert_eq!((report.now, report.work_items), (ts(NOW), 1));
    assert_eq!(deleted_at(&w, item), None);
}

// --- the blobs ------------------------------------------------------------------------------------------------------------------

/// A blob store that can be made to refuse a delete, over the world's own.
struct FlakyBlobs {
    inner: Arc<dyn BlobStore>,
    refuse_deletes: AtomicBool,
}

#[async_trait]
impl BlobStore for FlakyBlobs {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: Option<String>) -> Result<(), BlobError> {
        self.inner.put(key, bytes, content_type).await
    }
    async fn head(&self, key: &str) -> Result<Option<BlobHead>, BlobError> {
        self.inner.head(key).await
    }
    async fn get(&self, key: &str) -> Result<Option<Blob>, BlobError> {
        self.inner.get(key).await
    }
    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        if self.refuse_deletes.load(Ordering::SeqCst) {
            return Err(BlobError::Backend("the store is down".into()));
        }
        self.inner.delete(key).await
    }
    async fn list(&self, prefix: &str) -> Result<Vec<String>, BlobError> {
        self.inner.list(prefix).await
    }
}

/// A work item whose attachments will not be deleted is not deleted either (so no blob is left with nothing that names it): it is counted
/// in `failed`, the run says it is not complete, and the next run, with the store back, takes it. The work item beside it goes at once.
#[test]
fn a_work_item_whose_blob_will_not_go_is_left_for_the_next_run() {
    let (mut w, clock) = world_in(30, 90);
    let flaky = Arc::new(FlakyBlobs { inner: w.app.ports.blobs.clone(), refuse_deletes: AtomicBool::new(true) });
    w.app.ports.blobs = flaky.clone();
    let (stuck, free) = (make(&w, Tenant::A).item.id, make(&w, Tenant::B).item.id);
    block_on(put_attachment(
        &w.app,
        &w.principal(Actor::MemberA),
        &w.slug_a,
        w.project_a,
        stuck,
        "a.txt",
        b"x".to_vec(),
        None,
    ))
    .unwrap();
    clock.set(NOW - 40 * DAY);
    remove(&w, Tenant::A, stuck);
    remove(&w, Tenant::B, free);

    let first = run(&w, NOW);
    assert_eq!((first.work_items, first.failed, first.complete), (1, 1, false), "{first:?}");
    assert_eq!(deleted_at(&w, free), None, "the work item with no attachment went");
    assert!(deleted_at(&w, stuck).is_some(), "the one whose blob will not go stays");

    flaky.refuse_deletes.store(false, Ordering::SeqCst);
    let second = run(&w, NOW);
    assert_eq!((second.work_items, second.blobs, second.failed, second.complete), (1, 1, 0, true), "{second:?}");
    assert_eq!(deleted_at(&w, stuck), None);
}

// --- the outbox -----------------------------------------------------------------------------------------------------------------

/// A window in days, and outbox rows (each made by creating a work item), dispatched or not, made `over` seconds before the cutoff.
type Rows = (u32, Vec<(bool, i64)>);

fn rows() -> impl Strategy<Value = Rows> {
    let over = prop_oneof![3 => -2i64..=2, 1 => -300_000i64..300_000];
    (0u32..200, prop::collection::vec((any::<bool>(), over), 0..8))
}

proptest! {
    #![proptest_config(proptest_config(64))]

    /// Over any window and any outbox rows: a row goes exactly when it was dispatched and made strictly more than the window ago; a row
    /// that was not dispatched stays, however old (the sweeper still owes its message).
    #[test]
    fn prop_an_outbox_row_goes_exactly_when_it_was_dispatched_and_made_more_than_the_window_ago((days, rows) in rows()) {
        let (w, clock) = world_in(30, days);
        let cutoff = NOW - i64::from(days) * DAY;
        // the rows that were dispatched first, each swept as it is made, then the ones nobody swept
        let mut order: Vec<&(bool, i64)> = rows.iter().collect();
        order.sort_by_key(|(dispatched, _)| !dispatched);
        for (dispatched, over) in order {
            clock.set(cutoff - over);
            make(&w, Tenant::A);
            if *dispatched {
                block_on(sweep_outbox(&w.app, 100)).unwrap();
            }
        }
        let waiting = block_on(outbox_depth(&w.app)).unwrap();
        prop_assert_eq!(waiting, rows.iter().filter(|(dispatched, _)| !dispatched).count() as u64);

        clock.set(NOW);
        let report = run(&w, NOW);
        let want = rows.iter().filter(|(dispatched, over)| *dispatched && *over > 0).count() as u64;
        prop_assert_eq!(report.outbox, want, "the dispatched rows made more than {} days ago", days);
        prop_assert_eq!(report.outbox_cutoff, Some(ts(cutoff)));
        prop_assert_eq!(block_on(outbox_depth(&w.app)).unwrap(), waiting, "a row that was not dispatched is never purged");
        prop_assert_eq!(run(&w, NOW).outbox, 0, "a second run has nothing to do");
    }
}

/// Spec example, the acceptance criteria of #59 (90-day window): a dispatched row made 91 days ago is gone, while one made 89 days ago
/// and one that was never dispatched and was made 200 days ago remain.
#[test]
fn a_dispatched_row_made_91_days_ago_goes_and_one_made_89_days_ago_or_never_dispatched_stays() {
    let (w, clock) = world_in(30, 90);
    for age in [91 * DAY, 89 * DAY] {
        clock.set(NOW - age);
        make(&w, Tenant::A);
        block_on(sweep_outbox(&w.app, 100)).unwrap();
    }
    clock.set(NOW - 200 * DAY);
    make(&w, Tenant::A);
    let report = run(&w, NOW);
    assert_eq!(report.outbox, 1, "only the dispatched row of 91 days");
    assert_eq!(block_on(outbox_depth(&w.app)).unwrap(), 1, "the row nobody dispatched is still waiting");
    // the row of 89 days is the next to go, two days on
    assert_eq!(run(&w, NOW + 2 * DAY).outbox, 1);
}

// --- batches and the budget -----------------------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(proptest_config(48))]

    /// With `n` work items to delete and a budget of `batches` batches of `batch`: each run deletes the most the budget allows, says it is
    /// complete exactly when nothing is left, and a run after another takes the rest, so that the work is done in the runs the budget
    /// asks for and no work item is taken twice or missed.
    #[test]
    fn prop_a_run_does_its_budget_and_the_next_run_goes_on(n in 0usize..10, batch in 1u32..4, batches in 1u32..4) {
        let (w, clock) = world_in(30, 90);
        let ids: Vec<WorkItemId> = (0..n).map(|_| make(&w, Tenant::A).item.id).collect();
        clock.set(NOW - 40 * DAY);
        for id in &ids {
            remove(&w, Tenant::A, *id);
        }
        let limits = RetentionLimits { work_item_batch: batch, work_item_batches: batches, ..RetentionLimits::default() };
        let budget = (batch * batches) as usize;
        let mut left = n;
        let mut runs = 0;
        loop {
            let report = run_with(&w, NOW, limits);
            runs += 1;
            let done = left.min(budget);
            prop_assert_eq!(report.work_items, done as u64, "run {}", runs);
            left -= done;
            prop_assert_eq!(report.complete, left == 0, "run {}: complete exactly when nothing is left", runs);
            prop_assert_eq!(ids.iter().filter(|id| deleted_at(&w, **id).is_some()).count(), left);
            if report.complete {
                break;
            }
            prop_assert!(runs <= n, "the runs end");
        }
        prop_assert_eq!(runs, n.div_ceil(budget).max(1), "the runs the budget asks for");
    }

    /// The same for the outbox, which cannot be asked whether anything is left: a run that stops at its budget after a full batch says it
    /// may not be complete, so it is complete exactly when it stopped short of its budget.
    #[test]
    fn prop_an_outbox_run_does_its_budget_and_the_next_run_goes_on(n in 0usize..9, batch in 1u32..4, batches in 1u32..4) {
        let (w, clock) = world_in(30, 90);
        clock.set(NOW - 100 * DAY);
        for _ in 0..n {
            make(&w, Tenant::A);
            block_on(sweep_outbox(&w.app, 100)).unwrap();
        }
        let limits = RetentionLimits { outbox_batch: batch, outbox_batches: batches, ..RetentionLimits::default() };
        let budget = (batch * batches) as usize;
        let first = run_with(&w, NOW, limits);
        prop_assert_eq!(first.outbox, n.min(budget) as u64);
        prop_assert_eq!(first.complete, n < budget, "complete exactly when the run stopped short of its budget");
        let mut left = n - n.min(budget);
        while left > 0 {
            let more = run_with(&w, NOW, limits);
            prop_assert_eq!(more.outbox, left.min(budget) as u64);
            left -= left.min(budget);
        }
        prop_assert_eq!(run_with(&w, NOW, limits).outbox, 0);
    }
}

/// A budget of no batches does nothing, and says so when there is something it did not do.
#[test]
fn a_budget_of_no_batches_does_nothing_and_says_so_when_there_is_work() {
    let (w, clock) = world_in(30, 90);
    let item = make(&w, Tenant::A).item.id;
    clock.set(NOW - 40 * DAY);
    remove(&w, Tenant::A, item);
    let none = RetentionLimits { work_item_batches: 0, ..RetentionLimits::default() };
    let report = run_with(&w, NOW, none);
    assert_eq!((report.work_items, report.complete), (0, false));
    assert!(deleted_at(&w, item).is_some());
    let nothing_to_do = world_in(30, 90).0;
    assert!(run_with(&nothing_to_do, NOW, RetentionLimits { outbox_batches: 1, ..none }).complete);
}

// --- the windows ----------------------------------------------------------------------------------------------------------------

fn lookup(work: Option<&str>, outbox: Option<&str>) -> impl Fn(&str) -> Option<String> {
    let (work, outbox) = (work.map(str::to_string), outbox.map(str::to_string));
    move |name| match name {
        HARD_DELETE_AFTER_DAYS_VAR => work.clone(),
        OUTBOX_RETENTION_DAYS_VAR => outbox.clone(),
        _ => None,
    }
}

/// An unset window and an empty one are the defaults, 30 and 90 days (deviation D-001 and Q-0046.2).
#[rstest]
fn an_unset_or_empty_window_is_the_default(
    #[values(None, Some(""))] work: Option<&str>,
    #[values(None, Some(""))] outbox: Option<&str>,
) {
    let cfg = CoreConfig::default().with_retention_from(lookup(work, outbox)).unwrap();
    assert_eq!((cfg.hard_delete_after_days, cfg.outbox_retention_days), (30, 90));
}

/// What is not a whole number of days, 0 or more, is refused with the variable's name and the value, in either variable, where `env_num`
/// reads it as the default: a window read as other than the person meant deletes what they meant to keep. (Plane refuses a negative
/// `HARD_DELETE_AFTER_DAYS` for the same reason.)
#[rstest]
fn a_window_that_is_not_a_number_of_days_is_refused(
    #[values("-1", "-0", "1.5", "1e3", "abc", " 7", "7 ", "0x10", "٣", "+", "4294967296", "99999999999999999999")] bad: &str,
    #[values(HARD_DELETE_AFTER_DAYS_VAR, OUTBOX_RETENTION_DAYS_VAR)] var: &str,
) {
    let look = if var == HARD_DELETE_AFTER_DAYS_VAR { lookup(Some(bad), None) } else { lookup(None, Some(bad)) };
    let err = CoreConfig::default()
        .with_retention_from(look)
        .expect_err("a window that is not a whole number of days is refused, not read as the default");
    assert_eq!((err.name, err.got.as_str()), (var, bad));
    assert!(err.to_string().contains(var) && err.to_string().contains(&format!("{bad:?}")), "{err}");
}

proptest! {
    #![proptest_config(proptest_config(256))]

    /// Any number of days a person can write down is read as that number, for each window on its own, and `0` is a window.
    #[test]
    fn prop_a_whole_number_of_days_is_read_as_itself(work in any::<u32>(), outbox in any::<u32>(), zeros in 0usize..4) {
        let pad = "0".repeat(zeros);
        let cfg = CoreConfig::default()
            .with_retention_from(lookup(Some(&format!("{pad}{work}")), Some(&format!("{pad}{outbox}"))))
            .unwrap();
        prop_assert_eq!((cfg.hard_delete_after_days, cfg.outbox_retention_days), (work, outbox));
    }
}

/// A window so long that it reaches before the earliest time there is has no cutoff, and deletes nothing (and does not panic).
#[test]
fn a_window_longer_than_time_has_no_cutoff_and_deletes_nothing() {
    let (w, clock) = world_in(u32::MAX, u32::MAX);
    let item = make(&w, Tenant::A).item.id;
    clock.set(1);
    remove(&w, Tenant::A, item);
    let report = run(&w, NOW);
    assert_eq!((report.work_item_cutoff, report.outbox_cutoff, report.work_items, report.outbox), (None, None, 0, 0));
    assert!(report.complete);
    assert!(deleted_at(&w, item).is_some());
}
