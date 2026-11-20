//! The schedules (issue #59, ADR-0072) through the use-cases: which job a trigger runs, what running it does, and the time the native
//! server's timer wakes at. The Worker's `scheduled` handler and the server's timers call what is tested here (`run_cron`,
//! `run_job` and `next_utc_midnight_after`), and the contract suite's `cron` pass shows the handler is wired to it.
//!
//! The rules, written out:
//!
//! * The schedules are the sweep every minute and the hard delete at 00:00 UTC, each job has one, and no two share a cron or an id.
//! * A trigger runs the job of the schedule whose cron it is, written the same. Any other string, however close (a space more, a field
//!   fewer, another number), runs no job.
//! * Running a trigger does its job and no other: the hard delete's cron deletes what is old and publishes nothing, the sweep's
//!   publishes and deletes nothing, and a cron that is not in the table does neither.
//! * `next_utc_midnight_after(t)` is the first 00:00:00 UTC strictly after `t`, which is when `0 0 * * *` next fires.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_contracts::chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

const BASE: i64 = 1_760_000_000;
const DAY: i64 = 86_400;

fn ts(secs: i64) -> Timestamp {
    DateTime::from_timestamp(BASE + secs, 0).unwrap()
}

/// A clock the test moves, in seconds from `BASE`.
struct Clockwork(AtomicI64);
impl Clock for Clockwork {
    fn now(&self) -> Timestamp {
        ts(self.0.load(Ordering::SeqCst))
    }
}

// --- the table ------------------------------------------------------------------------------------------------------------------

/// The schedules ADR-0072 and issue #59 name: the outbox sweep every minute, and the hard delete once a day at 00:00 UTC (an example
/// the ADR and the issue give, not a generated case).
#[test]
fn the_schedules_are_the_ones_the_adr_and_the_issue_name() {
    let got: Vec<_> = SCHEDULES.iter().map(|s| (s.id, s.cron, s.job)).collect();
    assert_eq!(
        got,
        [("outbox-sweep", "* * * * *", Job::OutboxSweep), ("hard-delete", "0 0 * * *", Job::HardDelete)],
        "the table of schedules is not the one ADR-0072 and issue #59 name"
    );
}

#[rstest]
fn each_job_has_one_schedule(#[values(Job::OutboxSweep, Job::HardDelete)] job: Job) {
    let n = SCHEDULES.iter().filter(|s| s.job == job).count();
    assert_eq!(
        n, 1,
        "{job:?} has {n} schedules, and a job that runs from two triggers runs twice a time they coincide"
    );
}

#[rstest]
fn no_two_schedules_share_a_cron_or_an_id(#[values(0, 1)] i: usize, #[values(0, 1)] j: usize) {
    if i != j {
        assert_ne!(SCHEDULES[i].cron, SCHEDULES[j].cron, "two schedules with the same trigger");
        assert_ne!(SCHEDULES[i].id, SCHEDULES[j].id, "two schedules with the same id");
    }
}

// --- which job a trigger runs ---------------------------------------------------------------------------------------------------

#[rstest]
fn a_trigger_runs_the_job_of_its_schedule(#[values(0, 1)] n: usize) {
    let s = SCHEDULES[n];
    assert_eq!(job_for_cron(s.cron), Some(s.job), "the trigger '{}' of schedule {}", s.cron, s.id);
}

/// Text that is not a cron of the table: near misses of each (a space more or less, a field fewer or more, other numbers, other case),
/// the empty string, and anything at all.
fn not_a_cron_of_ours() -> impl Strategy<Value = String> {
    let crons: Vec<&'static str> = SCHEDULES.iter().map(|s| s.cron).collect();
    let near = prop::sample::select(crons).prop_flat_map(|c| {
        prop_oneof![
            Just(format!(" {c}")),
            Just(format!("{c} ")),
            Just(c.replace(' ', "  ")),
            Just(c.replace('*', "0")),
            Just(c.replace('0', "1")),
            Just(c.split(' ').take(4).collect::<Vec<_>>().join(" ")),
            Just(format!("{c} *")),
            Just(c.replace(' ', "\t")),
            Just(format!("{c}\n")),
        ]
    });
    prop_oneof![1 => Just(String::new()), 4 => near, 2 => any::<String>(), 3 => "[0-9*/, ]{0,14}"]
        .prop_filter("a cron of the table", |c| SCHEDULES.iter().all(|s| s.cron != c))
}

proptest! {
    #![proptest_config(proptest_config(256))]

    /// Any text that is not exactly a cron of the table runs no job: a trigger nobody listed is a no-op, never a second sweep.
    #[test]
    fn prop_a_string_that_is_not_a_cron_of_the_table_runs_no_job(cron in not_a_cron_of_ours()) {
        prop_assert_eq!(job_for_cron(&cron), None);
    }
}

// --- running a trigger ----------------------------------------------------------------------------------------------------------

/// The time the job runs: far enough from the start for a work item deleted at the start to be well past the default window.
const NOW: i64 = 1_000 * DAY;

/// A world with the default windows (30 days for a work item), a clock the test moves and a recording queue. The fast path is off, so
/// the outbox rows of what it does wait for the sweeper.
fn world() -> (World, Arc<Clockwork>) {
    let clock = Arc::new(Clockwork(AtomicI64::new(0)));
    let cfg = CoreConfig { outbox_fastpath: false, ..CoreConfig::default() };
    let w = world_built(cfg, Arc::new(RecordingQueue::default()), Arc::new(NoBackground), clock.clone());
    (w, clock)
}

fn work_item_exists(w: &World, id: WorkItemId) -> bool {
    block_on(work_item_deleted_at(&w.app, id)).unwrap().is_some()
}

