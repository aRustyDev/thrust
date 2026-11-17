//! The library's own tests: what the load tests compute, held to its definition over generated inputs. Hermetic (no target), so
//! they run in the default loop; the load tests that need a target are `load_…` in `latency.rs` and `scale.rs`.

use std::collections::HashSet;
use std::time::Duration;

use proptest::prelude::*;
use thrust_load_tests::*;

fn config(default_cases: u32) -> proptest::test_runner::Config {
    use proptest::test_runner::{Config, FailurePersistence, FileFailurePersistence};
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(default_cases);
    let failure_persistence: Option<Box<dyn FailurePersistence>> =
        if std::env::var_os("PROPTEST_DISABLE_FAILURE_PERSISTENCE").is_some() {
            None
        } else {
            Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions")))
        };
    Config { cases, failure_persistence, ..Config::default() }
}

/// A sample of latencies, sorted, from empty to a few hundred, with repeated values likely (small range).
fn sorted_sample() -> impl Strategy<Value = Vec<Duration>> {
    prop::collection::vec(0u64..50, 0..300).prop_map(|v| {
        let mut d: Vec<Duration> = v.into_iter().map(Duration::from_millis).collect();
        d.sort();
        d
    })
}

/// A percentile in (0, 1], with the ends and the three the reports use weighted in.
fn p() -> impl Strategy<Value = f64> {
    prop_oneof![Just(1.0), Just(0.5), Just(0.95), Just(0.99), (1u32..=10_000).prop_map(|k| f64::from(k) / 10_000.0),]
}

proptest! {
    #![proptest_config(config(256))]

    /// Nearest rank: the result is one of the sample, at least a fraction `p` of the sample is at or below it, and less than that
    /// is strictly below it. An empty sample gives zero.
    #[test]
    fn prop_percentile_is_the_nearest_rank(sample in sorted_sample(), p in p()) {
        let got = percentile(&sample, p);
        if sample.is_empty() {
            prop_assert_eq!(got, Duration::ZERO);
        } else {
            let n = sample.len();
            let rank = ((n as f64) * p).ceil() as usize;
            prop_assert!(sample.contains(&got));
            prop_assert!(sample.iter().filter(|d| **d <= got).count() >= rank.max(1));
            prop_assert!(sample.iter().filter(|d| **d < got).count() < rank.max(1));
        }
    }

    /// A higher percentile is never a smaller value, and the 100th is the largest.
    #[test]
    fn prop_percentile_grows_with_p(sample in sorted_sample(), a in p(), b in p()) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        prop_assert!(percentile(&sample, lo) <= percentile(&sample, hi));
        prop_assert_eq!(percentile(&sample, 1.0), sample.last().copied().unwrap_or(Duration::ZERO));
    }

    /// A summary is of the sample in any order, and its numbers are its percentiles and its largest value.
    #[test]
    fn prop_summary_is_of_the_sample_in_any_order(sample in sorted_sample(), seed in any::<u64>()) {
        let mut shuffled = sample.clone();
        let len = shuffled.len();
        if len > 1 {
            // a deterministic shuffle from the seed: every element swapped with one further on
            let mut s = seed;
            for i in 0..len - 1 {
                s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                let j = i + (s as usize) % (len - i);
                shuffled.swap(i, j);
            }
        }
        let got = summarize(shuffled);
        prop_assert_eq!(got.n, sample.len());
        prop_assert_eq!(got.p50, percentile(&sample, 0.50));
        prop_assert_eq!(got.p95, percentile(&sample, 0.95));
        prop_assert_eq!(got.p99, percentile(&sample, 0.99));
        prop_assert_eq!(got.max, sample.last().copied().unwrap_or(Duration::ZERO));
    }

    /// A report line says the sample's size and its summary in milliseconds, after the name, and `report` writes exactly that line.
    /// Whole-millisecond samples, so every figure is `<millis>.00`: worked out here from the summary, not by the report's arithmetic.
    #[test]
    fn prop_report_says_the_summary_in_milliseconds(
        millis in prop::collection::vec(0u64..10_000, 0..200),
        name in "[a-z][a-z ]{0,19}",
    ) {
        let sample: Vec<Duration> = millis.iter().map(|m| Duration::from_millis(*m)).collect();
        let s = summarize(sample.clone());
        let figure = |label: &str, d: Duration| format!("{label}={:>8}ms", format!("{}.00", d.as_millis()));
        let line = report_line(&name, sample.clone());
        prop_assert!(line.starts_with(&name));
        let n = format!("n={:<5}", s.n);
        prop_assert!(line.contains(&n), "{} lacks {}", line, n);
        for (label, d) in [("p50", s.p50), ("p95", s.p95), ("p99", s.p99), ("max", s.max)] {
            let want = figure(label, d);
            prop_assert!(line.contains(&want), "{} lacks {}", line, want);
        }
        let mut out = Vec::new();
        report(&mut out, &name, sample).expect("writing to memory");
        prop_assert_eq!(String::from_utf8(out).expect("utf-8"), format!("{line}\n"));
    }

    /// A number in the environment is read; an unset variable, or one that is not a number, gives the default.
    #[test]
    fn prop_env_usize_reads_a_number_or_gives_the_default(
        value in any::<u32>(),
        default in any::<u32>(),
        garbage in "[a-z]{1,8}",
        case in any::<u64>(),
    ) {
        // a name of its own per case; nextest runs each test in a process of its own, so no other test sees it
        let name = format!("THRUST_LOAD_TESTS_PROP_{case}");
        std::env::remove_var(&name);
        prop_assert_eq!(env_usize(&name, default as usize), default as usize);
        std::env::set_var(&name, value.to_string());
        prop_assert_eq!(env_usize(&name, default as usize), value as usize);
        std::env::set_var(&name, &garbage);
        prop_assert_eq!(env_usize(&name, default as usize), default as usize);
        std::env::remove_var(&name);
    }

    /// Enough keys for the requests, and not one more than that: each key is asked for at most five below the allowance (never
    /// fewer than one), and no key is left unused.
    #[test]
    fn prop_keys_needed_cover_the_requests_and_no_more(requests in 0usize..5_000, limit in 0u32..2_000) {
        let per = per_key(limit, 1);
        let keys = keys_needed(requests, limit);
        prop_assert!(per >= 1);
        prop_assert!(per <= (limit as usize).saturating_sub(5).max(1));
        prop_assert!(keys * per >= requests);
        if requests > 0 {
            prop_assert!((keys - 1) * per < requests);
        } else {
            prop_assert_eq!(keys, 0);
        }
    }

    /// A batch never straddles two keys: a key is asked for at least a batch.
    #[test]
    fn prop_a_key_holds_at_least_a_batch(limit in 0u32..2_000, concurrency in 0usize..64) {
        prop_assert!(per_key(limit, concurrency) >= concurrency.max(1));
    }

    /// Round-robin: after `calls` turns over `k` items, every item has been taken the floor or the ceiling of `calls / k` times,
    /// in order.
    #[test]
    fn prop_rotation_is_round_robin(k in 1usize..20, calls in 0usize..200) {
        let r = Rotation::new((0..k).collect::<Vec<_>>()).expect("k >= 1 items");
        let taken: Vec<usize> = (0..calls).map(|_| r.next()).collect();
        for (i, item) in taken.iter().enumerate() {
            prop_assert_eq!(*item, i % k);
        }
        for item in 0..k {
            let n = taken.iter().filter(|t| **t == item).count();
            prop_assert!(n == calls / k || n == calls.div_ceil(k));
        }
    }

    /// The seeded items: the same index gives the same item, no two indexes the same name, and a description for exactly the
    /// indexes not divisible by five.
    #[test]
    fn prop_scale_items_are_deterministic_distinct_and_mostly_described(start in 0usize..1_000_000, n in 1usize..300) {
        let mut names = HashSet::new();
        for i in start..start + n {
            let (name, description) = scale_item(i);
            prop_assert_eq!(scale_item(i), (name.clone(), description.clone()));
            let suffix = format!("#{i}");
            prop_assert!(name.ends_with(&suffix));
            prop_assert!(names.insert(name));
            prop_assert_eq!(description.is_some(), i % 5 != 0);
        }
    }
}

