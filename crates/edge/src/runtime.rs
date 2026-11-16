//! The Workers runtime glue (wasm32 only): bindings, configuration, the JWKS cache, the opt-in endpoints and the two ways out.
//! Every decision is delegated to the plain modules; what is here is plumbing that the spike (research/0016) proved:
//! Flagship through extern method imports, `redirect: manual`, and a fetched `101` returned with its WebSocket.

use std::cell::RefCell;

use js_sys::{Date, Promise, Reflect, JSON};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use worker::*;

use crate::access::{self, Expect, Jwks};
use crate::forward;
use crate::pin::{self, Kind, Pin};
use crate::route::{self, Decision, EvalContext, FlagError, Flags};

/// How long a pin lasts: a working day for a person, an hour for CI.
const HUMAN_PIN_SECONDS: u64 = 12 * 3600;
const CI_PIN_SECONDS: u64 = 3600;
/// How long fetched Access keys are trusted before they are fetched again (a token with an unknown `kid` refetches at once).
const JWKS_SECONDS: u64 = 3600;

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    /// The Flagship binding. Its methods are RPC stubs: they must be called as methods on the binding; `Function.prototype.call`
    /// is itself taken as a remote method and fails (research/0016).
    #[wasm_bindgen(extends = js_sys::Object)]
    type Flagship;
    #[wasm_bindgen(method, js_name = getStringValue)]
    fn get_string_value(this: &Flagship, key: &str, default: &JsValue, ctx: &JsValue) -> Promise;
    #[wasm_bindgen(method, js_name = getBooleanValue)]
    fn get_boolean_value(this: &Flagship, key: &str, default: &JsValue, ctx: &JsValue) -> Promise;
    #[wasm_bindgen(method, js_name = getObjectValue)]
    fn get_object_value(this: &Flagship, key: &str, default: &JsValue, ctx: &JsValue) -> Promise;
}

struct FlagshipFlags(Flagship);

impl FlagshipFlags {
    fn from_env(env: &Env) -> Option<Self> {
        let binding = Reflect::get(env, &JsValue::from("FLAGS")).ok()?;
        if binding.is_undefined() {
            return None;
        }
        Some(FlagshipFlags(binding.unchecked_into()))
    }
}

async fn settle(p: Promise) -> Result<JsValue, FlagError> {
    JsFuture::from(Promise::resolve(&p)).await.map_err(|_| FlagError)
}

fn empty_object() -> JsValue {
    js_sys::Object::new().into()
}

impl Flags for FlagshipFlags {
    async fn routing(&self) -> Result<bool, FlagError> {
        let v = settle(self.0.get_boolean_value(route::ROUTING_FLAG, &JsValue::FALSE, &empty_object())).await?;
        v.as_bool().ok_or(FlagError)
    }

    async fn alias(&self, key: &str) -> Result<String, FlagError> {
        let v = settle(self.0.get_string_value(key, &JsValue::from(""), &empty_object())).await?;
        v.as_string().ok_or(FlagError)
    }

    async fn target(&self, key: &str, ctx: &EvalContext) -> Result<serde_json::Value, FlagError> {
        let ctx = serde_json::to_string(ctx).map_err(|_| FlagError)?;
        let ctx = JSON::parse(&ctx).map_err(|_| FlagError)?;
        let v = settle(self.0.get_object_value(key, &empty_object(), &ctx)).await?;
        let text = JSON::stringify(&v).map_err(|_| FlagError)?.as_string().ok_or(FlagError)?;
        serde_json::from_str(&text).map_err(|_| FlagError)
    }
}

/// The edge's configuration: vars and secrets, read per request (they are bindings, not I/O).
struct Config {
    shared_origin: String,
    shared_host: String,
    issuer: String,
    preview_suffix: String,
    pin_key: String,
}

impl Config {
    fn from_env(env: &Env) -> Result<Self> {
        let shared_origin = env.var("SHARED_ORIGIN")?.to_string();
        let shared_host =
            shared_origin.strip_prefix("https://").ok_or("SHARED_ORIGIN must be https://<host>")?.to_string();
        let team = env.var("ACCESS_TEAM_DOMAIN")?.to_string();
        Ok(Config {
            shared_origin,
            shared_host,
            issuer: format!("https://{team}"),
            preview_suffix: env.var("PREVIEW_HOST_SUFFIX")?.to_string(),
            pin_key: env.secret("EDGE_PIN_KEY")?.to_string(),
        })
    }

    /// The AUD of the Access application of `audience`'s opt-in path: the var `ACCESS_AUD_<AUDIENCE>`.
    fn aud(env: &Env, audience: &str) -> Option<String> {
        env.var(&format!("ACCESS_AUD_{}", audience.to_ascii_uppercase()))
            .ok()
            .map(|v| v.to_string())
            .filter(|v| !v.is_empty())
    }
}

