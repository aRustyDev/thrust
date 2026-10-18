//! Local latency probe for the slice. Prints p50/p95/p99 for the three slice operations.
//! Usage: THRUST_BASE_URL=... THRUST_ADMIN_TOKEN=... bench [iterations] [concurrency]
//!
//! This measures the *local* path (localhost HTTP -> target -> adapters). It is not a
//! production latency number: see Stage 2 in research/0004.

use std::time::{Duration, Instant};

use serde_json::json;
use thrust_contract_tests::*;

fn pct(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() as f64) * p).ceil() as usize;
    sorted[idx.saturating_sub(1).min(sorted.len() - 1)]
}

fn report(name: &str, mut v: Vec<Duration>) {
    v.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!(
        "{name:<28} n={:<5} p50={:>7.2}ms p95={:>7.2}ms p99={:>7.2}ms max={:>7.2}ms",
        v.len(),
        ms(pct(&v, 0.50)),
        ms(pct(&v, 0.95)),
        ms(pct(&v, 0.99)),
        ms(*v.last().unwrap_or(&Duration::ZERO)),
    );
}

#[tokio::main]
async fn main() {
    let n: usize = std::env::args().nth(1).and_then(|v| v.parse().ok()).unwrap_or(200);
    let conc: usize = std::env::args().nth(2).and_then(|v| v.parse().ok()).unwrap_or(1);
    let t = std::sync::Arc::new(Target::from_env());
    let f = std::sync::Arc::new(Fixture::seed(&t).await);

    // warm-up so isolate/connection start-up is not in the percentiles (reported separately)
    let w0 = Instant::now();
    let warm_user = f.extra_member(&t).await;
    create_ok(&t, &warm_user, f.project_a, "warmup").await;
    println!("first create (cold-ish): {:.2}ms", w0.elapsed().as_secs_f64() * 1000.0);

    let per_user = (Target::create_limit() as usize).saturating_sub(5).max(1);
    let mut creates = Vec::new();
    let mut lists = Vec::new();
    let mut activities = Vec::new();
    let mut user = f.extra_member(&t).await;
    let mut used = 0usize;
    let mut last_item = None;

    // sequential section (conc == 1) or batches of `conc` concurrent requests
    let mut done = 0usize;
    while done < n {
        let batch = conc.min(n - done);
        if used + batch > per_user {
            user = f.extra_member(&t).await;
            used = 0;
        }
        let mut handles = Vec::new();
        for i in 0..batch {
            let t = t.clone();
            let f = f.clone();
            let tok = user.token.clone();
            let name = format!("bench-{}", done + i);
            handles.push(tokio::spawn(async move {
                let s = Instant::now();
                let r = t
                    .http
                    .post(t.url(&wi_path(f.project_a)))
                    .bearer_auth(tok)
                    .json(&json!({ "name": name }))
                    .send()
                    .await
                    .unwrap();
                let st = r.status();
                let body = r.bytes().await.unwrap();
                assert_eq!(st, 201, "{}", String::from_utf8_lossy(&body));
                let item: thrust_contracts::WorkItem = serde_json::from_slice(&body).unwrap();
                (s.elapsed(), item)
            }));
        }
        for h in handles {
            let (d, item) = h.await.unwrap();
            creates.push(d);
            last_item = Some(item);
        }
        used += batch;
        done += batch;
    }

    for _ in 0..n.min(100) {
        let s = Instant::now();
        let r = list(&t, &f.guest_a, f.project_a, None).await;
        assert_eq!(r.status(), 200);
        let _ = r.bytes().await.unwrap();
        lists.push(s.elapsed());
    }
    if let Some(item) = last_item {
        for _ in 0..n.min(100) {
            let s = Instant::now();
            let _ = activity(&t, &f.guest_a, f.project_a, item.id).await;
            activities.push(s.elapsed());
        }
    }

    println!("concurrency={conc}");
    report("POST create work item", creates);
    report("GET list work items", lists);
    report("GET activity", activities);
}
