//! Worker-only checks: things that exist ONLY on the Cloudflare target (Durable Object WebSocket
//! hibernation + SQLite storage, the `scheduled` handler). The native twin of these (Hocuspocus,
//! in-process scheduler) is out of scope for the spike. Run via `scripts/contract.sh worker-cf worker`
//! and `... cron`.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use thrust_contract_tests::*;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn ws_connect(t: &Target, room: &str, token: Option<&str>) -> Result<Ws, String> {
    // https -> wss (a deployed Worker), http -> ws (local workerd). Replacing only `http://` left an
    // `https://` base as-is and tungstenite refused the scheme (found in Stage 2).
    let url = t.base.replace("https://", "wss://").replace("http://", "ws://");
    let mut req = format!("{url}/_test/room/{room}/ws").into_client_request().unwrap();
    if let Some(tok) = token {
        req.headers_mut().insert("x-admin-token", tok.parse().unwrap());
    }
    tokio_tungstenite::connect_async(req).await.map(|(s, _)| s).map_err(|e| e.to_string())
}

async fn next_json(ws: &mut Ws) -> Value {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("timed out waiting for a websocket message")
            .expect("socket closed")
            .expect("socket error");
        if let Message::Text(t) = m {
            return serde_json::from_str(t.as_str()).unwrap();
        }
    }
}

#[tokio::test]
async fn worker_room_do_websocket_hibernation_api_and_sqlite_storage() {
    let t = Target::from_env();
    let room = format!("room-{}", uuid::Uuid::new_v4().simple());

    // control: no admin token -> the upgrade is refused (the route is not open). The refusal must be the
    // Worker's own HTTP 4xx answer: "any error" would also pass on a bad URL scheme or a TLS failure,
    // and a must-fail control that passes for the wrong reason is no control (Stage 2 hit exactly that).
    for (what, token) in [("unauthenticated", None), ("wrong token", Some("wrong"))] {
        match ws_connect(&t, &room, token).await {
            Ok(_) => panic!("{what} websocket upgrade must fail"),
            Err(e) => assert!(e.starts_with("HTTP error: 4"), "{what}: refusal must be an HTTP 4xx from the Worker, got: {e}"),
        }
    }

    let mut a = ws_connect(&t, &room, Some(&t.admin_token)).await.expect("connect a");
    a.send(Message::text("hello")).await.unwrap();
    let r1 = next_json(&mut a).await;
    assert_eq!(r1["echo"], "hello");
    assert_eq!(r1["seq"], 1);

    // a second socket on the SAME room object receives broadcasts from the first
    let mut b = ws_connect(&t, &room, Some(&t.admin_token)).await.expect("connect b");
    b.send(Message::text("from-b")).await.unwrap();
    let rb = next_json(&mut b).await;
    let ra = next_json(&mut a).await;
    assert_eq!(rb["seq"], 2);
    assert_eq!(ra["echo"], "from-b", "broadcast must reach the other socket");
    // the reply carries the SENDER's attachment, so a's first reply and b's reply must differ
    assert_ne!(r1["conn"], rb["conn"], "per-socket attachments must differ");

    // another room is isolated (control)
    let other = format!("room-{}", uuid::Uuid::new_v4().simple());
    let mut c = ws_connect(&t, &other, Some(&t.admin_token)).await.expect("connect c");
    c.send(Message::text("solo")).await.unwrap();
    assert_eq!(next_json(&mut c).await["seq"], 1, "a different room starts its own SQLite sequence");

    // storage: both messages are in the DO's SQLite database
    let hist: Value = t
        .http
        .get(t.url(&format!("/_test/room/{room}/history")))
        .header("x-admin-token", &t.admin_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let bodies: Vec<_> = hist["messages"].as_array().unwrap().iter().map(|m| m["body"].as_str().unwrap().to_string()).collect();
    assert_eq!(bodies, ["hello", "from-b"]);

    // Hibernation: workerd evicts an idle hibernatable DO after ~10 s; the socket stays open and the
    // object is re-created on the next message. `instance` changes iff that happened.
    let instance_before = r1["instance"].clone();
    let conn_before = r1["conn"].clone();
    tokio::time::sleep(Duration::from_secs(14)).await;
    a.send(Message::text("after-idle")).await.unwrap();
    let r3 = next_json(&mut a).await;
    assert_eq!(r3["seq"], 3, "SQLite state must survive an idle period");
    assert_eq!(r3["conn"], conn_before, "the socket attachment must survive an idle period");
    let hibernated = r3["instance"] != instance_before;
    eprintln!(
        "HIBERNATION-OBSERVATION: object instance before={instance_before} after={}; re-instantiated (hibernated)={hibernated}",
        r3["instance"]
    );
}

/// Pass `cron` (fast path OFF). Hits wrangler's `--test-scheduled` endpoint, which invokes the
/// Worker's `scheduled` handler exactly like a cron trigger would. The assertion that nothing
/// delivered the event beforehand is what keeps this from being vacuous.
#[tokio::test]
async fn worker_cron_scheduled_handler_runs_the_sweeper() {
    let t = Target::from_env();
    let f = Fixture::seed(&t).await;
    let before = t.outbox_depth().await;
    let item = create_ok(&t, &f.member_a, &f.a, "cron-swept").await;
    assert_eq!(t.outbox_depth().await, before + 1);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        activity(&t, &f.guest_a, &f.a, item.id).await.is_empty(),
        "activity exists before the cron ran: fast path is not off, test would be vacuous"
    );

    let r = t.http.get(t.url("/__scheduled?cron=*+*+*+*+*")).send().await.unwrap();
    assert_eq!(r.status(), 200, "scheduled endpoint");

    let acts = eventually("activity after the scheduled handler", Duration::from_secs(20), || async {
        let a = activity(&t, &f.guest_a, &f.a, item.id).await;
        (!a.is_empty()).then_some(a)
    })
    .await;
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0].verb, "created");
    assert_eq!(t.outbox_depth().await, 0, "the scheduled sweep must mark rows dispatched");
}