fn now() -> u64 {
    (Date::now() / 1000.0) as u64
}

thread_local! {
    static JWKS: RefCell<Option<(Jwks, u64)>> = const { RefCell::new(None) };
}

async fn fetch_jwks(issuer: &str) -> Result<Jwks> {
    let mut resp = Fetch::Url(Url::parse(&format!("{issuer}/cdn-cgi/access/certs"))?).send().await?;
    if resp.status_code() != 200 {
        return Err(Error::RustError(format!("the Access keys answered {}", resp.status_code())));
    }
    let bytes = resp.bytes().await?;
    access::parse_jwks(&bytes).map_err(|e| Error::RustError(format!("the Access keys: {e:?}")))
}

async fn jwks(issuer: &str, refresh: bool) -> Result<Jwks> {
    let cached = JWKS.with(|c| c.borrow().clone());
    if let Some((keys, at)) = cached {
        if !refresh && now() < at.saturating_add(JWKS_SECONDS) {
            return Ok(keys);
        }
    }
    let keys = fetch_jwks(issuer).await?;
    JWKS.with(|c| *c.borrow_mut() = Some((keys.clone(), now())));
    Ok(keys)
}

/// Verifies the request's Access token for `aud`, refetching the keys once if its `kid` is unknown.
async fn identity(req: &Request, cfg: &Config, aud: &str) -> std::result::Result<access::Identity, String> {
    let token = req.headers().get("cf-access-jwt-assertion").ok().flatten().ok_or("no Access token")?;
    let expect = Expect { issuer: &cfg.issuer, audience: aud, now: now() };
    let keys = jwks(&cfg.issuer, false).await.map_err(|e| e.to_string())?;
    match access::verify(&token, &keys, &expect) {
        Err(access::AccessError::UnknownKey) => {
            let keys = jwks(&cfg.issuer, true).await.map_err(|e| e.to_string())?;
            access::verify(&token, &keys, &expect).map_err(|e| format!("{e:?}"))
        }
        other => other.map_err(|e| format!("{e:?}")),
    }
}

fn header_pairs(h: &Headers) -> Vec<(String, String)> {
    h.entries().collect()
}

fn headers_from(pairs: &[(String, String)]) -> Result<Headers> {
    let h = Headers::new();
    for (n, v) in pairs {
        h.append(n, v)?;
    }
    Ok(h)
}

fn current_pin(req: &Request, cfg: &Config) -> Option<Pin> {
    let cookies = req.headers().get("cookie").ok().flatten()?;
    let value = pin::from_cookie_header(&cookies)?;
    pin::open(value, cfg.pin_key.as_bytes(), now()).ok()
}

fn plain(status: u16, body: &str) -> Result<Response> {
    Ok(Response::ok(body)?.with_status(status))
}

fn see_other(location: &str, set_cookie: &str) -> Result<Response> {
    let h = Headers::new();
    h.set("location", location)?;
    h.set("set-cookie", set_cookie)?;
    h.set("cache-control", "no-store")?;
    Ok(Response::empty()?.with_status(303).with_headers(h))
}

async fn opt_in(req: &Request, env: &Env, cfg: &Config, audience: &str) -> Result<Response> {
    if !pin::valid_audience(audience) {
        return plain(404, "no such audience");
    }
    let Some(aud) = Config::aud(env, audience) else { return plain(404, "no such audience") };
    let who = match identity(req, cfg, &aud).await {
        Ok(who) => who,
        Err(why) => return plain(403, &format!("not admitted: {why}")),
    };
    let url = req.url()?;
    let requested = url.query_pairs().find(|(k, _)| k == "target").map(|(_, v)| v.into_owned()).unwrap_or_default();
    if !pin::valid_name(&requested) {
        return plain(400, "target: a target or alias name is required");
    }
    let ttl = match who.kind {
        Kind::Human => HUMAN_PIN_SECONDS,
        Kind::Ci => CI_PIN_SECONDS,
    };
    let candidate =
        Pin { principal: who.principal, kind: who.kind, audience: audience.to_string(), requested, exp: now() + ttl };
    // The pin is only issued for a target the rules would route this principal to now, so a typo fails here, not silently later.
    let Some(flags) = FlagshipFlags::from_env(env) else { return plain(503, "routing is unavailable") };
    if let Decision::Production = route::decide(Some(&candidate), &flags, &cfg.preview_suffix).await {
        return plain(403, "that target does not exist or is not open to you");
    }
    let value = pin::seal(&candidate, cfg.pin_key.as_bytes()).map_err(|e| Error::RustError(format!("{e:?}")))?;
    see_other("/", &pin::set_cookie(&value, ttl))
}