/// A rotation of nothing is refused rather than dividing by zero on its first turn.
#[test]
fn an_empty_rotation_is_refused() {
    // the one input outside the generator above (it draws k >= 1)
    assert!(Rotation::<u8>::new(Vec::new()).is_none());
}

/// `timed` makes every call once, each with its own index `0..n`, at most `concurrency` at once, and returns one duration a call.
#[test]
fn prop_timed_runs_every_call_once_within_its_concurrency() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("a runtime");
    let mut runner = proptest::test_runner::TestRunner::new(config(64));
    runner
        .run(&(0usize..60, 0usize..10), |(n, concurrency)| {
            let indexes = Arc::new(Mutex::new(Vec::new()));
            let in_flight = Arc::new(AtomicUsize::new(0));
            let most = Arc::new(AtomicUsize::new(0));
            let (ix, f, m) = (indexes.clone(), in_flight.clone(), most.clone());
            let durations = rt.block_on(timed(n, concurrency, move |i| {
                let (ix, f, m) = (ix.clone(), f.clone(), m.clone());
                async move {
                    let now = f.fetch_add(1, Ordering::SeqCst) + 1;
                    m.fetch_max(now, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    ix.lock().expect("not poisoned").push(i);
                    f.fetch_sub(1, Ordering::SeqCst);
                }
            }));
            let mut seen = indexes.lock().expect("not poisoned").clone();
            seen.sort_unstable();
            prop_assert_eq!(durations.len(), n);
            prop_assert_eq!(seen, (0..n).collect::<Vec<_>>());
            prop_assert!(most.load(Ordering::SeqCst) <= concurrency.max(1));
            Ok(())
        })
        .expect("timed's property");
}
