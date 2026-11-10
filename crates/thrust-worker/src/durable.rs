//! Durable Object classes: `RoomDo` only, the template of the P1 collab Durable Object (FR-05, issue #43). The rate limiter that
//! used to be here is the Workers Rate Limiting binding now (ADR-0033). ARCHITECTURE 2a reserves `thrust-collab-do` as the only
//! crate that exports DO classes; the spike keeps them here because the wasm module that exports the classes must be the same
//! module as the `fetch` handler.

use serde::Deserialize;
use serde_json::json;
use worker::worker_sys::web_sys;
use worker::*;

use crate::text;

#[derive(Deserialize)]
struct NRow {
    n: i64,
}

// ---------------------------------------------------------------------------------------------
// RoomDo: WebSocket hibernation API + SQLite storage (the shape of the P1 collab DO, ADR-0060)
// ---------------------------------------------------------------------------------------------

#[durable_object]
pub struct RoomDo {
    state: State,
    /// Changes every time the runtime re-instantiates this object (e.g. after hibernation).
    instance: u64,
}

impl DurableObject for RoomDo {
    fn new(state: State, _env: Env) -> Self {
        let _ = state.storage().sql().exec(
            "CREATE TABLE IF NOT EXISTS messages (id INTEGER PRIMARY KEY AUTOINCREMENT, body TEXT NOT NULL)",
            None,
        );
        Self { state, instance: Date::now().as_millis() }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        let path = req.path();
        if path.ends_with("/history") {
            let rows: Vec<serde_json::Value> =
                self.state.storage().sql().exec("SELECT id, body FROM messages ORDER BY id", None)?.to_array()?;
            return Response::from_json(&json!({ "instance": self.instance, "messages": rows }));
        }
        if req.headers().get("Upgrade")?.as_deref() != Some("websocket") {
            return Response::error("expected a websocket upgrade", 426);
        }
        let pair = WebSocketPair::new()?;
        // Hibernation API: the runtime owns the socket; this object may be evicted while it is open
        // and is re-created (`new`) when the next message arrives.
        self.state.accept_web_socket(&pair.server);
        let conn = self.state.get_websockets().len() as u64;
        pair.server.serialize_attachment(json!({ "conn": conn }))?;
        Response::from_websocket(pair.client)
    }

    async fn websocket_message(&self, ws: WebSocket, message: WebSocketIncomingMessage) -> Result<()> {
        let WebSocketIncomingMessage::String(body) = message else {
            return Ok(());
        };
        let sql = self.state.storage().sql();
        sql.exec("INSERT INTO messages (body) VALUES (?)", vec![body.clone().into()])?;
        let n: NRow = sql.exec("SELECT count(*) AS n FROM messages", None)?.one()?;
        let conn = ws
            .deserialize_attachment::<serde_json::Value>()?
            .and_then(|v| v.get("conn").cloned())
            .unwrap_or(serde_json::Value::Null);
        let reply = json!({ "echo": body, "seq": n.n, "conn": conn, "instance": self.instance }).to_string();
        for peer in self.state.get_websockets() {
            peer.send_with_str(&reply)?;
        }
        Ok(())
    }

    async fn websocket_close(&self, ws: WebSocket, code: usize, reason: String, _was_clean: bool) -> Result<()> {
        // 1005/1006/1015 are reserved ("no status", "abnormal", "TLS failure"): they are reported
        // to us when the peer vanished but may never be put on the wire (measured: echoing 1006
        // throws InvalidAccessError). Answer with a normal closure instead.
        let code = if (1000..=1003).contains(&code) || (1007..=1014).contains(&code) || (3000..=4999).contains(&code) {
            code as u16
        } else {
            1000
        };
        let _ = ws.close(Some(code), Some(reason));
        Ok(())
    }

    async fn websocket_error(&self, _ws: WebSocket, error: Error) -> Result<()> {
        console_error!("room websocket error: {error}");
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Test route: /_test/room/{name}/{ws|history}  (needs test routes ON and the admin token)
// ---------------------------------------------------------------------------------------------

pub async fn room_route(req: web_sys::Request, env: &Env, path: &str) -> Result<web_sys::Response> {
    let test_routes = matches!(text(env, "THRUST_TEST_ROUTES").as_deref(), Some("1") | Some("true") | Some("on"));
    let given = req.headers().get("x-admin-token").ok().flatten();
    let want = text(env, "THRUST_ADMIN_TOKEN");
    let ok = test_routes
        && match (want, given) {
            (Some(w), Some(g)) => {
                w.len() == g.len() && w.bytes().zip(g.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
            }
            _ => false,
        };
    if !ok {
        let r: Response = Response::error("not found", 404)?;
        return Ok(r.into());
    }
    let name = path.trim_start_matches("/_test/room/").split('/').next().unwrap_or("default");
    let stub = env.durable_object("ROOM")?.get_by_name(name)?;
    let resp = stub.fetch_with_request(Request::from(req)).await?;
    Ok(resp.into())
}
