//! `DATABASE_URL=... migrate [--no-contract | --plan]` — applies the adapter's own SQL. Native only (feature `native`).
//!
//! Each migration file is applied once, and recorded in the database's ledger (`thrust_schema_migrations`, issue #195); a database
//! without one gets every file once and all of them recorded. What the ledger refuses (a recorded file that was edited, a build from a
//! branch that lacks what was applied since, a file older than the newest one applied) fails the run and applies nothing. A build
//! older than the schema applies nothing and succeeds. See `thrust_adapters_postgres::ledger`.
//!
//!   (no flag)       apply what is missing, contract migrations (`NNNN_contract_*.sql`) included
//!   --no-contract   the same, but a contract migration to apply refuses the run and nothing is applied: the deploy of a target that
//!                   is not production must not take away what an older version still uses
//!   --plan          print what would be done as one line of JSON on stdout, and change nothing (not even the ledger table):
//!                   `{"outcome": "baseline|current|apply|older|diverged|out_of_order|changed", "pending": [..], "contract": [..],
//!                   "unknown": [..], "changed": [..]}`. It exits 0 whenever it printed a plan, refusals included: the plan is the answer
//!
//! Exit status: 0 done (or planned); 1 refused, or the database or the connection failed; 2 called wrongly (nothing was attempted).
//!
//! Everything else it is told comes through the environment, never argv (the URL has the password in it):
//!
//!   DATABASE_URL          the connection string
//!   DATABASE_SSLMODE      `disable` (the default) or `verify-ca`. Anything else is refused, `require` and `verify-full` included
//!   DATABASE_SSLROOTCERT  the PEM file of the CA certificate(s) to trust. Required by `verify-ca`, refused with `disable`
//!
//! `verify-ca` checks the server's certificate chain, signature and dates, and not its name, because the deploy reaches `thrust-pg`
//! through a tunnel at 127.0.0.1 while the certificate names a host (issue #49, ADR-0094). It never falls back to plaintext: a server
//! that answers no to TLS is an error, and so is a URL whose `sslmode` says the opposite. See `thrust_adapters_postgres::tls`.

use std::process::ExitCode;

use thrust_adapters_postgres::tls::{connect, error_chain, Tls};
use thrust_adapters_postgres::{migrate_with, plan_migrations, MigrateOptions, MIGRATIONS};

/// An environment variable that is either not there or readable. One that is set to something that is not UTF-8 is an error, not
/// "not set": a setting that is silently ignored is how a deploy ends up connecting without the TLS it was told to use.
fn env(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(v) => Ok(Some(v)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

/// What the command line asked for.
#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Apply(MigrateOptions),
    Plan,
}

/// The flags, of which at most one is given. Anything else is a usage error, so a typo is not read as "apply everything".
fn mode(args: &[String]) -> Result<Mode, String> {
    match args {
        [] => Ok(Mode::Apply(MigrateOptions { allow_contract: true })),
        [one] if one == "--no-contract" => Ok(Mode::Apply(MigrateOptions { allow_contract: false })),
        [one] if one == "--plan" => Ok(Mode::Plan),
        _ => Err(format!("usage: migrate [--no-contract | --plan] (got: {})", args.join(" "))),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match mode(&args) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("migrate: {e}");
            return ExitCode::from(2);
        }
    };
    match run(mode).await {
        Ok(()) => ExitCode::SUCCESS,
        // the cause too (`error_chain`): tokio-postgres's own text for a refused TLS handshake or a pg_hba rejection omits it
        Err(e) => {
            eprintln!("migrate: {}", error_chain(e.as_ref()));
            ExitCode::FAILURE
        }
    }
}

async fn run(mode: Mode) -> Result<(), Box<dyn std::error::Error>> {
    let url = env("DATABASE_URL")?.ok_or("DATABASE_URL must be set")?;
    let ssl_mode = env("DATABASE_SSLMODE")?;
    let root_cert = match env("DATABASE_SSLROOTCERT")? {
        None => None,
        Some(path) => Some(std::fs::read(&path).map_err(|e| format!("DATABASE_SSLROOTCERT {path}: {e}"))?),
    };
    let tls = Tls::from_settings(ssl_mode.as_deref(), root_cert.as_deref())?;
    let conn = connect(&url, &tls).await?;
    let said = match mode {
        Mode::Plan => {
            let (present, plan) = plan_migrations(&conn.client, &MIGRATIONS).await?;
            println!("{}", plan.to_json(present));
            format!("planned (tls: {})", tls.mode())
        }
        Mode::Apply(options) => {
            let outcome = migrate_with(&conn.client, &MIGRATIONS, options).await?;
            format!("migrated (tls: {}): {outcome}", tls.mode())
        }
    };
    // Not `?`: every statement has run once `migrate` returns, so a connection that does not hang up cleanly says nothing about the
    // migration, and a deploy must not go red over it. Against Postgres 16 `close` returned Ok (`CLOSE-OBSERVATION` in
    // thrust-adapters-selfhosted/tests/tls); another server, or a proxy in front of one, was not tried.
    let _ = conn.close().await;
    eprintln!("{said}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    /// Every command line of up to two words from the flags and a stranger, against what the usage says: none, or exactly one flag.
    #[rstest]
    fn the_command_line_takes_at_most_one_flag(
        #[values(None, Some("--no-contract"), Some("--plan"), Some("--force"))] first: Option<&str>,
        #[values(None, Some("--no-contract"), Some("--plan"), Some("--force"))] second: Option<&str>,
    ) {
        let args: Vec<String> = [first, second].into_iter().flatten().map(String::from).collect();
        let want = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
            [] => Some(Mode::Apply(MigrateOptions { allow_contract: true })),
            ["--no-contract"] => Some(Mode::Apply(MigrateOptions { allow_contract: false })),
            ["--plan"] => Some(Mode::Plan),
            _ => None,
        };
        assert_eq!(mode(&args).ok(), want, "{args:?}");
    }
}
