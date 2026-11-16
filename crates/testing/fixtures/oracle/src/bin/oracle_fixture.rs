//! Prints the oracle's seed as the JSON body of `POST /internal/seed` (or, with `--manifest`, the manifest of what is in it).
//!
//! ```text
//! oracle-fixture [--now 2026-11-09T12:00:00Z] [--manifest]
//! ```
//!
//! `--now` is the instant the fixture is made at (the dates it gives are relative to its day): without it, the system clock.
//! `scripts/oracle-v1-reset.sh` is the caller; it empties the target and loads this.

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use thrust_contracts::chrono::DateTime;
use thrust_contracts::Timestamp;
use thrust_oracle_fixture::oracle_seed;

fn now_from_clock() -> Option<Timestamp> {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    DateTime::from_timestamp(i64::try_from(since.as_secs()).ok()?, since.subsec_nanos())
}

fn main() -> ExitCode {
    let mut now = None;
    let mut manifest = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest" => manifest = true,
            "--now" => match args.next().as_deref().map(DateTime::parse_from_rfc3339) {
                Some(Ok(t)) => now = Some(t.to_utc()),
                _ => {
                    eprintln!("oracle-fixture: --now takes an RFC 3339 time, such as 2026-11-09T12:00:00Z");
                    return ExitCode::from(2);
                }
            },
            other => {
                eprintln!(
                    "oracle-fixture: unknown argument {other:?} (usage: oracle-fixture [--now TIME] [--manifest])"
                );
                return ExitCode::from(2);
            }
        }
    }
    let Some(now) = now.or_else(now_from_clock) else {
        eprintln!("oracle-fixture: the system clock is not a time; give --now");
        return ExitCode::from(1);
    };
    let seed = oracle_seed(now);
    let out =
        if manifest { serde_json::to_string_pretty(&seed.manifest) } else { serde_json::to_string(&seed.fixture) };
    match out {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("oracle-fixture: {e}");
            ExitCode::from(1)
        }
    }
}
