//! The native half of spike 0003's K4: how long one argon2id hash takes at each of OWASP's five equivalent
//! configurations, on this machine (issue #38). The deployed half is `scripts/measure-argon2.sh`.
//!
//!   scripts/rs cargo run --release -p thrust-core --example argon2_timing -- [runs]
//!
//! It is an example, not a test: it reads a wall clock, which hermetic tests may not (AGENTS.md rule 6). The release
//! profile of this workspace optimises for size (`opt-level = "s"`, the Worker's), so this is the speed of the same build
//! the wasm bundle gets, not of a speed-tuned native one.

use std::time::Instant;

use thrust_core::{Argon2Params, Argon2idHasher};
use thrust_ports::PasswordHasher;

/// OWASP Password Storage Cheat Sheet: the minimum configuration and its four equivalents (p = 1).
const OWASP: [(u32, u32); 5] = [(47_104, 1), (19_456, 2), (12_288, 3), (9_216, 4), (7_168, 5)];

fn main() {
    let runs: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(100);
    println!("argon2id, p = 1, {runs} runs each, ms per hash (wall, native, this build profile)");
    println!("{:>8} {:>3} {:>9} {:>9} {:>9} {:>9}", "m KiB", "t", "p50", "p99", "max", "mean");
    for (m, t) in OWASP {
        let hasher = Argon2idHasher::new(Argon2Params { m_cost_kib: m, t_cost: t, p_cost: 1 });
        let mut ms: Vec<f64> = (0..runs)
            .map(|i| {
                let salt = [(i % 251) as u8; 16];
                let start = Instant::now();
                let phc = hasher.hash("correct horse battery staple", &salt).expect("hash");
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                assert!(phc.contains(&format!("m={m},t={t},p=1")), "{phc}");
                elapsed
            })
            .collect();
        ms.sort_by(f64::total_cmp);
        let pct = |p: f64| ms[((ms.len() as f64 * p).ceil() as usize).clamp(1, ms.len()) - 1];
        let mean = ms.iter().sum::<f64>() / ms.len() as f64;
        println!("{m:>8} {t:>3} {:>9.1} {:>9.1} {:>9.1} {:>9.1}", pct(0.50), pct(0.99), ms[ms.len() - 1], mean);
    }
}
