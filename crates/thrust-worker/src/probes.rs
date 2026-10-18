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

fn admin_ok(req: &web_sys::Request, env: &Env) -> bool {
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
    for _ in 0..5 {
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