async fn whoami(req: &Request, env: &Env, cfg: &Config) -> Result<Response> {
    let pin = current_pin(req, cfg);
    let decision = match (&pin, FlagshipFlags::from_env(env)) {
        (Some(p), Some(flags)) => route::decide(Some(p), &flags, &cfg.preview_suffix).await,
        _ => Decision::Production,
    };
    let (route_name, deployment) = match &decision {
        Decision::Production => (route::PRODUCTION.to_string(), None),
        Decision::Target(t) => (t.name.clone(), Some(t.deployment.clone())),
    };
    Response::from_json(&serde_json::json!({
        "pinned": pin.as_ref().map(|p| serde_json::json!({
            "kind": p.kind, "audience": p.audience, "requested": p.requested, "exp": p.exp,
        })),
        "route": route_name,
        "deployment": deployment,
    }))
}

fn body(req: &Request) -> Option<JsValue> {
    req.inner().body().map(JsValue::from)
}

/// Passes a `101` through with its WebSocket, or rebuilds the response's headers. `shared_origin` matters only with a
/// `preview_host` (a `Location` to rewrite).
fn hand_back(resp: Response, route: &str, preview_host: Option<&str>, shared_origin: &str) -> Result<Response> {
    if resp.status_code() == 101 {
        return match resp.websocket() {
            Some(ws) => Response::from_websocket(ws),
            None => plain(502, "an upgrade without a WebSocket"),
        };
    }
    let pairs = forward::response_headers(&header_pairs(resp.headers()), route, preview_host, shared_origin);
    Ok(resp.with_headers(headers_from(&pairs)?))
}

/// Production through the service binding. It needs none of the edge's own configuration, so production does not depend on it.
async fn to_production(req: &Request, env: &Env) -> Result<Response> {
    let mut init = RequestInit::new();
    init.with_method(req.method())
        .with_headers(headers_from(&route::scrub(&header_pairs(req.headers())))?)
        .with_redirect(RequestRedirect::Manual)
        .with_body(body(req));
    let upstream = Request::new_with_init(req.url()?.as_str(), &init)?;
    let resp = env.service("THRUST")?.fetch_request(upstream).await?;
    hand_back(resp, route::PRODUCTION, None, "")
}

async fn to_target(req: &Request, env: &Env, cfg: &Config, target: &route::Target) -> Result<Response> {
    let url = req.url()?;
    let path_and_query = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_string(),
    };
    let Some(upstream_url) = forward::upstream_url(&target.url, &path_and_query) else {
        return to_production(req, env).await;
    };
    let mut pairs = forward::upstream_headers(&route::scrub(&header_pairs(req.headers())), &cfg.shared_host);
    pairs.push(("cf-access-client-id".to_string(), env.secret("PREVIEW_ACCESS_CLIENT_ID")?.to_string()));
    pairs.push(("cf-access-client-secret".to_string(), env.secret("PREVIEW_ACCESS_CLIENT_SECRET")?.to_string()));
    let mut init = RequestInit::new();
    init.with_method(req.method())
        .with_headers(headers_from(&pairs)?)
        .with_redirect(RequestRedirect::Manual)
        .with_body(body(req));
    let resp = Fetch::Request(Request::new_with_init(&upstream_url, &init)?).send().await?;
    hand_back(resp, &target.name, Some(target.host()), &cfg.shared_origin)
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();
    let path = req.path();
    // A missing var or secret takes the edge's own paths and the routing down, never production.
    let cfg = match Config::from_env(&env) {
        Ok(cfg) => cfg,
        Err(e) if path.starts_with("/_edge/") => return plain(503, &format!("the edge is not configured: {e}")),
        Err(_) => return to_production(&req, &env).await,
    };
    if let Some(rest) = path.strip_prefix("/_edge/") {
        return match rest.split_once('/') {
            Some(("opt-in", audience)) => opt_in(&req, &env, &cfg, audience).await,
            None if rest == "opt-out" => see_other("/", &pin::clear_cookie()),
            None if rest == "whoami" => whoami(&req, &env, &cfg).await,
            _ => plain(404, "not an edge path"),
        };
    }
    let decision = match (current_pin(&req, &cfg), FlagshipFlags::from_env(&env)) {
        (Some(p), Some(flags)) => route::decide(Some(&p), &flags, &cfg.preview_suffix).await,
        _ => Decision::Production,
    };
    match &decision {
        Decision::Production => to_production(&req, &env).await,
        Decision::Target(t) => to_target(&req, &env, &cfg, t).await,
    }
}
