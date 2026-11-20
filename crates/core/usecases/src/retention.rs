//! The daily hard delete (issue #59, F-D10, deviation D-001): what `run_retention` deletes for good, and how the windows are set.
//!
//! Two things go. A work item that was soft-deleted more than `hard_delete_after_days` ago goes with its history (D-043) and the blobs
//! under it, once every work item under it has been deleted that long too; the project's counter stays, so its number is not handed
//! out again (ADR-0043 B1). An outbox row that was dispatched more than `outbox_retention_days` ago goes; one that was not dispatched
//! never does (ADR-0046, Q-0046.2). The retained outbox is the P0 audit trail (ADR-0048), so its window is how long that is kept.
//!
//! The job runs for every tenant at once, so it takes no `Scope` and works through the untenanted `Database` calls that the app's
//! `SystemScope` allows. It runs in bounded batches inside the cron's wall time (a run that reaches its budget says so and leaves the
//! rest to the next day), and it is idempotent: a run that is interrupted, or run twice, leaves what a clean run would.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use thrust_contracts::chrono::TimeDelta;
use thrust_contracts::*;
use thrust_ports::*;

use crate::{blob_prefix, App, CoreConfig, CoreError};

/// The environment variable that sets [`CoreConfig::hard_delete_after_days`].
pub const HARD_DELETE_AFTER_DAYS_VAR: &str = "THRUST_HARD_DELETE_AFTER_DAYS";
/// The environment variable that sets [`CoreConfig::outbox_retention_days`].
pub const OUTBOX_RETENTION_DAYS_VAR: &str = "THRUST_OUTBOX_RETENTION_DAYS";

/// A retention window that is not a whole number of days. Plane refuses a negative `HARD_DELETE_AFTER_DAYS` for the reason this does: a
/// window read as something else than the person meant deletes rows that were meant to stay, and nothing brings them back. So the
/// deployment does not start, where a value that does not parse is elsewhere read as the default.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("{name} is {got:?}, which is not a number of days: it must be a whole number, 0 or more")]
pub struct RetentionConfigError {
    pub name: &'static str,
    pub got: String,
}

impl CoreConfig {
    /// The retention windows as the environment (or any lookup of a variable by name) sets them. A variable that is not there, or is
    /// empty, leaves the window as it is; one that is anything but a whole number of days, 0 or more, is an error naming it.
    pub fn with_retention_from(
        mut self,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, RetentionConfigError> {
        self.hard_delete_after_days = days(&lookup, HARD_DELETE_AFTER_DAYS_VAR, self.hard_delete_after_days)?;
        self.outbox_retention_days = days(&lookup, OUTBOX_RETENTION_DAYS_VAR, self.outbox_retention_days)?;
        Ok(self)
    }
}

fn days(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: u32,
) -> Result<u32, RetentionConfigError> {
    match lookup(name) {
        None => Ok(default),
        Some(v) if v.is_empty() => Ok(default),
        Some(v) => match v.parse::<u32>() {
            Ok(n) => Ok(n),
            // MUTANT (control run only): a window that does not parse is the default, as `env_num` reads every other number. The test
            // that holds the deployment to refuse a mistake must catch it.
            #[cfg(feature = "mutant_retention_days_lenient")]
            Err(_) => Ok(default),
            #[cfg(not(feature = "mutant_retention_days_lenient"))]
            Err(_) => Err(RetentionConfigError { name, got: v }),
        },
    }
}

/// How much one run does. The defaults are what a cron's wall time allows with room to spare; a test gives small ones to see the
/// batches and the budget at work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionLimits {
    /// Work items taken at once: one transaction, and the blobs of each listed and deleted before it.
    pub work_item_batch: u32,
    /// Batches of work items a run does at most.
    pub work_item_batches: u32,
    /// Outbox rows deleted at once.
    pub outbox_batch: u32,
    /// Batches of outbox rows a run does at most.
    pub outbox_batches: u32,
}

