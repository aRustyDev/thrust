//! The CPU profile of the v1 router workload. The `prof` profile runs every `prof_cpu_` test under samply, through the nextest
//! wrapper script `scripts/samply-wrapper.sh`, which writes `target/profiling/<test>.json.gz` (open it with `samply load <file>`,
//! a flame graph in the Firefox Profiler). Profile the build that ships:
//!
//!   cargo nextest run -p thrust-profiling --profile prof --cargo-profile release -E 'test(/^prof_cpu_/)'
//!
//! This binary keeps the system allocator: dhat (`tests/heap.rs`) records a backtrace per allocation, which a CPU profile would
//! then be a profile of. No number gates yet (X19).

mod common;

use common::world;
use thrust_profiling::{run, Mix};

/// How many times the profiled mix runs, each on a fresh world (a second run on the same world would find the first run's work
/// items and do more). samply samples at 1 kHz, and one run of the mix is too short for a profile worth reading.
const ROUNDS: usize = 20;

#[test]
fn prof_cpu_router() {
    // Set by the wrapper: without it this would pass, and no profile would be written.
    assert!(
        std::env::var_os("THRUST_PROF_SAMPLY").is_some(),
        "prof_cpu_router runs under samply, through the `prof` profile's wrapper (scripts/samply-wrapper.sh): \
         cargo nextest run -p thrust-profiling --profile prof"
    );
    let mut requests = 0;
    for _ in 0..ROUNDS {
        let out = run(&world(), &Mix::PROFILE, 0);
        assert!(out.refused.is_empty(), "the workload was refused: {:?}", out.refused);
        requests += out.requests;
    }
    assert_eq!(requests, ROUNDS * Mix::PROFILE.requests());
    println!("prof_cpu_router: {requests} requests, {ROUNDS} rounds of {:?}", Mix::PROFILE);
}
