//! What the load tests compute, apart from the requests they make: percentiles of a sample of latencies, how many API keys a
//! run needs to stay under a key's allowance and which one to take next, and the deterministic work items the scale test seeds.
//! The load tests themselves (`tests/latency.rs`, `tests/scale.rs`, named `load_…`) need a running target and run with the
//! `load` profile; this library's own tests (`tests/measure.rs`) are hermetic and run in the default loop.

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The nearest-rank percentile of a sorted sample: the smallest value with at least `p` of the sample at or below it. `p` in
/// (0, 1]; an empty sample gives zero.
pub fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = ((sorted.len() as f64) * p).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// A sample's size, its 50th, 95th and 99th percentiles, and its largest value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub n: usize,
    pub p50: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub max: Duration,
}

pub fn summarize(mut sample: Vec<Duration>) -> Summary {
    sample.sort();
    Summary {
        n: sample.len(),
        p50: percentile(&sample, 0.50),
        p95: percentile(&sample, 0.95),
        p99: percentile(&sample, 0.99),
        max: sample.last().copied().unwrap_or(Duration::ZERO),
    }
}

/// One line of a load test's output: what was measured and its summary, in milliseconds.
pub fn report_line(name: &str, sample: Vec<Duration>) -> String {
    let s = summarize(sample);
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    format!(
        "{name:<44} n={:<5} p50={:>8.2}ms p95={:>8.2}ms p99={:>8.2}ms max={:>8.2}ms",
        s.n,
        ms(s.p50),
        ms(s.p95),
        ms(s.p99),
        ms(s.max)
    )
}

/// Write `report_line` and a newline to `out`: a load test's numbers are its output (the load tests pass stdout).
pub fn report(out: &mut impl std::io::Write, name: &str, sample: Vec<Duration>) -> std::io::Result<()> {
    writeln!(out, "{}", report_line(name, sample))
}

/// How many requests one API key is asked for before the next is taken: five below the allowance a minute (`limit`), and never
/// fewer than a batch of `concurrency`, so a batch does not straddle two keys.
pub fn per_key(limit: u32, concurrency: usize) -> usize {
    (limit as usize).saturating_sub(5).max(concurrency.max(1))
}

/// How many keys `requests` requests need when each key is asked for at most `per_key(limit, 1)` of them.
pub fn keys_needed(requests: usize, limit: u32) -> usize {
    requests.div_ceil(per_key(limit, 1))
}

/// Items taken in turn, round-robin, from any task.
pub struct Rotation<T> {
    items: Vec<T>,
    turn: AtomicUsize,
}

impl<T: Clone> Rotation<T> {
    /// `None` for no items: a rotation of nothing has nothing to give.
    pub fn new(items: Vec<T>) -> Option<Self> {
        (!items.is_empty()).then_some(Self { items, turn: AtomicUsize::new(0) })
    }

    pub fn next(&self) -> T {
        self.items[self.turn.fetch_add(1, Ordering::Relaxed) % self.items.len()].clone()
    }
}

/// A `usize` from the environment, or `default` when it is unset or not a number.
pub fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Run `n` calls of `f` with up to `concurrency` in flight, and return how long each took. A call that panics fails the run.
pub async fn timed<F, Fut>(n: usize, concurrency: usize, f: F) -> Vec<Duration>
where
    F: Fn(usize) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let f = Arc::new(f);
    let mut out = Vec::with_capacity(n);
    let mut done = 0;
    while done < n {
        let batch = concurrency.max(1).min(n - done);
        let handles: Vec<_> = (0..batch)
            .map(|k| {
                let f = f.clone();
                let i = done + k;
                tokio::spawn(async move {
                    let started = Instant::now();
                    f(i).await;
                    started.elapsed()
                })
            })
            .collect();
        for h in handles {
            match h.await {
                Ok(d) => out.push(d),
                Err(e) => std::panic::resume_unwind(e.into_panic()),
            }
        }
        done += batch;
    }
    out
}

const VERBS: &[&str] =
    &["Fix", "Add", "Remove", "Refactor", "Document", "Investigate", "Migrate", "Review", "Rename", "Test"];
const NOUNS: &[&str] = &[
    "login flow",
    "billing page",
    "search index",
    "queue consumer",
    "retry policy",
    "cycle view",
    "module board",
    "export job",
    "webhook handler",
    "email digest",
    "rate limiter",
    "audit log",
    "session cookie",
    "api key",
    "import wizard",
    "kanban column",
];

/// The `i`th work item the scale test seeds: a name that ends in `#i`, so no two are alike, and a description for four in five,
/// so a search has text to look in. The same `i` gives the same item.
pub fn scale_item(i: usize) -> (String, Option<String>) {
    let name = format!("{} {} #{i}", VERBS[i % VERBS.len()], NOUNS[(i / VERBS.len()) % NOUNS.len()]);
    let description = (!i.is_multiple_of(5)).then(|| {
        format!(
            "{} the {} so that the {} no longer {}; see {} for context (ref {:06})",
            VERBS[(i * 7) % VERBS.len()],
            NOUNS[(i * 3) % NOUNS.len()],
            NOUNS[(i * 5) % NOUNS.len()],
            ["fails", "stalls", "duplicates work", "loses events"][i % 4],
            NOUNS[(i * 11) % NOUNS.len()],
            i * 37 % 1_000_000
        )
    });
    (name, description)
}
