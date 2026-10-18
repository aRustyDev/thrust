// TEMPLATE (rust-test-logic), adapted from rust-testing-plan.zip, 2026-10-18: copy to tests/oracle.rs; isqrt is an EXAMPLE unit: substitute your function and adjust the ../.. depth of the legacy-oracle path.
//! Live differential tests against the legacy TypeScript implementation (`oracle_` prefix).
//! Need Node.js + the built legacy package; run with `cargo nextest run --profile oracle`.

use proptest::prelude::*;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};

mod common; // tests/common/mod.rs: proptest_config with failure persistence

struct TsOracle {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl TsOracle {
    fn spawn() -> Self {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../legacy-oracle/ts-oracle.mjs");
        let mut child = Command::new("node")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("could not start `node`: oracle tests need Node.js and the built legacy package");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self { _child: child, stdin, stdout }
    }

    fn call(&mut self, func: &str, args: Value) -> Value {
        writeln!(self.stdin, "{}", json!({ "fn": func, "args": args })).unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let resp: Value = serde_json::from_str(&line).expect("oracle returned invalid JSON");
        if let Some(err) = resp.get("error") {
            panic!("legacy oracle failed on {func}: {err}");
        }
        resp["ok"].clone()
    }
}

/// One Node process per test process; nextest runs each test in its own process.
fn oracle() -> MutexGuard<'static, TsOracle> {
    static ORACLE: OnceLock<Mutex<TsOracle>> = OnceLock::new();
    ORACLE.get_or_init(|| Mutex::new(TsOracle::spawn())).lock().unwrap()
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    #[test]
    fn oracle_isqrt_matches_legacy(n in any::<u64>()) {
        let legacy: u64 = oracle()
            .call("isqrt", json!([n.to_string()]))
            .as_str().unwrap().parse().unwrap();
        prop_assert_eq!(your_crate::isqrt(n), legacy, "input {}", n);
    }
}
