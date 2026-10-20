//! The ONE contract suite. Identical assertions and fixtures on every target.
//!
//! Every test that asserts "X is allowed" is paired with a control asserting "the neighbouring Y
//! is rejected" (cross-tenant, non-member, guest, duplicate, over-limit, oversize), so a server
//! that accepts everything cannot pass. `scripts/mutants.sh` additionally runs this suite
//! against deliberately broken servers and requires it to FAIL.
//!
//! The hand-written scenarios below each pin one behaviour. `model` generates them instead: random operation
//! sequences against a reference model, and every caller × project × route against the access table.

mod model;

use std::time::Duration;

use futures_util::future::join_all;
use serde_json::{json, Value};
use thrust_contract_tests::*;
use thrust_contracts::*;

fn t() -> Target {
    Target::from_env()
}

#[tokio::test]
async fn health_ok() {
    let t = t();
    let r = t.http.get(t.url("/healthz")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().await.unwrap(), "ok");
}

#[tokio::test]
async fn unauthenticated_requests_are_rejected() {
    let t = t();
    let f = Fixture::seed(&t).await;
    // no credentials
    let r = t.http.post(t.url(&wi_path(f.project_a))).json(&json!({"name":"x"})).send().await.unwrap();
    assert_eq!(r.status(), 401, "no token");
    // unknown token
    let r = t
        .http
        .post(t.url(&wi_path(f.project_a)))
        .bearer_auth("not-a-session")
        .json(&json!({"name":"x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401, "bad token");
    // control: nothing was created by the rejected calls
    let items: Vec<WorkItem> = list(&t, &f.admin_a, f.project_a, None).await.json().await.unwrap();
    assert!(items.is_empty());
}

#[tokio::test]
async fn internal_routes_require_the_admin_token() {
    let t = t();
    let r = t.http.post(t.url("/internal/sweep")).send().await.unwrap();
    assert_eq!(r.status(), 401, "no admin token");
    let r = t.http.post(t.url("/internal/sweep")).header("x-admin-token", "wrong").send().await.unwrap();
    assert_eq!(r.status(), 401, "wrong admin token");
}

#[tokio::test]
async fn create_allocates_monotonic_sequence_and_sort_order() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let a = create_ok(&t, &f.member_a, f.project_a, "first").await;
    let b = create_ok(&t, &f.member_a, f.project_a, "second").await;
    assert_eq!((a.sequence_id, b.sequence_id), (1, 2));
    assert_eq!((a.sort_order, b.sort_order), (10_000, 20_000));
    assert_eq!(a.workspace_id, f.ws_a);
    assert_eq!(a.created_by, f.member_a.id);
    assert!(a.deleted_at.is_none());
    // control: a different project has its own counter (sequence is per project, not global)
    let c = create_ok(&t, &f.admin_b, f.project_b, "other tenant").await;
    assert_eq!(c.sequence_id, 1);
}

#[tokio::test]
async fn list_returns_visible_items_in_sequence_order() {
    let t = t();
    let f = Fixture::seed(&t).await;
    for n in ["a", "b", "c"] {
        create_ok(&t, &f.member_a, f.project_a, n).await;
    }
    let items: Vec<WorkItem> = list(&t, &f.guest_a, f.project_a, None).await.json().await.unwrap();
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["a", "b", "c"]);
    assert_eq!(items.iter().map(|i| i.sequence_id).collect::<Vec<_>>(), [1, 2, 3]);
}

#[tokio::test]
async fn soft_delete_hides_rows_and_never_reuses_sequence_ids() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let _one = create_ok(&t, &f.member_a, f.project_a, "one").await;
    let two = create_ok(&t, &f.member_a, f.project_a, "two").await;
    let _three = create_ok(&t, &f.member_a, f.project_a, "three").await;

    let r = t
        .http
        .delete(t.url(&format!("{}/{}", wi_path(f.project_a), two.id)))
        .bearer_auth(&f.member_a.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);

    // default scope hides the deleted row
    let visible: Vec<WorkItem> = list(&t, &f.member_a, f.project_a, None).await.json().await.unwrap();
    assert_eq!(visible.iter().map(|i| i.sequence_id).collect::<Vec<_>>(), [1, 3]);

    // control 1: a member may NOT ask for deleted rows
    let r = list(&t, &f.member_a, f.project_a, Some("with_deleted")).await;
    assert_eq!(r.status(), 403, "member must not see deleted rows");
    // control 2: unknown scope is a client error, not a silent default
    assert_eq!(list(&t, &f.member_a, f.project_a, Some("bogus")).await.status(), 400);

    // an admin may, and sees the tombstone
    let all: Vec<WorkItem> = list(&t, &f.admin_a, f.project_a, Some("with_deleted")).await.json().await.unwrap();
    assert_eq!(all.len(), 3);
    let tomb = all.iter().find(|i| i.id == two.id).unwrap();
    assert!(tomb.deleted_at.is_some());

    // deleting twice is a 404, not a second success
    let r = t
        .http
        .delete(t.url(&format!("{}/{}", wi_path(f.project_a), two.id)))
        .bearer_auth(&f.member_a.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // numbers are never reused: the next item is #4, not #2 or #3 (ADR-0043 B1)
    let four = create_ok(&t, &f.member_a, f.project_a, "four").await;
    assert_eq!(four.sequence_id, 4);
}

#[tokio::test]
async fn cross_tenant_access_is_rejected_and_has_no_effect() {
    let t = t();
    let f = Fixture::seed(&t).await;
    // Tenant B's admin targets tenant A's project: 404, indistinguishable from "no such project".
    let r = create(&t, &f.admin_b, f.project_a, json!({"name": "intruder"})).await;
    assert_eq!(r.status(), 404, "cross-tenant create must be 404");
    assert_eq!(list(&t, &f.admin_b, f.project_a, None).await.status(), 404, "cross-tenant list");
    assert_eq!(
        list(&t, &f.admin_b, f.project_a, Some("with_deleted")).await.status(),
        404,
        "cross-tenant with_deleted list"
    );
    // control: the rejected create left nothing behind in tenant A ...
    let items: Vec<WorkItem> = list(&t, &f.admin_a, f.project_a, None).await.json().await.unwrap();
    assert!(items.is_empty(), "cross-tenant create leaked a row");
    // ... and tenant A's own admin is NOT rejected on the same path (the 404 above is tenancy, not a bad route)
    assert_eq!(list(&t, &f.admin_a, f.project_a, None).await.status(), 200);
}

#[tokio::test]
async fn non_members_and_guests_are_forbidden_to_write() {
    let t = t();
    let f = Fixture::seed(&t).await;
    assert_eq!(create(&t, &f.outsider_a, f.project_a, json!({"name":"x"})).await.status(), 403, "outsider create");
    assert_eq!(list(&t, &f.outsider_a, f.project_a, None).await.status(), 403, "outsider list");
    assert_eq!(create(&t, &f.guest_a, f.project_a, json!({"name":"x"})).await.status(), 403, "guest create");
    // control: guests can read, members can write
    assert_eq!(list(&t, &f.guest_a, f.project_a, None).await.status(), 200, "guest list");
    assert_eq!(create(&t, &f.member_a, f.project_a, json!({"name":"ok"})).await.status(), 201);
}

#[tokio::test]
async fn external_id_is_unique_among_live_rows_only() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let body = |ext: &str| json!({"name": "imported", "external_source": "github", "external_id": ext});
    let first = create(&t, &f.member_a, f.project_a, body("42")).await;
    assert_eq!(first.status(), 201);
    let first: WorkItem = first.json().await.unwrap();
    // duplicate live (source, id) -> 409 (the must-fail control for the unique constraint)
    let dup = create(&t, &f.member_a, f.project_a, body("42")).await;
    assert_eq!(dup.status(), 409, "duplicate external_id must conflict");
    // control: a different id is fine
    assert_eq!(create(&t, &f.member_a, f.project_a, body("43")).await.status(), 201);
    // the failed duplicate must not have burned a visible row
    let n = list(&t, &f.member_a, f.project_a, None).await.json::<Vec<WorkItem>>().await.unwrap().len();
    assert_eq!(n, 2);
    // after a soft delete the key is free again (partial unique index semantics)
    let r = t
        .http
        .delete(t.url(&format!("{}/{}", wi_path(f.project_a), first.id)))
        .bearer_auth(&f.member_a.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(create(&t, &f.member_a, f.project_a, body("42")).await.status(), 201, "re-create after soft delete");
}

#[tokio::test]
async fn concurrent_creates_never_duplicate_a_sequence_id() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let n = 20usize;
    let futs = (0..n).map(|i| {
        let t = &t;
        let f = &f;
        async move { create(t, &f.member_a, f.project_a, json!({"name": format!("c{i}")})).await }
    });
    let resps = join_all(futs).await;
    let mut seqs = Vec::new();
    let mut sorts = Vec::new();
    for r in resps {
        assert_eq!(r.status(), 201);
        let w: WorkItem = r.json().await.unwrap();
        seqs.push(w.sequence_id);
        sorts.push(w.sort_order);
    }
    seqs.sort();
    sorts.sort();
    assert_eq!(seqs, (1..=n as i64).collect::<Vec<_>>(), "sequence ids must be unique and contiguous");
    sorts.dedup();
    assert_eq!(sorts.len(), n, "sort_order must be unique too");
}

