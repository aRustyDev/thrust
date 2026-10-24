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

// ---------------------------------------------------------------------------------------------------------------------
// Cell `worker-do` (spike 0002, risk R1): how ADR-0045's explicit `Tx` maps onto the object's SQLite.
// Run via `scripts/contract.sh worker-do do`. The same scenario runs hermetically on rusqlite in
// `crates/thrust-adapters-sqlite/tests/commit.rs`; here it runs on the REAL `SqlStorage` and `transactionSync`.
// ---------------------------------------------------------------------------------------------------------------------

async fn tx_control(t: &Target, mode: &str) -> Value {
    let r = t.admin_get(&format!("/_test/do/tx-control?mode={mode}")).await;
    assert_eq!(r.status(), 200, "tx-control probe ({mode}): {:?}", r.text().await);
    r.json().await.unwrap()
}

/// `Tx::commit` is one `transactionSync` in the object: a commit that fails at any statement, over every number of
/// creates, leaves the database exactly as it was, and the same transaction then commits.
#[tokio::test]
async fn worker_do_tx_commit_is_atomic_on_the_objects_sqlite() {
    let t = Target::from_env();
    let report = tx_control(&t, "atomic").await;
    assert_eq!(report["mode"], "atomic");
    assert_eq!(report["violations"], json!([]), "the commit is not atomic on the object's SqlStorage: {report}");
    // not vacuous: the scenario really failed statements. Of the 36 cases, the ones that name a statement the commit
    // issues that many times fire: the counter upsert and the delete once each, the item and outbox inserts once per
    // create, so 3 + 3 + 2 * (1 + 2 + 3) = 18. The other 18 are the "no fault fires, the commit applies" half.
    assert_eq!(report["cases"], 36);
    assert_eq!(report["fired"], 18, "the faults did not fire as often as the scenario issues statements: {report}");
}

/// The control for the control: the same scenario over a `Sql` whose `atomically` is no transaction at all (each
/// statement on its own) must be caught on the real runtime. If it were not, the test above could not tell a
/// transaction from a script.
#[tokio::test]
async fn worker_do_tx_naive_mapping_is_caught() {
    let t = Target::from_env();
    let report = tx_control(&t, "naive").await;
    assert_eq!(report["mode"], "naive");
    let violations = report["violations"].as_array().unwrap();
    assert!(!violations.is_empty(), "a commit with no transaction passed the atomicity control: {report}");
    assert!(
        violations.iter().all(|v| v.as_str().unwrap().contains("changed the database")),
        "the naive mapping failed, but not for the reason under test: {report}"
    );
    // control: no probe without the admin token (the probe is a test route)
    let r = t.http.get(t.url("/_test/do/tx-control?mode=atomic")).send().await.unwrap();
    assert_eq!(r.status(), 404, "the probe answered without the admin token");
}

