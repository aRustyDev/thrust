//! Binding probes (test routes). Each one performs the smallest REAL operation through the
//! workers-rs API (or raw wasm-bindgen where workers-rs has no wrapper) so the binding-coverage
//! matrix can say `[tested]` instead of guessing. Gated by test routes + the admin token.
//!
//! Route: GET /_test/bindings/{ratelimit|secret|email|service|workflow|ai}

use serde_json::{json, Value};
use worker::worker_sys::web_sys;
use worker::*;

use crate::text;

fn ok_json(v: Value) -> Result<web_sys::Response> {
    let r: Response = Response::from_json(&v)?;
    Ok(r.into())
}

pub(crate) fn admin_ok(req: &web_sys::Request, env: &Env) -> bool {
    let test_routes = matches!(text(env, "THRUST_TEST_ROUTES").as_deref(), Some("1") | Some("true") | Some("on"));
    let given = req.headers().get("x-admin-token").ok().flatten();
    match (test_routes, text(env, "THRUST_ADMIN_TOKEN"), given) {
        (true, Some(w), Some(g)) => w.len() == g.len() && w.bytes().zip(g.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0,
        _ => false,
    }
}

pub async fn probe_route(req: web_sys::Request, env: &Env, path: &str) -> Result<web_sys::Response> {
    if !admin_ok(&req, env) {
        let r: Response = Response::error("not found", 404)?;
        return Ok(r.into());
    }
    let name = path.trim_start_matches("/_test/bindings/");
    let out = match name {
        "ratelimit" => ratelimit(env).await,
        "secret" => secret(env).await,
        "email" => email(env).await,
        "service" => service(env).await,
        "workflow" => workflow(env).await,
        "ai" => ai(env).await,
        "pgshapes" => pgshapes(env).await,
        "r2purge" => r2purge(env).await,
        _ => Err(Error::RustError(format!("unknown probe '{name}'"))),
    };
    match out {
        Ok(v) => ok_json(v),
        Err(e) => {
            let r: Response = Response::from_json(&json!({ "probe": name, "error": e.to_string() }))?.with_status(502);
            Ok(r.into())
        }
    }
}

/// Rate Limiting binding (`[[ratelimits]]`, limit 3 per 10 s): native in workers-rs.
async fn ratelimit(env: &Env) -> Result<Value> {
    let rl = env.rate_limiter("RL")?;
    let key = format!("probe-{}", Date::now().as_millis());
    let mut outcomes = Vec::new();
    // 10 calls, not 5: local workerd refuses exactly after the limit, but the deployed binding is approximate
    // (measured: it allowed a 4th call), so give a lazily-applied limit room to show itself.
    for _ in 0..10 {
        outcomes.push(rl.limit(key.clone()).await?.success);
    }
    Ok(json!({ "outcomes": outcomes }))
}

/// Secrets Store binding: native in workers-rs. Never returns the value, only its length.
async fn secret(env: &Env) -> Result<Value> {
    let s = env.secret_store("SS")?;
    let v = s.get().await?;
    Ok(json!({ "present": v.is_some(), "len": v.map(|x| x.len()) }))
}

/// Email Service `send_email` binding: native in workers-rs (`worker::SendEmail`). Under
/// `wrangler dev` without `remote = true` the binding is simulated and nothing is sent.
async fn email(env: &Env) -> Result<Value> {
    let sender = env.send_email("EMAIL")?;
    let msg = worker::SendEmailBuilder::builder("noreply@example.com", "someone@example.com", "spike 0001 probe")
        .text("probe body")
        .build();
    let res = sender
        .send_with_builder(&msg)
        .await
        .map_err(|e| Error::RustError(format!("{e:?}")))?;
    Ok(json!({ "message_id_len": res.message_id().len() }))
}

/// Service binding to this same Worker (`[[services]] SELF`): native in workers-rs (`Fetcher`).
async fn service(env: &Env) -> Result<Value> {
    let f = env.service("SELF")?;
    let resp = f.fetch("https://self.internal/healthz", None).await?;
    Ok(json!({ "status": resp.status().as_u16() }))
}

/// Workflows: workers-rs 0.8.7 has NO Workflow wrapper and no way to define a Workflow class
/// (`WorkflowEntrypoint` is a JS class). This calls the binding through raw `js_sys::Reflect`
/// (the "wasm-bindgen" route); the Workflow class itself lives in the JS shim `entry.mjs`.
async fn workflow(env: &Env) -> Result<Value> {
    use worker::js_sys::{Function, Object, Promise, Reflect};
    use worker::wasm_bindgen::{JsCast, JsValue};
    use worker::wasm_bindgen_futures::JsFuture;

    let js_err = |e: JsValue| Error::RustError(format!("{e:?}"));
    let binding = Reflect::get(env, &JsValue::from("WORKFLOW")).map_err(js_err)?;
    if binding.is_undefined() {
        return Err(Error::RustError("WORKFLOW binding is undefined".into()));
    }
    let call = |target: &JsValue, method: &str, arg: Option<&JsValue>| -> std::result::Result<JsValue, JsValue> {
        let f: Function = Reflect::get(target, &JsValue::from(method))?.dyn_into()?;
        match arg {
            Some(a) => f.call1(target, a),
            None => f.call0(target),
        }
    };

    let opts = Object::new();
    let params = Object::new();
    Reflect::set(&params, &"n".into(), &JsValue::from(21)).map_err(js_err)?;
    Reflect::set(&opts, &"params".into(), &params).map_err(js_err)?;
    let instance = JsFuture::from(Promise::from(call(&binding, "create", Some(&opts)).map_err(js_err)?))
        .await
        .map_err(js_err)?;
    let id = Reflect::get(&instance, &"id".into()).map_err(js_err)?.as_string().unwrap_or_default();

    // poll status() until the instance completes (it is durable and runs in the background)
    let mut last = JsValue::UNDEFINED;
    for _ in 0..40 {
        let st = JsFuture::from(Promise::from(call(&instance, "status", None).map_err(js_err)?))
            .await
            .map_err(js_err)?;
        let status = Reflect::get(&st, &"status".into()).map_err(js_err)?.as_string().unwrap_or_default();
        last = st;
        if status == "complete" || status == "errored" || status == "terminated" {
            break;
        }
        worker::Delay::from(std::time::Duration::from_millis(250)).await;
    }
    let status = Reflect::get(&last, &"status".into()).map_err(js_err)?.as_string().unwrap_or_default();
    let output = Reflect::get(&last, &"output".into())
        .ok()
        .and_then(|o| serde_wasm_bindgen::from_value::<Value>(o).ok())
        .unwrap_or(Value::Null);
    Ok(json!({ "id_len": id.len(), "status": status, "output": output }))
}

/// Workers AI / AI Gateway binding: native in workers-rs (`worker::Ai`). Compile-checked only:
/// `wrangler dev --local` cannot run it (the AI binding always reaches the remote service).
#[allow(dead_code)]
async fn ai(env: &Env) -> Result<Value> {
    let ai = env.ai("AI")?;
    let out: Value = ai.run("@cf/meta/llama-3.1-8b-instruct", json!({ "prompt": "ping" })).await?;
    Ok(out)
}

/// Stage 2 teardown helper: empty the spike's R2 bucket through the binding. An R2 bucket must be empty before
/// it can be deleted, and wrangler has neither object listing nor a bulk delete. Admin-gated like every probe.
async fn r2purge(env: &Env) -> Result<Value> {
    let bucket = env.bucket("BLOBS")?;
    let mut deleted = 0u64;
    loop {
        let page = bucket.list().limit(1000).execute().await?;
        let keys: Vec<String> = page.objects().iter().map(|o| o.key()).collect();
        if keys.is_empty() {
            break;
        }
        for key in &keys {
            bucket.delete(key.clone()).await?;
        }
        deleted += keys.len() as u64;
    }
    Ok(json!({ "deleted": deleted }))
}

/// Stage 2 diagnostic: which Postgres wire-protocol SHAPES does the Rust driver get through Hyperdrive?
/// Every case opens its own fresh connection (so one failure cannot poison the next) and reports the
/// driver's error verbatim (`Debug` shows tokio-postgres' `kind` and `cause`). Writes are rolled back.
/// Only meaningful with the Cloudflare adapter set; the schema is whatever the role's search_path points at.
async fn pgshapes(env: &Env) -> Result<Value> {
    use thrust_adapters_cloudflare::HyperdriveConnector;
    use thrust_adapters_postgres::{ClientHandle, Connector};

    async fn conn(env: &Env) -> std::result::Result<Box<dyn ClientHandle>, String> {
        let hd = env.hyperdrive("HYPERDRIVE").map_err(|e| e.to_string())?;
        HyperdriveConnector::new(hd).checkout(true).await.map_err(|e| format!("{e:?}"))
    }

    macro_rules! case {
        ($res:ident, $name:expr, |$cl:ident| $body:block) => {{
            let r: std::result::Result<String, String> = async {
                let h = conn(env).await?;
                let $cl = h.client();
                $body
            }
            .await;
            $res.insert(
                $name.to_string(),
                match r {
                    Ok(s) => json!({ "ok": s }),
                    Err(e) => json!({ "error": e }),
                },
            );
        }};
    }
    fn d<E: std::fmt::Debug>(e: E) -> String {
        format!("{e:?}")
    }
    let mut res = serde_json::Map::new();

    case!(res, "1_simple_select", |cl| {
        cl.simple_query("SELECT 1").await.map_err(d)?;
        Ok("simple_query".to_string())
    });
    case!(res, "2_ext_noparam", |cl| {
        let r = cl.query_one("SELECT 1::int8 AS n", &[]).await.map_err(d)?;
        Ok(format!("n={}", r.get::<_, i64>("n")))
    });
    case!(res, "3_ext_param", |cl| {
        let r = cl.query_one("SELECT $1::int8 AS n", &[&41i64]).await.map_err(d)?;
        Ok(format!("n={}", r.get::<_, i64>("n")))
    });
    case!(res, "4_two_ext_param_same_conn_no_tx", |cl| {
        let a = cl.query_one("SELECT $1::int8 AS n", &[&1i64]).await.map_err(d)?;
        let b = cl.query_one("SELECT $1::int8 AS n", &[&2i64]).await.map_err(d)?;
        Ok(format!("{}+{}", a.get::<_, i64>("n"), b.get::<_, i64>("n")))
    });
    case!(res, "5_tx_simple", |cl| {
        cl.batch_execute("BEGIN").await.map_err(d)?;
        cl.simple_query("SELECT 1").await.map_err(d)?;
        cl.batch_execute("COMMIT").await.map_err(d)?;
        Ok("begin/simple/commit".to_string())
    });
    case!(res, "6_tx_ext_noparam", |cl| {
        cl.batch_execute("BEGIN").await.map_err(d)?;
        let r = cl.query_one("SELECT 1::int8 AS n", &[]).await.map_err(d)?;
        cl.batch_execute("COMMIT").await.map_err(d)?;
        Ok(format!("n={}", r.get::<_, i64>("n")))
    });
    case!(res, "7_tx_ext_param", |cl| {
        cl.batch_execute("BEGIN").await.map_err(d)?;
        let r = cl.query_one("SELECT $1::int8 AS n", &[&1i64]).await.map_err(d)?;
        cl.batch_execute("COMMIT").await.map_err(d)?;
        Ok(format!("n={}", r.get::<_, i64>("n")))
    });
    case!(res, "8_tx_two_ext_param", |cl| {
        cl.batch_execute("BEGIN").await.map_err(d)?;
        let a = cl.query_one("SELECT $1::int8 AS n", &[&1i64]).await.map_err(d)?;
        let b = cl.query_one("SELECT $1::int8 AS n", &[&2i64]).await.map_err(d)?;
        cl.batch_execute("COMMIT").await.map_err(d)?;
        Ok(format!("{}+{}", a.get::<_, i64>("n"), b.get::<_, i64>("n")))
    });
    case!(res, "9_tx_insert_text_param_rollback", |cl| {
        cl.batch_execute("BEGIN").await.map_err(d)?;
        let n = cl
            .execute(
                "INSERT INTO projects (id, workspace_id, name) SELECT gen_random_uuid(), gen_random_uuid(), $1",
                &[&"pgshapes-probe"],
            )
            .await
            .map_err(d)?;
        cl.batch_execute("ROLLBACK").await.map_err(d)?;
        Ok(format!("rows={n}"))
    });
    case!(res, "10_tx_insert_on_conflict_param_rollback", |cl| {
        cl.batch_execute("BEGIN").await.map_err(d)?;
        let n = cl
            .execute(
                "INSERT INTO projects (id, workspace_id, name) SELECT gen_random_uuid(), gen_random_uuid(), $1 \
                 ON CONFLICT (id) DO NOTHING",
                &[&"pgshapes-probe"],
            )
            .await
            .map_err(d)?;
        cl.batch_execute("ROLLBACK").await.map_err(d)?;
        Ok(format!("rows={n}"))
    });
    // Bisect: is it "a persistent write" or "an extended-protocol statement that answers NoData"?
    case!(res, "11_tx_SIMPLE_insert_rollback", |cl| {
        cl.batch_execute("BEGIN").await.map_err(d)?;
        cl.simple_query("INSERT INTO projects (id, workspace_id, name) SELECT gen_random_uuid(), gen_random_uuid(), 'pgshapes-probe'")
            .await
            .map_err(d)?;
        cl.batch_execute("ROLLBACK").await.map_err(d)?;
        Ok("simple-protocol insert, rolled back".to_string())
    });
    case!(res, "12_tx_ext_insert_RETURNING_rollback", |cl| {
        cl.batch_execute("BEGIN").await.map_err(d)?;
        let r = cl
            .query_one(
                "INSERT INTO projects (id, workspace_id, name) SELECT gen_random_uuid(), gen_random_uuid(), $1 RETURNING name",
                &[&"pgshapes-probe"],
            )
            .await
            .map_err(d)?;
        cl.batch_execute("ROLLBACK").await.map_err(d)?;
        Ok(format!("name={}", r.get::<_, String>("name")))
    });
    case!(res, "13_tx_ext_update_zero_rows_nodata", |cl| {
        cl.batch_execute("BEGIN").await.map_err(d)?;
        let n = cl
            .execute("UPDATE projects SET name = name WHERE name = $1", &[&"pgshapes-no-such-row"])
            .await
            .map_err(d)?;
        cl.batch_execute("ROLLBACK").await.map_err(d)?;
        Ok(format!("rows={n}"))
    });
    case!(res, "14_tx_ext_insert_zero_rows_nodata", |cl| {
        cl.batch_execute("BEGIN").await.map_err(d)?;
        let n = cl
            .execute(
                "INSERT INTO projects (id, workspace_id, name) SELECT gen_random_uuid(), gen_random_uuid(), $1 WHERE false",
                &[&"pgshapes-probe"],
            )
            .await
            .map_err(d)?;
        cl.batch_execute("ROLLBACK").await.map_err(d)?;
        Ok(format!("rows={n}"))
    });
    case!(res, "15_tx_ext_temp_table_insert_nodata", |cl| {
        cl.batch_execute("BEGIN").await.map_err(d)?;
        cl.batch_execute("CREATE TEMP TABLE pgs(x int) ON COMMIT DROP").await.map_err(d)?;
        let n = cl.execute("INSERT INTO pgs VALUES ($1)", &[&1i32]).await.map_err(d)?;
        cl.batch_execute("COMMIT").await.map_err(d)?;
        Ok(format!("rows={n}"))
    });
    // `execute_typed` is ONE pipeline on the unnamed statement (Parse, Bind, Describe, Execute, Sync) instead of
    // prepare-then-execute on a NAMED statement. Does the NoData failure follow the named-statement flow?
    {
        use tokio_postgres::types::{ToSql, Type};
        let probe: &(dyn ToSql + Sync) = &"pgshapes-probe";
        case!(res, "16_tx_execute_typed_insert_rollback", |cl| {
            cl.batch_execute("BEGIN").await.map_err(d)?;
            let n = cl
                .execute_typed(
                    "INSERT INTO projects (id, workspace_id, name) SELECT gen_random_uuid(), gen_random_uuid(), $1",
                    &[(probe, Type::TEXT)],
                )
                .await
                .map_err(d)?;
            cl.batch_execute("ROLLBACK").await.map_err(d)?;
            Ok(format!("rows={n}"))
        });
        case!(res, "17_tx_execute_typed_update_zero_rows", |cl| {
            cl.batch_execute("BEGIN").await.map_err(d)?;
            let n = cl
                .execute_typed("UPDATE projects SET name = name WHERE name = $1", &[(probe, Type::TEXT)])
                .await
                .map_err(d)?;
            cl.batch_execute("ROLLBACK").await.map_err(d)?;
            Ok(format!("rows={n}"))
        });
        case!(res, "18_execute_typed_autocommit_zero_rows", |cl| {
            let n = cl
                .execute_typed(
                    "INSERT INTO projects (id, workspace_id, name) SELECT gen_random_uuid(), gen_random_uuid(), $1 WHERE false",
                    &[(probe, Type::TEXT)],
                )
                .await
                .map_err(d)?;
            Ok(format!("rows={n}"))
        });
    }
    Ok(Value::Object(res))
}