#[tokio::test]
async fn outbox_event_reaches_the_queue_consumer_exactly_once() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let item = create_ok(&t, &f.member_a, f.project_a, "evented").await;

    let acts = eventually("created activity row", Duration::from_secs(15), || async {
        let a = activity(&t, &f.guest_a, f.project_a, item.id).await;
        (!a.is_empty()).then_some(a)
    })
    .await;
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0].verb, "created");
    assert_eq!(acts[0].actor_id, Some(f.member_a.id));
    assert_eq!(acts[0].entity_id, item.id.as_uuid());
    assert_eq!(acts[0].workspace_id, f.ws_a);

    // delete -> second event -> second activity row; no duplicates of the first
    let r = t
        .http
        .delete(t.url(&format!("{}/{}", wi_path(f.project_a), item.id)))
        .bearer_auth(&f.member_a.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let acts = eventually("deleted activity row", Duration::from_secs(15), || async {
        let a = activity(&t, &f.guest_a, f.project_a, item.id).await;
        (a.len() >= 2).then_some(a)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let acts_later = activity(&t, &f.guest_a, f.project_a, item.id).await;
    assert_eq!(acts.len(), 2);
    assert_eq!(acts_later.len(), 2, "redelivery must not duplicate activity rows");
    let mut verbs: Vec<_> = acts_later.iter().map(|a| a.verb.clone()).collect();
    verbs.sort();
    assert_eq!(verbs, ["created", "deleted"]);

    // control: tenant B cannot read tenant A's activity
    let r = t
        .http
        .get(t.url(&format!("{}/{}/activity", wi_path(f.project_a), item.id)))
        .bearer_auth(&f.admin_b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

/// The test above never actually redelivers anything, so on its own it cannot tell an idempotent
/// consumer from a consumer that happens to be called once. This one forces at-least-once
/// behaviour: the SAME envelope is put on the queue twice, and a different one once.
#[tokio::test]
async fn redelivered_event_is_processed_once() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let item = create_ok(&t, &f.member_a, f.project_a, "redelivery").await;
    eventually("original activity row", Duration::from_secs(15), || async {
        (activity(&t, &f.guest_a, f.project_a, item.id).await.len() == 1).then_some(())
    })
    .await;

    let envelope = |event_id: uuid::Uuid, verb: &str| {
        json!({
            "event_id": event_id,
            "schema_version": 1,
            "workspace_id": f.ws_a,
            "event_type": format!("work_item.{verb}"),
            "entity_type": "work_item",
            "entity_id": item.id,
            "actor_id": f.member_a.id,
            "occurred_at": "2026-10-18T00:00:00Z",
            "payload": { "project_id": f.project_a, "changed_fields": ["name"] },
            "trace_id": null
        })
    };
    let dup_id = uuid::Uuid::new_v4();
    for _ in 0..2 {
        let r = t.admin_post("/internal/inject-event", &envelope(dup_id, "updated")).await;
        assert_eq!(r.status(), 202, "inject: {:?}", r.text().await);
    }
    // control: a DIFFERENT event id must still be processed (the dedupe is by event id, not "drop everything")
    let r = t.admin_post("/internal/inject-event", &envelope(uuid::Uuid::new_v4(), "moved")).await;
    assert_eq!(r.status(), 202);

    eventually("both distinct injected events", Duration::from_secs(15), || async {
        (activity(&t, &f.guest_a, f.project_a, item.id).await.len() >= 3).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(2000)).await; // give a (wrong) duplicate time to show up
    let acts = activity(&t, &f.guest_a, f.project_a, item.id).await;
    let mut verbs: Vec<_> = acts.iter().map(|a| a.verb.clone()).collect();
    verbs.sort();
    assert_eq!(verbs, ["created", "moved", "updated"], "the duplicated event must produce ONE activity row");
}

#[tokio::test]
async fn create_is_rate_limited_per_user() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let limit = Target::create_limit();
    let user = f.extra_member(&t).await;
    for i in 0..limit {
        let r = create(&t, &user, f.project_a, json!({"name": format!("r{i}")})).await;
        assert_eq!(r.status(), 201, "request {i} of {limit} must pass");
    }
    let r = create(&t, &user, f.project_a, json!({"name": "over"})).await;
    assert_eq!(r.status(), 429, "request {} must be limited", limit + 1);
    assert!(r.headers().contains_key("retry-after"));
    // control: the limit is per user, another member is unaffected
    let other = f.extra_member(&t).await;
    assert_eq!(create(&t, &other, f.project_a, json!({"name": "fine"})).await.status(), 201);
}

#[tokio::test]
async fn attachments_roundtrip_with_authz() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let item = create_ok(&t, &f.member_a, f.project_a, "with file").await;
    let path = format!("{}/{}/attachments/notes.txt", wi_path(f.project_a), item.id);
    let payload = b"hello r2 \x00\x01\x02 binary-safe".to_vec();

    // guest cannot write (control)
    let r = t.http.put(t.url(&path)).bearer_auth(&f.guest_a.token).body(payload.clone()).send().await.unwrap();
    assert_eq!(r.status(), 403);
    // member can
    let r = t
        .http
        .put(t.url(&path))
        .bearer_auth(&f.member_a.token)
        .header("content-type", "application/x-thrust")
        .body(payload.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    // guest can read, bytes and content-type survive
    let r = t.http.get(t.url(&path)).bearer_auth(&f.guest_a.token).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers().get("content-type").unwrap(), "application/x-thrust");
    assert_eq!(r.bytes().await.unwrap().to_vec(), payload);
    // unknown name -> 404 (control: the 200 above is not "always 200")
    let other = format!("{}/{}/attachments/missing.txt", wi_path(f.project_a), item.id);
    assert_eq!(t.http.get(t.url(&other)).bearer_auth(&f.guest_a.token).send().await.unwrap().status(), 404);
    // other tenant cannot read it
    assert_eq!(t.http.get(t.url(&path)).bearer_auth(&f.admin_b.token).send().await.unwrap().status(), 404);
    // path traversal in the name is rejected
    let evil = format!("{}/{}/attachments/..%2Fescape", wi_path(f.project_a), item.id);
    let st = t.http.put(t.url(&evil)).bearer_auth(&f.member_a.token).body("x").send().await.unwrap().status();
    assert!(st == 400 || st == 404, "traversal name must not be accepted, got {st}");
    // oversize is rejected before touching storage (limit is 5 MiB)
    let big = vec![7u8; 5 * 1024 * 1024 + 1];
    let r = t.http.put(t.url(&path)).bearer_auth(&f.member_a.token).body(big).send().await.unwrap();
    assert_eq!(r.status(), 413);
    // the oversize attempt did not clobber the stored object
    let r = t.http.get(t.url(&path)).bearer_auth(&f.guest_a.token).send().await.unwrap();
    assert_eq!(r.bytes().await.unwrap().to_vec(), payload);
    // delete, then it is gone
    let r = t.http.delete(t.url(&path)).bearer_auth(&f.member_a.token).send().await.unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(t.http.get(t.url(&path)).bearer_auth(&f.guest_a.token).send().await.unwrap().status(), 404);
}

#[tokio::test]
async fn invalid_input_is_a_client_error() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let r = create(&t, &f.member_a, f.project_a, json!({"name": "   "})).await;
    assert_eq!(r.status(), 400);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["error"]["code"], "invalid");
    let long = "x".repeat(256);
    assert_eq!(create(&t, &f.member_a, f.project_a, json!({"name": long})).await.status(), 400);
    // not a uuid in the path
    let r = t
        .http
        .post(t.url("/api/v1/projects/not-a-uuid/work-items"))
        .bearer_auth(&f.member_a.token)
        .json(&json!({"name":"x"}))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_client_error(), "got {}", r.status());
    // control: a 255-char name is accepted
    assert_eq!(create(&t, &f.member_a, f.project_a, json!({"name": "y".repeat(255)})).await.status(), 201);
}

