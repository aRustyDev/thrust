//! The migration ledger (issue #195): which migration files a database has had applied, and so which ones a build may still apply.
//!
//! Until the ledger, `migrate` ran every file on every start, and the files were written to be run again (`IF NOT EXISTS`, a dropped and
//! re-added constraint). That is safe for one version of the code over its own database. It is not safe once more than one version runs
//! over the same data (#195: a release candidate, or an older release kept for comparison, next to production): an older build running
//! its files again would put an older definition back over a newer one. So a database now records each file it has applied, with the
//! SHA-256 of the SQL, and a build applies only what is missing, and only when it is the newest.
//!
//! This module is the arithmetic, and has no database in it: [`plan`] compares the files a build carries with the ones a database
//! recorded. `migrate` (lib.rs) reads the ledger, asks [`plan`], and applies.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

/// The table in which a database records the migration files it has applied: one row per file, with the SHA-256 of the SQL that was
/// applied. Unqualified, so it lives in the first schema of the connection's `search_path`, as every table of the migrations does: a
/// database whose schemas are per target has one ledger per target. The adapter's `reset` leaves it alone: it is not data.
pub const LEDGER_TABLE: &str = "thrust_schema_migrations";

/// One migration file, as a build carries it or as a database recorded it: its file name (`0001_init.sql`) and the SHA-256 of its SQL,
/// in lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    pub file: String,
    pub sha256: String,
}

/// The SHA-256 of a migration's SQL, in lowercase hex: what the ledger records and compares.
pub fn sha256_hex(sql: &str) -> String {
    Sha256::digest(sql.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// A contract migration is named `NNNN_contract_<what>.sql`: four digits, `_contract_`, a name that is not empty, `.sql`. It is one that
/// takes away what an older version of the code still uses (a column dropped, a constraint tightened), so it may run only when no
/// older version runs on the same data. Every other migration may only add (scripts/check-migration-lint.sh).
pub fn is_contract(file: &str) -> bool {
    let (Some(number), Some(rest)) = (file.get(..4), file.get(4..)) else {
        return false;
    };
    number.bytes().all(|b| b.is_ascii_digit())
        && rest.strip_prefix("_contract_").is_some_and(|what| what.len() > ".sql".len() && what.ends_with(".sql"))
}

/// What a build may do with a database's schema, given the files each has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// The build has every file the database recorded. Apply `pending`, in this order; an empty `pending` is a database that is up to
    /// date. `contract` is the pending files that are contract migrations (also in `pending`), so that a caller can refuse them.
    Apply { pending: Vec<String>, contract: Vec<String> },
    /// The database recorded every file this build has, and more (`unknown`): the build is older than the schema. It applies nothing,
    /// and that is not an error: an older release next to a newer one runs on the newer schema (#195, the expand rule).
    Older { unknown: Vec<String> },
    /// REFUSED. Each side has files the other does not: the build is from a branch that did not have what was applied since.
    Diverged { pending: Vec<String>, unknown: Vec<String> },
    /// REFUSED. A file the database does not have is older than the newest one it does: it would be applied out of the order the
    /// files were written in (two branches that took the same number, or a file added under an old one).
    OutOfOrder { pending: Vec<String>, newest_applied: String },
    /// REFUSED. A file the database recorded is not what this build carries under that name: a migration was edited after it ran.
    Changed { files: Vec<String> },
}

impl Plan {
    /// The plan as one JSON object, for `migrate --plan` and the deploy pipeline: `outcome` is one of `baseline` (the database has no
    /// ledger yet: every file runs once and is recorded), `current`, `apply`, `older`, `diverged`, `out_of_order`, `changed`; the lists
    /// are always there, empty when they do not apply.
    pub fn to_json(&self, ledger_present: bool) -> serde_json::Value {
        let empty: Vec<String> = Vec::new();
        let (outcome, pending, contract, unknown, changed) = match self {
            Plan::Apply { pending, contract } => {
                let outcome = if !ledger_present {
                    "baseline"
                } else if pending.is_empty() {
                    "current"
                } else {
                    "apply"
                };
                (outcome, pending, contract, &empty, &empty)
            }
            Plan::Older { unknown } => ("older", &empty, &empty, unknown, &empty),
            Plan::Diverged { pending, unknown } => ("diverged", pending, &empty, unknown, &empty),
            Plan::OutOfOrder { pending, .. } => ("out_of_order", pending, &empty, &empty, &empty),
            Plan::Changed { files } => ("changed", &empty, &empty, &empty, files),
        };
        let mut out = serde_json::json!({
            "outcome": outcome,
            "pending": pending,
            "contract": contract,
            "unknown": unknown,
            "changed": changed,
        });
        if let (Plan::OutOfOrder { newest_applied, .. }, Some(map)) = (self, out.as_object_mut()) {
            map.insert("newest_applied".into(), serde_json::Value::String(newest_applied.clone()));
        }
        out
    }
}

/// Compare the files a build carries (`known`) with the ones a database recorded (`ledger`). Names are compared as text, which is
/// their order because every name starts with a four-digit number.
///
/// In this order: a recorded file whose checksum differs is [`Plan::Changed`]; then, with `pending` the known files the ledger does not
/// have and `unknown` the recorded files the build does not know, both empty or only `pending` is [`Plan::Apply`] (unless a pending
/// file sorts before the newest recorded one: [`Plan::OutOfOrder`]); only `unknown` is [`Plan::Older`]; both is [`Plan::Diverged`].
pub fn plan(known: &[Entry], ledger: &[Entry]) -> Plan {
    let recorded: BTreeMap<&str, &str> = ledger.iter().map(|e| (e.file.as_str(), e.sha256.as_str())).collect();
    let carried: BTreeMap<&str, &str> = known.iter().map(|e| (e.file.as_str(), e.sha256.as_str())).collect();

    let changed: Vec<String> = carried
        .iter()
        .filter(|(file, sha)| recorded.get(*file).is_some_and(|was| was != *sha))
        .map(|(file, _)| (*file).to_string())
        .collect();
    if !changed.is_empty() {
        return Plan::Changed { files: changed };
    }

    let pending: Vec<String> =
        carried.keys().filter(|file| !recorded.contains_key(*file)).map(|file| (*file).to_string()).collect();
    let unknown: Vec<String> =
        recorded.keys().filter(|file| !carried.contains_key(*file)).map(|file| (*file).to_string()).collect();

    match (pending.is_empty(), unknown.is_empty()) {
        (_, true) => {
            if let (Some(first), Some(newest)) = (pending.first(), recorded.keys().next_back()) {
                // never equal: a pending file is by definition one the ledger does not have
                if first.as_str() < *newest {
                    return Plan::OutOfOrder { pending, newest_applied: (*newest).to_string() };
                }
            }
            let contract = pending.iter().filter(|file| is_contract(file)).cloned().collect();
            Plan::Apply { pending, contract }
        }
        (true, false) => Plan::Older { unknown },
        (false, false) => Plan::Diverged { pending, unknown },
    }
}