impl Default for RetentionLimits {
    fn default() -> Self {
        Self { work_item_batch: 100, work_item_batches: 50, outbox_batch: 1000, outbox_batches: 100 }
    }
}

/// What a run did, and the windows it did it with (`POST /internal/retention` answers with it, and the schedule logs it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionReport {
    /// The time the run counted from: the app's clock, or the one a test route gave.
    pub now: Timestamp,
    pub hard_delete_after_days: u32,
    pub outbox_retention_days: u32,
    /// `now` less the window: a work item deleted strictly before it is old. `None` when the window reaches back past the earliest
    /// time there is, so that nothing is old enough.
    pub work_item_cutoff: Option<Timestamp>,
    pub outbox_cutoff: Option<Timestamp>,
    /// Work items deleted for good, with their history.
    pub work_items: u64,
    /// Blobs deleted: the attachments of those work items.
    pub blobs: u64,
    /// Dispatched outbox rows deleted.
    pub outbox: u64,
    /// Work items that are left for the next run because a blob under them would not go (the work item is kept, so no blob is left
    /// with nothing that names it).
    pub failed: u64,
    /// `false` when the run stopped at its budget, or left a work item for a blob that would not go: there may be more to delete.
    pub complete: bool,
}

/// `now` less `days`, or `None` when that is before the earliest time there is.
#[cfg(not(feature = "mutant_retention_ignores_cutoff"))]
fn cutoff(now: Timestamp, days: u32) -> Option<Timestamp> {
    TimeDelta::try_days(i64::from(days)).and_then(|window| now.checked_sub_signed(window))
}

// MUTANT (control run only): the window is not counted back from `now`, so a row goes as soon as it is older than `now`, whatever the
// window says. The tests that keep a row inside its window must catch it.
#[cfg(feature = "mutant_retention_ignores_cutoff")]
fn cutoff(now: Timestamp, _days: u32) -> Option<Timestamp> {
    Some(now)
}

/// How many times a work item's prefix is listed and cleared before the work item is left for the next run: R2 lists at most a thousand
/// keys a call, so a work item with more is cleared a call at a time.
const BLOB_ROUNDS: u32 = 20;

/// The daily hard delete, counted from the app's clock and in the default batches. See [`run_retention_with`].
pub async fn run_retention(app: &App) -> Result<RetentionReport, CoreError> {
    run_retention_at(app, app.ports.clock.now()).await
}

/// The daily hard delete counted from `now` and in the default batches: what `POST /internal/retention` runs, so that a contract test
/// can stand a month ahead of a work item it deleted a moment ago. See [`run_retention_with`].
pub async fn run_retention_at(app: &App, now: Timestamp) -> Result<RetentionReport, CoreError> {
    run_retention_with(app, now, RetentionLimits::default()).await
}

/// The daily hard delete (issue #59), counted from `now` and in batches of the sizes `limits` give. An error from the database ends the
/// run, and what it had done stays done. A blob that cannot be deleted does not: that work item is left (and counted in `failed`) for the
/// next run, and the others go on.
pub async fn run_retention_with(
    app: &App,
    now: Timestamp,
    limits: RetentionLimits,
) -> Result<RetentionReport, CoreError> {
    let work_item_cutoff = cutoff(now, app.cfg.hard_delete_after_days);
    let outbox_cutoff = cutoff(now, app.cfg.outbox_retention_days);
    let mut report = RetentionReport {
        now,
        hard_delete_after_days: app.cfg.hard_delete_after_days,
        outbox_retention_days: app.cfg.outbox_retention_days,
        work_item_cutoff,
        outbox_cutoff,
        work_items: 0,
        blobs: 0,
        outbox: 0,
        failed: 0,
        complete: true,
    };
    if let Some(cutoff) = work_item_cutoff {
        take_work_items(app, cutoff, limits, &mut report).await?;
    }
    if let Some(cutoff) = outbox_cutoff {
        take_outbox(app, cutoff, limits, &mut report).await?;
    }
    // a work item left for the next run is work there is still to do
    if report.failed > 0 {
        report.complete = false;
    }
    Ok(report)
}