/// Own pass (`panic_`): a panic under panic=abort resets the whole Worker isolate, which would
/// kill every other test running in parallel (measured: it did). Run alone, against a fresh
/// target.
///
/// `THRUST_STATE_SURVIVES_PANIC=0` is set only for targets whose state lives inside the isolate
/// (worker-memory); there losing state is the documented consequence. Everywhere else the
/// pre-panic row MUST still be there.
#[tokio::test]
async fn panic_is_contained_and_the_target_keeps_serving() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let before = create_ok(&t, &f.member_a, f.project_a, "before-panic").await;

    match t.http.get(t.url("/_test/panic")).send().await {
        Ok(r) => {
            assert!(r.status().is_server_error(), "panic route must answer 5xx, got {}", r.status());
            eprintln!("PANIC-OBSERVATION: the panicking request got HTTP {}", r.status());
        }
        Err(e) => eprintln!("PANIC-OBSERVATION: the panicking request got a transport error: {e}"),
    }

    // control: the target serves again (a Worker re-instantiates lazily on the next request)
    let mut ok = false;
    for _ in 0..10 {
        if let Ok(r) = t.http.get(t.url("/healthz")).send().await {
            if r.status() == 200 {
                ok = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(ok, "target did not recover after a panic");

    let survives = std::env::var("THRUST_STATE_SURVIVES_PANIC").map(|v| v != "0").unwrap_or(true);
    let r = list(&t, &f.member_a, f.project_a, None).await;
    if survives {
        assert_eq!(r.status(), 200, "durable state must survive a panic");
        let items: Vec<WorkItem> = r.json().await.unwrap();
        assert!(items.iter().any(|i| i.id == before.id), "row written before the panic is gone");
    } else {
        eprintln!(
            "PANIC-OBSERVATION: in-isolate state after the panic: HTTP {} (loss is expected for this target)",
            r.status()
        );
    }
}

/// Blast radius of a panic. 16 clients hammer an AUTHENTICATED list request (it awaits database /
/// KV I/O, so requests are genuinely suspended mid-flight on Postgres-backed targets) while ONE
/// request panics the handler; we count how many of the OTHER requests fail. Native (panic caught
/// per request): none. Worker (panic=abort, whole isolate reset): suspended requests can fail,
/// and on the in-isolate memory set all state is lost (401s afterwards).
/// The numbers are printed as observations; the assertion is that the target recovers.
#[tokio::test]
async fn blast_radius_of_one_panic() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    let t = Arc::new(t());
    let f = Arc::new(Fixture::seed(&t).await);
    let stop = Arc::new(AtomicBool::new(false));
    let ok = Arc::new(AtomicUsize::new(0));
    let bad = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let (t, f, stop, ok, bad) = (t.clone(), f.clone(), stop.clone(), ok.clone(), bad.clone());
        tasks.push(tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                let r = t
                    .http
                    .get(t.url(&wi_path(f.project_a)))
                    .bearer_auth(&f.member_a.token)
                    .send()
                    .await;
                match r {
                    Ok(r) if r.status() == 200 => ok.fetch_add(1, Ordering::Relaxed),
                    _ => bad.fetch_add(1, Ordering::Relaxed),
                };
            }
        }));
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = t.http.get(t.url("/_test/panic")).send().await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    stop.store(true, Ordering::Relaxed);
    for h in tasks {
        let _ = h.await;
    }
    eprintln!(
        "PANIC-OBSERVATION: concurrent authenticated list requests around one panic: ok={} failed={}",
        ok.load(Ordering::Relaxed),
        bad.load(Ordering::Relaxed)
    );
    assert!(ok.load(Ordering::Relaxed) > 100, "the load generator did not actually run");
    // control: the target answers afterwards
    let mut served = false;
    for _ in 0..10 {
        if let Ok(r) = t.http.get(t.url("/healthz")).send().await {
            if r.status() == 200 {
                served = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(served, "target did not recover");
}

/// Only run in the "sweeper" pass, against a target started with the outbox fast path OFF.
#[tokio::test]
async fn sweeper_publishes_rows_the_fast_path_did_not() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let before = t.outbox_depth().await;
    let item = create_ok(&t, &f.member_a, f.project_a, "swept").await;
    assert_eq!(t.outbox_depth().await, before + 1, "row must sit in the outbox");
    // control: with the fast path off nothing may have delivered the event yet
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        activity(&t, &f.guest_a, f.project_a, item.id).await.is_empty(),
        "an activity row appeared without the sweeper: the fast path is not actually off, this test would be vacuous"
    );
    // run the sweeper (what cron does)
    let r = t.admin_post("/internal/sweep", &json!({})).await;
    assert_eq!(r.status(), 200);
    let rep: Value = r.json().await.unwrap();
    assert!(rep["published"].as_u64().unwrap() >= 1, "sweeper published nothing: {rep}");
    let acts = eventually("activity after sweep", Duration::from_secs(15), || async {
        let a = activity(&t, &f.guest_a, f.project_a, item.id).await;
        (!a.is_empty()).then_some(a)
    })
    .await;
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0].verb, "created");
    assert_eq!(t.outbox_depth().await, 0, "swept rows must be marked dispatched");
}