/// A world that holds a work item deleted a thousand days ago (so, past the window) and the outbox rows of making and deleting it, not
/// yet published, with the clock at the time a job would run. Returns the work item and how many rows are waiting.
fn a_world_with_old_work_to_do() -> (World, WorkItemId, u64) {
    let (w, clock) = world();
    let access = block_on(v1_issue_writer(&w.app, &w.principal(Actor::MemberA), &w.slug_a, w.project_a)).unwrap();
    let item =
        block_on(create_issue(&w.app, &access, IssueDraft { name: "item".into(), ..IssueDraft::default() })).unwrap();
    block_on(delete_issue(&w.app, &access, item.item.id)).unwrap();
    let pending = block_on(outbox_depth(&w.app)).unwrap();
    assert!(pending > 0, "the setup left no outbox row to publish, so no case could tell a sweep from none");
    clock.0.store(NOW, Ordering::SeqCst);
    (w, item.item.id, pending)
}

/// Each schedule's trigger does its job and no other: the hard delete's deletes the old work item and publishes nothing, and the
/// sweep's publishes the waiting rows and deletes nothing.
#[rstest]
fn a_trigger_does_its_job_and_no_other(#[values(0, 1)] n: usize) {
    let s = SCHEDULES[n];
    let (w, id, pending) = a_world_with_old_work_to_do();
    let ran = block_on(run_cron(&w.app, s.cron)).expect("a cron of the table runs a job").expect("the job failed");
    let left = block_on(outbox_depth(&w.app)).unwrap();
    match (s.job, ran) {
        (Job::HardDelete, JobReport::HardDelete(r)) => {
            assert!(!work_item_exists(&w, id), "the hard delete left the work item deleted a thousand days ago");
            assert_eq!(left, pending, "the hard delete published outbox rows, which is the sweep's job");
            assert_eq!((r.work_items, r.complete), (1, true), "what the hard delete says it did");
        }
        (Job::OutboxSweep, JobReport::OutboxSweep(r)) => {
            assert!(work_item_exists(&w, id), "the sweep deleted a work item, which is the hard delete's job");
            assert_eq!(left, 0, "the sweep left rows unpublished");
            assert_eq!(
                (r.found as u64, r.published as u64, r.failed),
                (pending, pending, 0),
                "what the sweep says it did"
            );
        }
        (job, got) => panic!("the schedule {} is {job:?}, and its trigger ran {got:?}", s.id),
    }
}

proptest! {
    #![proptest_config(proptest_config(32))]

    /// A trigger that is not in the table does nothing at all: the work item stays and the rows stay unpublished.
    #[test]
    fn prop_a_cron_that_is_not_in_the_table_changes_nothing(cron in not_a_cron_of_ours()) {
        let (w, id, pending) = a_world_with_old_work_to_do();
        prop_assert!(block_on(run_cron(&w.app, &cron)).is_none(), "'{}' ran a job", cron);
        prop_assert!(work_item_exists(&w, id), "'{}' deleted a work item", cron);
        prop_assert_eq!(block_on(outbox_depth(&w.app)).unwrap(), pending, "'{}' published outbox rows", cron);
    }
}

// --- the native server's timer ---------------------------------------------------------------------------------------------------

/// Instants from before 1970 to after 2100, with the exact midnight and the last second before one weighted in: those are where a
/// timer that wakes early, or is asked at the instant it fired, goes wrong.
fn instants() -> impl Strategy<Value = Timestamp> {
    let second_of_day = prop_oneof![2 => Just(0i64), 2 => Just(DAY - 1), 4 => 0..DAY];
    let nanos = prop_oneof![2 => Just(0u32), 1 => 0..1_000_000_000u32];
    (-50_000i64..60_000, second_of_day, nanos)
        .prop_map(|(day, secs, nanos)| DateTime::from_timestamp(day * DAY + secs, nanos).unwrap())
}

proptest! {
    #![proptest_config(proptest_config(256))]

    /// The next midnight is the first 00:00:00 UTC strictly after the instant: after it, at midnight to the nanosecond, and no more than
    /// a day on (so that none was skipped). At a midnight itself it is the next day's: a timer asked at the instant it fired waits.
    #[test]
    fn prop_the_next_midnight_is_the_first_one_strictly_after_the_instant(t in instants()) {
        let next = next_utc_midnight_after(t).unwrap();
        prop_assert!(next > t, "{next} is not after {t}");
        prop_assert_eq!(next.time(), NaiveTime::MIN, "{} is not a midnight", next);
        prop_assert!(next - t <= TimeDelta::days(1), "{next} is more than a day after {t}: a midnight was skipped");
    }

    /// At the end of the range a `Timestamp` has, the last day has no midnight after it, and no earlier day is without one.
    #[test]
    fn prop_only_the_last_day_there_is_has_no_next_midnight(back in 0i64..3 * DAY, nanos in 0u32..1_000_000_000) {
        let t = DateTime::<Utc>::MAX_UTC - TimeDelta::seconds(back) - TimeDelta::nanoseconds(i64::from(nanos));
        prop_assert_eq!(next_utc_midnight_after(t).is_none(), t.date_naive() == NaiveDate::MAX, "{}", t);
    }
}

/// The server's timer calls `next_utc_midnight_after` and the Worker has a trigger, both for the hard delete: the table says the hard
/// delete is the midnight cron. A schedule moved to another time must move the timer with it, and this fails until it does.
#[test]
fn the_hard_delete_is_the_schedule_the_servers_midnight_timer_stands_for() {
    let hard_delete = SCHEDULES.iter().find(|s| s.job == Job::HardDelete).unwrap();
    assert_eq!(hard_delete.cron, "0 0 * * *", "the server's timer wakes at 00:00 UTC; move it with the cron");
}