/// Walk the purgeable work items by their key, a batch at a time, and delete each batch for good: the blobs first, then the rows. The
/// walk moves on from the last row of each page, so a work item that is left (a blob would not go) is not asked for again this run.
async fn take_work_items(
    app: &App,
    cutoff: Timestamp,
    limits: RetentionLimits,
    report: &mut RetentionReport,
) -> Result<(), CoreError> {
    let db = &app.ports.db;
    let batch = limits.work_item_batch.max(1);
    let mut after = None;
    let mut batches = 0;
    loop {
        if batches == limits.work_item_batches {
            // the budget is spent: is there anything past the last page?
            if !db.purgeable_work_items(&app.system, cutoff, after, 1).await?.is_empty() {
                report.complete = false;
            }
            return Ok(());
        }
        let page = db.purgeable_work_items(&app.system, cutoff, after, batch).await?;
        let Some(last) = page.last() else {
            return Ok(());
        };
        after = Some((last.deleted_at, last.id));
        batches += 1;
        let mut ready = Vec::with_capacity(page.len());
        for candidate in &page {
            match delete_blobs(app, candidate, &mut report.blobs).await {
                Ok(()) => ready.push(candidate.id),
                Err(_) => report.failed += 1,
            }
        }
        report.work_items += db.purge_work_items(&app.system, &ready, cutoff).await?;
        if page.len() < batch as usize {
            return Ok(());
        }
    }
}

/// Delete every blob under a work item's prefix. The count goes up as each one goes, so a work item that fails half way still reports
/// what was deleted.
async fn delete_blobs(app: &App, candidate: &PurgeCandidate, deleted: &mut u64) -> Result<(), BlobError> {
    // MUTANT (control run only): a work item goes and its attachments stay behind, with nothing that names them. The test that lists a
    // purged work item's prefix afterwards must catch it.
    #[cfg(feature = "mutant_retention_keeps_blobs")]
    {
        let _ = (app, candidate, deleted);
        Ok(())
    }
    #[cfg(not(feature = "mutant_retention_keeps_blobs"))]
    {
        let prefix = blob_prefix(candidate.workspace_id, candidate.project_id, candidate.id);
        for _ in 0..BLOB_ROUNDS {
            let keys = app.ports.blobs.list(&prefix).await?;
            if keys.is_empty() {
                return Ok(());
            }
            for key in &keys {
                app.ports.blobs.delete(key).await?;
                *deleted += 1;
            }
        }
        if app.ports.blobs.list(&prefix).await?.is_empty() {
            Ok(())
        } else {
            Err(BlobError::Backend(format!("{prefix} still has blobs after {BLOB_ROUNDS} rounds")))
        }
    }
}

/// Delete the dispatched outbox rows made before the cutoff, the oldest first, a batch at a time. The table cannot be asked whether
/// anything is left, so a run that stops at its budget after a full batch says it may not be complete.
async fn take_outbox(
    app: &App,
    cutoff: Timestamp,
    limits: RetentionLimits,
    report: &mut RetentionReport,
) -> Result<(), CoreError> {
    let batch = limits.outbox_batch.max(1);
    let mut batches = 0;
    loop {
        if batches == limits.outbox_batches {
            report.complete = false;
            return Ok(());
        }
        let n = app.ports.db.purge_outbox(&app.system, cutoff, batch).await?;
        report.outbox += n;
        batches += 1;
        if n < u64::from(batch) {
            return Ok(());
        }
    }
}

/// Test support behind `GET /internal/work-items/{id}`: the work item's `deleted_at` whatever its state. `None`: no row has the id (it
/// never did, or it was deleted for good); `Some(None)`: a live one; `Some(Some(at))`: a deleted one, and when.
pub async fn work_item_deleted_at(app: &App, id: WorkItemId) -> Result<Option<Option<Timestamp>>, CoreError> {
    Ok(app.ports.db.work_item_deleted_at(&app.system, id).await?)
}
