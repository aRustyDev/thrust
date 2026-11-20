//! What each scheduled trigger runs (ADR-0072, issue #59): the one table of schedules, the job a cron string names, and the one place that
//! runs a job, so that the Worker's `scheduled` handler and the native server's timers cannot drift apart.
//!
//! The Worker has a trigger per schedule in its wrangler files (`triggers.crons`), and `scheduled` hands the trigger's own cron string to
//! [`run_cron`]. A string that is not in [`SCHEDULES`] runs nothing, so a trigger added to a wrangler file and not to this table is a
//! logged no-op and not a second sweep; `.github/scripts/tests/deploy_parity.py` holds the files to this table, reading the literals
//! below, so those are written one to a line in the one shape its pattern reads.
//!
//! The native server has no cron. It runs the sweep on an interval (`THRUST_SWEEP_INTERVAL_SECS`) and the hard delete at each
//! [`next_utc_midnight_after`], which is when `0 0 * * *` fires: the table says the hard delete is that cron, and a test holds it there.

use thrust_contracts::chrono::NaiveTime;
use thrust_contracts::*;

use crate::{run_retention, sweep_outbox, App, CoreError, RetentionReport, SweepReport};

/// Outbox rows the sweeper publishes in one run (ADR-0046).
pub const SWEEP_BATCH: u32 = 100;

/// A job a schedule runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// Publish the outbox rows the fast path did not (ADR-0046).
    OutboxSweep,
    /// Delete for good what has been deleted for long enough, and the outbox rows that are old enough (issue #59, F-D10, D-001).
    HardDelete,
}

/// One schedule: a trigger, and the job it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule {
    /// What a log line and a test call it.
    pub id: &'static str,
    /// The trigger, in the five fields a Cloudflare cron trigger takes, in UTC.
    pub cron: &'static str,
    pub job: Job,
}

/// Every schedule there is. Every wrangler file's `triggers.crons` is exactly the crons below (`deploy_parity.py`).
pub const SCHEDULES: [Schedule; 2] = [
    Schedule { id: "outbox-sweep", cron: "* * * * *", job: Job::OutboxSweep },
    Schedule { id: "hard-delete", cron: "0 0 * * *", job: Job::HardDelete },
];

/// The job the trigger `cron` runs: the schedule whose cron it is, written the same, or none.
#[cfg(not(feature = "mutant_cron_ignored"))]
pub fn job_for_cron(cron: &str) -> Option<Job> {
    SCHEDULES.iter().find(|s| s.cron == cron).map(|s| s.job)
}

// MUTANT (control run only): the trigger is not read, so whichever fires runs the first schedule's job. The tests that say which cron
// runs which job must catch it.
#[cfg(feature = "mutant_cron_ignored")]
pub fn job_for_cron(_cron: &str) -> Option<Job> {
    SCHEDULES.first().map(|s| s.job)
}

/// What a job did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobReport {
    OutboxSweep(SweepReport),
    HardDelete(RetentionReport),
}

/// Run `job` once, counting from the app's clock. An error from the database ends the run, and what it had done stays done.
pub async fn run_job(app: &App, job: Job) -> Result<JobReport, CoreError> {
    match job {
        Job::OutboxSweep => sweep_outbox(app, SWEEP_BATCH).await.map(JobReport::OutboxSweep),
        Job::HardDelete => run_retention(app).await.map(JobReport::HardDelete),
    }
}

/// What a firing of the trigger `cron` does: its job is run, or, when no schedule has that cron, nothing is and `None` says so.
pub async fn run_cron(app: &App, cron: &str) -> Option<Result<JobReport, CoreError>> {
    let job = job_for_cron(cron)?;
    Some(run_job(app, job).await)
}

/// The first 00:00:00 UTC strictly after `t`: the time `0 0 * * *` next fires. Strictly, so that a timer that wakes a moment early, or
/// is asked again at the instant it fired, waits for the next day and cannot run a day's job twice. `None` only on the last day of the
/// range a [`Timestamp`] has, which has no midnight after it.
pub fn next_utc_midnight_after(t: Timestamp) -> Option<Timestamp> {
    t.date_naive().succ_opt().map(|day| day.and_time(NaiveTime::MIN).and_utc())
}
