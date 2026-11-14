//! `DATABASE_URL=... migrate` — applies the adapter's own SQL. Native only (feature `native`).
//!
//! Everything it is told comes through the environment, never argv (the URL has the password in it):
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

/// An environment variable that is either not there or readable. One that is set to something that is not UTF-8 is an error, not
/// "not set": a setting that is silently ignored is how a deploy ends up connecting without the TLS it was told to use.
fn env(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(v) => Ok(Some(v)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        // the cause too (`error_chain`): tokio-postgres's own text for a refused TLS handshake or a pg_hba rejection omits it
        Err(e) => {
            eprintln!("migrate: {}", error_chain(e.as_ref()));
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let url = env("DATABASE_URL")?.ok_or("DATABASE_URL must be set")?;
    let mode = env("DATABASE_SSLMODE")?;
    let root_cert = match env("DATABASE_SSLROOTCERT")? {
        None => None,
        Some(path) => Some(std::fs::read(&path).map_err(|e| format!("DATABASE_SSLROOTCERT {path}: {e}"))?),
    };
    let tls = Tls::from_settings(mode.as_deref(), root_cert.as_deref())?;
    let conn = connect(&url, &tls).await?;
    thrust_adapters_postgres::migrate(&conn.client).await?;
    // Not `?`: every statement has run once `migrate` returns, so a connection that does not hang up cleanly says nothing about the
    // migration, and a deploy must not go red over it. Against Postgres 16 `close` returned Ok (`CLOSE-OBSERVATION` in
    // thrust-adapters-selfhosted/tests/tls); another server, or a proxy in front of one, was not tried.
    let _ = conn.close().await;
    eprintln!("migrated (tls: {})", tls.mode());
    Ok(())
}
