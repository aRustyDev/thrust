//! The latency probe of the work-item path, as two load tests (one request at a time, and eight at once). Each prints p50, p95
//! and p99 for a create and a list through `/api/v1` (what the app's API clients call), for the slice's activity feed, and for a
//! create's whole way to a visible activity row. It needs a running target (`THRUST_BASE_URL`, `THRUST_ADMIN_TOKEN`):
//! `scripts/contract.sh <target> bench` starts one and runs these with the `load` profile, release-built.
//!
//! This measures the *local* path (localhost HTTP -> target -> adapters). It is not a production latency number: see Stage 2 in
//! research/0004. A test passes when every request it made succeeded; a number is not a pass or a fail (no budget gates yet,
//! X19). `THRUST_LOAD_ITERATIONS` sets the creates per test (200).
//!
//! The create and the list were the slice's until the vehicle tests moved (G-26), so a number from this probe is not comparable to
//! one recorded before: a `/api/v1` create reads the project's states and members, takes a state, writes the item's attributes and
//! a history, and its list is a page of at most a hundred. Each request counts against the key's allowance (Plane's 60 a minute,
//! `Target::v1_limit`), so the probe takes a fresh member (a fresh key) every `limit - 5` requests, whatever the target's limit is.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use thrust_contract_tests::*;
use thrust_load_tests::{env_usize, per_key, report};

async fn latency(concurrency: usize) {
    let n = env_usize("THRUST_LOAD_ITERATIONS", 200);
    let t = Arc::new(Target::from_env());
    let f = Arc::new(Fixture::seed(&t).await);

    // warm-up so isolate/connection start-up is not in the percentiles (reported separately)
    let w0 = Instant::now();
    let warm_user = f.extra_member(&t).await;
    v1_create_ok(&t, &warm_user, &f.a, "warmup").await;
    println!("first create (cold-ish): {:.2}ms", w0.elapsed().as_secs_f64() * 1000.0);

    // a key's allowance is per minute, and the batches below may be a concurrent few: stay clear of it
    let per_key = per_key(Target::v1_limit(), concurrency);
    let mut creates = Vec::new();
    let mut lists = Vec::new();
    let mut activities = Vec::new();
    let mut user = f.extra_member(&t).await;
    let mut used = 0usize;
    let mut last_item = None;

    // batches of `concurrency` requests (one at a time when it is 1)
    let mut done = 0usize;
    while done < n {
        let batch = concurrency.min(n - done);
        if used + batch > per_key {
            user = f.extra_member(&t).await;
            used = 0;
        }
        let mut handles = Vec::new();
        for i in 0..batch {
            let t = t.clone();
            let f = f.clone();
            let who = user.clone();
            let name = format!("bench-{}", done + i);
            handles.push(tokio::spawn(async move {
                let s = Instant::now();
                let r = v1_create(&t, &who, &f.a, json!({ "name": name })).await;
                let st = r.status();
                let body = r.bytes().await.unwrap();
                assert_eq!(st, 201, "{}", String::from_utf8_lossy(&body));
                let item = Issue(serde_json::from_slice(&body).unwrap());
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

    let mut lister = f.extra_member(&t).await;
    let mut listed = 0usize;
    for _ in 0..n.min(100) {
        if listed == per_key {
            lister = f.extra_member(&t).await;
            listed = 0;
        }
        let s = Instant::now();
        let r = v1_list(&t, &lister, &f.a).await;
        assert_eq!(r.status(), 200);
        let _ = r.bytes().await.unwrap();
        lists.push(s.elapsed());
        listed += 1;
    }
    if let Some(item) = last_item {
        // the slice's feed reads a session, and is not `/api/v1`'s: no key's allowance is spent here
        for _ in 0..n.min(100) {
            let s = Instant::now();
            let _ = activity(&t, &f.guest_a, &f.a, item.work_item_id()).await;
            activities.push(s.elapsed());
        }
    }

    // End-to-end delivery: create -> outbox -> queue -> consumer -> activity row visible to a reader. `THRUST_BENCH_SKIP_DELIVERY=1`
    // skips it: against a Hyperdrive config with caching ENABLED the poll below sees a cached empty list and times out (spike 0002,
    // research/0011), and the other three numbers would be lost with the panic.
    let skip_delivery = std::env::var("THRUST_BENCH_SKIP_DELIVERY").is_ok();
    let mut delivery = Vec::new();
    let mut duser = f.extra_member(&t).await;
    let mut dused = 0usize;
    for i in 0..(if skip_delivery { 0usize } else { 40usize }) {
        if dused == per_key.min(20) {
            duser = f.extra_member(&t).await;
            dused = 0;
        }
        let s = Instant::now();
        let item = v1_create_ok(&t, &duser, &f.a, &format!("delivery-{i}")).await;
        dused += 1;
        let id = item.work_item_id();
        loop {
            if !activity(&t, &f.guest_a, &f.a, id).await.is_empty() {
                break;
            }
            assert!(s.elapsed() < Duration::from_secs(20), "activity never appeared");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        delivery.push(s.elapsed());
    }

    println!("concurrency={concurrency}");
    report(&mut std::io::stdout(), "POST create work item", creates).unwrap();
    report(&mut std::io::stdout(), "GET list work items", lists).unwrap();
    report(&mut std::io::stdout(), "GET activity", activities).unwrap();
    if !skip_delivery {
        report(&mut std::io::stdout(), "create -> activity visible", delivery).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn load_latency_one_request_at_a_time() {
    latency(1).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn load_latency_eight_requests_at_once() {
    latency(8).await;
}