// ---------------------------------------------------------------------------------------------
// Binding probes (pass `bindings`): one REAL operation per binding, through the Rust API where
// workers-rs has a wrapper and through raw wasm-bindgen where it does not (Workflows).
// ---------------------------------------------------------------------------------------------

async fn probe(t: &Target, name: &str) -> (u16, Value) {
    let r = t
        .http
        .get(t.url(&format!("/_test/bindings/{name}")))
        .header("x-admin-token", &t.admin_token)
        .send()
        .await
        .unwrap();
    let st = r.status().as_u16();
    (st, r.json::<Value>().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn worker_bindings_probe_routes_require_the_admin_token() {
    let t = Target::from_env();
    let r = t.http.get(t.url("/_test/bindings/ratelimit")).send().await.unwrap();
    assert_eq!(r.status(), 404, "probe routes must not be open");
    let r = t.http.get(t.url("/_test/bindings/ratelimit")).header("x-admin-token", "wrong").send().await.unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn worker_bindings_rate_limiting_binding() {
    let t = Target::from_env();
    let (st, v) = probe(&t, "ratelimit").await;
    assert_eq!(st, 200, "{v}");
    // limit = 3 per 10 s. What must hold on EVERY target: the first three calls are allowed (it never
    // limits early) and a refusal appears (the must-fail control: it does limit). Local workerd is exact
    // (three allowed, then refused); the deployed binding is approximate and was measured allowing a
    // fourth call, so the exact `[t,t,t,f,f]` shape is NOT asserted. The observed outcomes go in the message.
    let o: Vec<bool> = v["outcomes"].as_array().expect("outcomes").iter().map(|b| b.as_bool().unwrap()).collect();
    assert!(o.len() >= 8, "probe must make enough calls to see a refusal: {v}");
    assert_eq!(&o[..3], &[true, true, true], "limit 3: the first three calls must be allowed: {v}");
    assert!(o.contains(&false), "limit 3 per 10 s: some call must be refused: {v}");
    println!("RATELIMIT-OBSERVATION allowed_before_first_refusal={}", o.iter().position(|a| !a).unwrap());
}

#[tokio::test]
async fn worker_bindings_secrets_store_binding() {
    let t = Target::from_env();
    let (st, v) = probe(&t, "secret").await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["present"], true);
    // Local workerd: the dummy created by scripts/worker-dev.sh (28 chars). Deployed: the length of the secret
    // actually created in the store, passed as THRUST_EXPECT_SECRET_LEN. Matching a length that differs from
    // the local dummy's is what shows the REAL store was read.
    let want: usize = std::env::var("THRUST_EXPECT_SECRET_LEN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or("local-dev-dummy-not-a-secret".len());
    assert_eq!(v["len"], want);
    assert!(!v.to_string().contains("dummy"), "the probe must never echo the secret value");
}

#[tokio::test]
async fn worker_bindings_email_send_binding_simulated_locally() {
    let t = Target::from_env();
    let (st, v) = probe(&t, "email").await;
    assert_eq!(st, 200, "{v}");
    assert!(v["message_id_len"].as_u64().unwrap() > 0, "{v}");
}

#[tokio::test]
async fn worker_bindings_service_binding() {
    let t = Target::from_env();
    let (st, v) = probe(&t, "service").await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["status"], 200);
}

#[tokio::test]
async fn worker_bindings_workflow_via_wasm_bindgen_and_js_shim() {
    let t = Target::from_env();
    let (st, v) = probe(&t, "workflow").await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["status"], "complete", "{v}");
    assert_eq!(v["output"]["doubled"], 42, "the workflow's step.do ran: {v}");
    assert_eq!(v["output"]["health"]["status"], 200, "workflow -> service binding -> Rust Worker: {v}");
    assert_eq!(v["output"]["health"]["body"], "ok");
}

/// Pass `crontrigger` of `scripts/stage2-contract.sh` (fast path OFF, DEPLOYED Worker only). The `stage2_` prefix is
/// load-bearing: the local `cron` pass filters on `worker_cron_`, and a local `wrangler dev` never fires a cron by
/// itself, so a `worker_cron_*` name would make that pass wait 180 s and fail. Stage 2 twin of the `cron` test: a deployed
/// Worker has no `/__scheduled` endpoint (that is `wrangler dev` only), so this creates an item and waits
/// for the REAL cron trigger (`* * * * *`) to run the sweeper, calling nothing that sweeps. The empty-activity
/// check two seconds after the create is what keeps it from being vacuous (the fast path must really be off).
#[tokio::test]
async fn stage2_cron_real_trigger_runs_the_sweeper() {
    let t = Target::from_env();
    let f = Fixture::seed(&t).await;
    let item = create_ok(&t, &f.member_a, &f.a, "cron-real-trigger").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        activity(&t, &f.guest_a, &f.a, item.id).await.is_empty(),
        "activity exists before any cron ran: fast path is not off, test would be vacuous"
    );
    let start = std::time::Instant::now();
    let acts = loop {
        let a = activity(&t, &f.guest_a, &f.a, item.id).await;
        if !a.is_empty() {
            break a;
        }
        assert!(start.elapsed() < Duration::from_secs(180), "no delivery within 180 s: the cron trigger did not sweep");
        tokio::time::sleep(Duration::from_secs(1)).await;
    };
    eprintln!("CRON-OBSERVATION: delivered {} s after the 2 s empty check (cron `* * * * *`)", start.elapsed().as_secs());
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0].verb, "created");
}
