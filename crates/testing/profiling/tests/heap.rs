//! The heap profile of the v1 router workload, with dhat-rs installed as this test binary's global allocator (and only this one's:
//! every integration test file is a binary of its own). Run with the `prof` profile:
//!
//!   cargo nextest run -p thrust-profiling --profile prof -E 'binary(heap)'
//!
//! nextest runs each test in a process of its own, so a test's numbers hold no other test's allocations, and dhat's one profiler a
//! process is never contended. `prof_heap_router` writes `target/profiling/dhat-heap-router.json`, for dhat's viewer
//! (<https://nnethercote.github.io/dh_view/dh_view.html>), and prints the totals; no number gates yet (X19).

use std::path::PathBuf;

mod common;

use common::world;
use thrust_profiling::{run, Mix};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// Where the profiles go: the workspace's `target/profiling/` (THRUST_ROOT, `.cargo/config.toml`).
fn profile_dir() -> PathBuf {
    PathBuf::from(env!("THRUST_ROOT")).join("target").join("profiling")
}

#[test]
fn prof_heap_router() {
    let dir = profile_dir();
    std::fs::create_dir_all(&dir).expect("target/profiling can be made");
    let file = dir.join("dhat-heap-router.json");
    let _ = std::fs::remove_file(&file);
    let profiler = dhat::Profiler::builder().file_name(&file).build();

    let w = world();
    let out = run(&w, &Mix::PROFILE, 0);
    assert!(out.refused.is_empty(), "the workload was refused: {:?}", out.refused);
    assert_eq!(out.requests, Mix::PROFILE.requests());

    let s = dhat::HeapStats::get();
    println!(
        "prof_heap_router: {} requests ({:?}); allocated {} bytes in {} blocks; peak {} bytes in {} blocks; still held {} bytes in {} blocks",
        out.requests,
        Mix::PROFILE,
        s.total_bytes,
        s.total_blocks,
        s.max_bytes,
        s.max_blocks,
        s.curr_bytes,
        s.curr_blocks
    );
    drop(profiler);
    let written = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
    assert!(written > 0, "dhat wrote no profile at {}", file.display());
    println!("profile: {} ({written} bytes)", file.display());
}

/// The control: the profile counts what the run allocates. Each run gets a fresh world (a second run on the same world would find
/// the first run's work items and do more). Two runs with no ballast allocate the same to within a few hundred bytes (measured 758
/// on 2026-11-17), so the measure is not noise; a run that also holds `BALLAST` bytes allocates `BALLAST` more, to within that
/// noise, so the measure is of the run and not of nothing. A smaller mix than the profiled one: the control is about counting.
#[test]
fn prof_heap_counts_a_known_extra_allocation() {
    const BALLAST: u64 = 4 << 20;
    const MIX: Mix = Mix { creates: 50, lists: 10, searches: 5, updates: 10 };
    let tolerance = BALLAST / 100;
    let _profiler = dhat::Profiler::builder().testing().build();
    let measure = |ballast: u64| {
        let w = world();
        let before = dhat::HeapStats::get().total_bytes;
        let out = run(&w, &MIX, ballast as usize);
        assert!(out.refused.is_empty(), "the workload was refused: {:?}", out.refused);
        dhat::HeapStats::get().total_bytes - before
    };
    // first, a run whose one-time costs (lazy statics, the first world's allocations) are not compared
    let _ = measure(0);
    let a = measure(0);
    let b = measure(0);
    let with_ballast = measure(BALLAST);
    println!(
        "prof_heap_counts_a_known_extra_allocation: {a} and {b} bytes without ballast, {with_ballast} with {BALLAST}"
    );
    assert!(a.abs_diff(b) < tolerance, "two identical runs differ by {} bytes", a.abs_diff(b));
    let added = with_ballast.saturating_sub(a.min(b));
    assert!(added.abs_diff(BALLAST) < tolerance, "{BALLAST} bytes of ballast added {added} bytes");
}