/// Spike 0002 risk R3, asked of the real runtime: does the SQLite inside a Durable Object have FTS5 with the `trigram`
/// tokenizer, which is what substring search (`icontains` parity, ADR-0083) needs? The probe builds a scratch FTS5 table
/// and drops it again. The findings are printed whatever they are (`FTS5-OBSERVATION`), so a build of workerd without the
/// tokenizer is reported as a finding with its SQLite version, and fails this test.
#[tokio::test]
async fn worker_do_fts5_trigram_tokenizer_is_available() {
    let t = Target::from_env();
    let r = t.admin_get("/_test/do/fts-probe").await;
    assert_eq!(r.status(), 200, "fts-probe: {:?}", r.text().await);
    let report: Value = r.json().await.unwrap();
    eprintln!("FTS5-OBSERVATION: {report}");
    let tri = &report["fts5_trigram"];
    assert_eq!(tri["created"], true, "no FTS5 `trigram` tokenizer in this SQLite: {report}");
    assert_eq!(tri["inserted"], true, "{report}");
    // a substring, in the middle of a word, with the other case, across a space: what `icontains` finds
    assert_eq!(tri["substring_world"], json!(["Hello World"]), "{report}");
    assert_eq!(tri["substring_inside_word"], json!(["needle in a haystack"]), "{report}");
    assert_eq!(tri["phrase_with_spaces"], json!(["needle in a haystack"]), "{report}");
    // non-ASCII case folding, as `icontains` does it
    assert_eq!(tri["non_ascii_case_fold"], json!(["\u{c9}COLE normale"]), "{report}");
    // controls: a query in no row finds none, so the hits above are not "everything matches"; and a 2-character
    // query matches NOTHING through the index (a trigram needs 3), which is why the adapter falls back to LIKE for
    // short queries, and LIKE finds it (with `%` and `_` escaped)
    assert_eq!(tri["absent"], json!([]), "{report}");
    assert_eq!(tri["two_characters_match"], json!([]), "{report}");
    assert_eq!(tri["two_characters_like"], json!(["Hello World"]), "{report}");
    assert_eq!(tri["percent_escaped_like"], json!(["50% off_sale"]), "{report}");
    // the probe is repeatable: it drops its scratch table
    let again: Value = t.admin_get("/_test/do/fts-probe").await.json().await.unwrap();
    assert_eq!(again["fts5_trigram"]["created"], true);
}

/// The cost probe (`/_test/do/cost-probe`) reports the rows Durable Object SQLite bills by. A probe that reported zeros
/// would make the cost projection look free, so this checks it counts what must be counted: a create writes more than one
/// row (the table row, its indexes and its outbox row), a bulk-seeded item costs more than a bare row because the FTS5
/// trigram index is written too, reads write nothing, and a 2-character query scans the project.
#[tokio::test]
async fn worker_do_cost_probe_counts_the_rows_that_are_billed() {
    let t = Target::from_env();
    let r = t.admin_get("/_test/do/cost-probe").await;
    assert_eq!(r.status(), 200, "cost-probe: {:?}", r.text().await);
    let c: Value = r.json().await.unwrap();
    eprintln!("COST-OBSERVATION: {c}");
    let f = |v: &Value, k: &str| v[k].as_f64().unwrap_or_else(|| panic!("no {k} in {v}"));
    assert!(f(&c["create_use_case"], "rows_written_per_op") >= 4.0, "a create wrote fewer rows than table + indexes + outbox: {c}");
    assert!(f(&c["bulk_seed_per_item"], "rows_written_per_op") > 2.0, "{c}");
    assert!(f(&c["relay_mark_dispatched"], "rows_written_per_op") >= 1.0, "{c}");
    assert!(f(&c["consumer_activity_insert"], "rows_written_per_op") >= 1.0, "{c}");
    // reads write nothing (control for "the counter is just counting everything as written")
    assert_eq!(f(&c["list_of_live_items"]["cost"], "rows_written_per_op"), 0.0, "{c}");
    assert_eq!(f(&c["search"]["indexed_common_word_Fix"], "rows_written_per_op"), 0.0, "{c}");
    // A 2-character query scans the project (a trigram needs three characters), so it reads about as many rows as the
    // project holds. A 3-character query is answered through the FTS5 index (`a_three_character_query_uses_the_trigram_index`
    // shows the plan), but SQLite's planner reads the project's rows through `work_items_list` and uses the index's matches
    // as a membership filter, so rows read stay near the project's size for a miss too. That is reported
    // (`COST-OBSERVATION`), not asserted: the cost projection uses it, and it is O(project), not O(matches).
    let items = c["list_of_live_items"]["items"].as_f64().unwrap();
    assert!(f(&c["search"]["scan_two_characters_ou"], "rows_read_per_op") >= items / 2.0, "a 2-character query did not scan: {c}");
    assert!(f(&c["search"]["indexed_phrase_miss"], "rows_read_per_op") > 0.0, "{c}");
    assert!(c["database_bytes_after"].as_i64().unwrap() > c["database_bytes_before"].as_i64().unwrap(), "{c}");
}

