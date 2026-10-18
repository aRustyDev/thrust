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
    let url = t.base.replace("http://", "ws://");
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

    // control: no admin token -> the upgrade is refused (the route is not open)
    assert!(ws_connect(&t, &room, None).await.is_err(), "unauthenticated websocket upgrade must fail");
    assert!(ws_connect(&t, &room, Some("wrong")).await.is_err(), "wrong token must fail");

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
    let item = create_ok(&t, &f.member_a, f.project_a, "cron-swept").await;
    assert_eq!(t.outbox_depth().await, before + 1);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        activity(&t, &f.guest_a, f.project_a, item.id).await.is_empty(),
        "activity exists before the cron ran: fast path is not off, test would be vacuous"
    );

    let r = t.http.get(t.url("/__scheduled?cron=*+*+*+*+*")).send().await.unwrap();
    assert_eq!(r.status(), 200, "scheduled endpoint");

    let acts = eventually("activity after the scheduled handler", Duration::from_secs(20), || async {
        let a = activity(&t, &f.guest_a, f.project_a, item.id).await;
        (!a.is_empty()).then_some(a)
    })
    .await;
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0].verb, "created");
    assert_eq!(t.outbox_depth().await, 0, "the scheduled sweep must mark rows dispatched");
}
