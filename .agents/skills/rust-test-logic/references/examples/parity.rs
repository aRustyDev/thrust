// TEMPLATE (rust-test-logic), adapted from rust-testing-plan.zip, 2026-10-18; for scalar units (strings, integers) only: copy to tests/parity.rs; isqrt is an EXAMPLE unit: substitute your function, fixture glob and the your_crate import.
//! Golden parity tests: fixtures recorded from the legacy TypeScript implementation by
//! `legacy-oracle/gen-fixtures.mjs`. One nextest test per fixture file, discovered at runtime,
//! so new fixture files run without recompiling.
//!
//! Cargo.toml of the crate:
//!   [dev-dependencies] datatest-stable = "0.3", serde = { version = "1", features = ["derive"] }, serde_json = "1"
//!   [[test]] name = "parity"  harness = false

use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

/// Exactly one of `expected` / `error` is set. Integers are strings (JS f64 precision).
#[derive(Deserialize)]
struct Case {
    input: String,
    expected: Option<String>,
    error: Option<String>,
}

fn parity_isqrt(path: &Path, contents: String) -> datatest_stable::Result<()> {
    let fixture: Fixture = serde_json::from_str(&contents)?;
    let mut divergences = Vec::new();
    for case in fixture.cases {
        let input: u64 = case.input.parse()?;
        // For fallible functions: match on the Result and compare the error *kind*.
        let actual = your_crate::isqrt(input).to_string();
        match (case.expected, case.error) {
            (Some(expected), None) if actual != expected => {
                divergences.push(format!("isqrt({input}) = {actual}, legacy TS returned {expected}"))
            }
            (Some(_), None) => {}
            (None, Some(kind)) => {
                divergences.push(format!("isqrt({input}) = {actual}, legacy TS failed with {kind}"))
            }
            _ => return Err(format!("{}: a case needs exactly one of expected/error", path.display()).into()),
        }
    }
    if divergences.is_empty() {
        Ok(())
    } else {
        // Report every divergence in the file, not just the first.
        Err(format!("{}: {} divergence(s)\n{}", path.display(), divergences.len(), divergences.join("\n")).into())
    }
}

datatest_stable::harness! {
    { test = parity_isqrt, root = "tests/parity/fixtures", pattern = r"^isqrt-.*\.json$" },
}