/// The migration story's phases on the real runtime (`/_test/do/migration-story`), and its failure case: the real runner
/// given a migration whose third statement fails changes nothing (no table, no ledger row), and the object goes on serving.
#[tokio::test]
async fn worker_do_migration_story_phases_run_and_a_failed_migration_changes_nothing() {
    let t = Target::from_env();
    let phase = |p: &str| {
        let t = &t;
        let p = p.to_string();
        async move {
            let r = t.admin_get(&format!("/_test/do/migration-story?phase={p}")).await;
            assert_eq!(r.status(), 200, "phase {p}");
            r.json::<Value>().await.unwrap()
        }
    };
    // seed a few rows so the copy has something in it
    let f = Fixture::seed(&t).await;
    seed_some(&t, &f).await;
    let mut copied = 0i64;
    for p in ["copy", "expand", "fts", "contract"] {
        let v = phase(p).await;
        assert_eq!(v["result"]["ok"], true, "phase {p} failed: {v}");
        match p {
            "copy" => {
                copied = v["rows"].as_i64().unwrap();
                assert!(copied >= 3, "{v}");
            }
            // the expand migration backfilled every row of the copy
            "expand" => assert_eq!(v["rows_with_estimate"], copied, "{v}"),
            "contract" => assert_eq!(v["estimate_column_left"], 0, "the contract migration left the column: {v}"),
            _ => {}
        }
    }
    let failed = phase("fail").await;
    assert_eq!(failed["migrate_failed"], true, "the broken migration succeeded: {failed}");
    assert!(failed["error"].as_str().unwrap().contains("no_such_table"), "{failed}");
    assert_eq!(failed["scratch_table_left_behind"], 0, "two statements of the failed migration survived: {failed}");
    assert_eq!(failed["ledger_rows_for_99"], 0, "{failed}");
    // control: the real migrations are all recorded, and the object still answers
    assert_eq!(failed["ledger_versions"], json!([1, 6]), "{failed}");
    assert_eq!(t.http.get(t.url("/healthz")).send().await.unwrap().status(), 200);
    assert_eq!(list(&t, &f.admin_a, &f.a, None).await.status(), 200, "the object stopped serving after the failed migration");
    assert_eq!(phase("cleanup").await["result"]["ok"], true);
}

async fn seed_some(t: &Target, f: &Fixture) {
    for n in ["one", "two", "three"] {
        create_ok(t, &f.member_a, &f.a, n).await;
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The one real email send of spike 0002 (step 10). Run via `scripts/stage2-contract.sh email` against a DEPLOYED Worker
// that has the `EMAIL` binding and the vars `THRUST_EMAIL_FROM` (an owned sender domain with Email Sending enabled) and
// `THRUST_EMAIL_TO` (a verified destination). The owner approved exactly one send, so this test sends once and a second run
// would send a second mail: run it once. The existing `worker_bindings_email_send_binding_simulated_locally` is simulated and
// skipped when deployed.
// ---------------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn stage2_email_real_send_once() {
    let t = Target::from_env();
    // control first, and it sends nothing: a sender domain the account does not own is refused by the deployed binding. If it
    // were accepted the real send below would say nothing about domain ownership.
    let r = t.admin_get("/_test/bindings/email-refused").await;
    let status = r.status();
    let body: Value = r.json().await.unwrap();
    assert_eq!(status, 502, "an unowned sender domain was not refused: {body}");
    let err = body["error"].as_str().unwrap_or_default();
    assert!(err.contains("not allowed") || err.contains("not owned"), "refused, but not for ownership: {body}");
    // the real send
    let r = t.admin_get("/_test/bindings/email-real").await;
    let status = r.status();
    let body: Value = r.json().await.unwrap();
    eprintln!("EMAIL-OBSERVATION: {body}");
    assert_eq!(status, 200, "the real send failed: {body}");
    assert_eq!(body["sent"], true);
    assert!(!body["message_id"].as_str().unwrap_or_default().is_empty(), "no message id: {body}");
}
