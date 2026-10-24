//! Cloudflare Worker composition root.
//!
//! Compile-time adapter set = this crate's dependency/feature list (ADR-0007):
//!   * `cloudflare` (always): Hyperdrive/Postgres (sessions included), R2, Queues, Durable Objects
//!   * `durable` (always): SQLite inside the one instance-level Durable Object `AppDo` (spike 0002, cell `worker-do`);
//!     the Worker only forwards to it
//!   * `memory` (default, dev/test only): in-isolate adapters
//! `THRUST_ADAPTERS` only chooses between sets that were compiled in.

use std::sync::Arc;

use axum::body::Body;
use thrust_adapters_cloudflare::*;
use thrust_adapters_oidc::{OidcConfig, OpenIdProvider};
use thrust_adapters_postgres::PgDatabase;
use thrust_api::{ApiConfig, InternalErrorDetail};
use thrust_contracts::Envelope;
use thrust_core::{ActivityConsumer, Argon2Params, Argon2idHasher, App, CoreConfig, Ports, SystemScope};
use thrust_ports::{Background, Clock, EventConsumer, IdGen, OidcProvider, PasswordHasher, Rng};
use http_body_util::BodyExt;
use tower_service::Service;
use worker::worker_sys::web_sys;
use worker::*;

mod app_do;
mod durable;
mod probes;
#[cfg(feature = "memory")]
mod memory;

static ROUTER: std::sync::OnceLock<axum::Router> = std::sync::OnceLock::new();

/// Same ceiling as the router's `DefaultBodyLimit` (8 MiB).
const MAX_REQUEST_BODY: usize = 8 * 1024 * 1024;

// ---------------------------------------------------------------------------------------------
// Configuration (vars + secrets from the Worker env; secrets never come from files)
// ---------------------------------------------------------------------------------------------

pub(crate) fn text(env: &Env, name: &str) -> Option<String> {
    env.secret(name)
        .ok()
        .map(|s| s.to_string())
        .or_else(|| env.var(name).ok().map(|v| v.to_string()))
}

fn flag(env: &Env, name: &str, default: bool) -> bool {
    match text(env, name).as_deref() {
        Some("1") | Some("true") | Some("on") => true,
        Some("0") | Some("false") | Some("off") => false,
        _ => default,
    }
}

fn num<T: std::str::FromStr>(env: &Env, name: &str, default: T) -> T {
    text(env, name).and_then(|v| v.parse().ok()).unwrap_or(default)
}

pub(crate) fn core_config(env: &Env) -> CoreConfig {
    let defaults = CoreConfig::default();
    CoreConfig {
        create_rate_limit: num(env, "THRUST_CREATE_RATE_LIMIT", 1000),
        rate_window_secs: num(env, "THRUST_RATE_WINDOW_SECS", 60),
        outbox_fastpath: flag(env, "THRUST_OUTBOX_FASTPATH", true),
        sign_in_rate_limit: num(env, "THRUST_SIGNIN_RATE_LIMIT", defaults.sign_in_rate_limit),
        // OFF unless a var says on, as in the native server (Q-0050.8): the stock frontend sends the CSRF token only on its
        // auth form posts. The flip is when the custom frontend replaces it.
        csrf_enforce: flag(env, "THRUST_CSRF_ENFORCE", false),
        // OWASP's minimum unless a var says otherwise (a test target sets a cheap cost)
        argon2: Argon2Params {
            m_cost_kib: num(env, "THRUST_ARGON2_M_KIB", defaults.argon2.m_cost_kib),
            t_cost: num(env, "THRUST_ARGON2_T", defaults.argon2.t_cost),
            p_cost: num(env, "THRUST_ARGON2_P", defaults.argon2.p_cost),
        },
        ..defaults
    }
}

/// The OIDC provider, when the issuer, client id and redirect URI are all configured (the client secret is a Worker
/// secret). Some of the three and not all is an error, not a Worker that quietly has no login.
pub(crate) fn oidc_provider(env: &Env) -> Result<Option<Arc<dyn OidcProvider>>> {
    let get = |n: &str| text(env, n).filter(|v| !v.is_empty());
    match (get("THRUST_OIDC_ISSUER"), get("THRUST_OIDC_CLIENT_ID"), get("THRUST_OIDC_REDIRECT_URI")) {
        (Some(issuer), Some(client_id), Some(redirect_uri)) => {
            let cfg = OidcConfig { issuer, client_id, client_secret: get("THRUST_OIDC_CLIENT_SECRET"), redirect_uri };
            Ok(Some(Arc::new(OpenIdProvider::new(cfg, Arc::new(WorkerFetch)))))
        }
        (None, None, None) => Ok(None),
        _ => Err(Error::RustError(
            "THRUST_OIDC_ISSUER, THRUST_OIDC_CLIENT_ID and THRUST_OIDC_REDIRECT_URI must be set together".into(),
        )),
    }
}

fn adapter_set(env: &Env) -> String {
    let default = if cfg!(feature = "memory") { "memory" } else { "cloudflare" };
    text(env, "THRUST_ADAPTERS").unwrap_or_else(|| default.into())
}

fn api_config(env: &Env, adapters: &str) -> ApiConfig {
    ApiConfig {
        test_routes: flag(env, "THRUST_TEST_ROUTES", false),
        admin_token: text(env, "THRUST_ADMIN_TOKEN"),
        target: "worker".into(),
        adapters: adapters.to_string(),
    }
}

/// Build the per-request `App`. Request-scoped because `ctx.waitUntil` and the Hyperdrive socket
/// belong to one request in workerd.
fn build_app(env: &Env, background: Arc<dyn Background>) -> Result<App> {
    let adapters = adapter_set(env);
    let clock: Arc<dyn Clock> = Arc::new(JsClock);
    let ids: Arc<dyn IdGen> = Arc::new(JsIdGen);

    let cfg = core_config(env);
    let rng: Arc<dyn Rng> = Arc::new(JsRng);
    let hasher: Arc<dyn PasswordHasher> = Arc::new(Argon2idHasher::new(cfg.argon2));
    let oidc = oidc_provider(env)?;

    let app = match adapters.as_str() {
        "cloudflare" => {
            let connector = HyperdriveConnector::new(env.hyperdrive("HYPERDRIVE")?);
            // This composition root mints the proof for the relay, sweeper, consumer and seed.
            App::new(
                Ports {
                    db: Arc::new(PgDatabase::new(Arc::new(connector))),
                    queue: Arc::new(CfQueue(env.queue("EVENTS")?)),
                    blobs: Arc::new(R2BlobStore(env.bucket("BLOBS")?)),
                    limiter: Arc::new(DoRateLimiter(env.durable_object("RATE_LIMITER")?)),
                    clock,
                    ids,
                    background,
                    rng,
                    hasher,
                    oidc,
                },
                cfg,
                SystemScope::mint(),
            )
        }
        #[cfg(feature = "memory")]
        "memory" => memory::app(clock, ids, background, rng, hasher, oidc, cfg),
        "durable" => {
            return Err(Error::RustError("the durable set is built inside AppDo; the Worker only forwards to it".into()))
        }
        other => return Err(Error::RustError(format!("THRUST_ADAPTERS '{other}' is not compiled into this bundle"))),
    };
    Ok(app)
}

// ---------------------------------------------------------------------------------------------
// fetch
// ---------------------------------------------------------------------------------------------

#[event(fetch)]
async fn fetch(req: web_sys::Request, env: Env, ctx: Context) -> Result<web_sys::Response> {
    console_error_panic_hook::set_once();

    // Durable Object test routes (WebSocket hibernation needs the raw request/response).
    let path = Url::parse(&req.url()).map(|u| u.path().to_string()).unwrap_or_default();
    if path.starts_with("/_test/room/") {
        return durable::room_route(req, &env, &path).await;
    }
    // Binding probes: one real operation per binding the matrix claims (test routes + admin token).
    if path.starts_with("/_test/bindings/") {
        return probes::probe_route(req, &env, &path).await;
    }

    // `durable`: everything that touches the database runs inside the one instance-level object. `/_do/*` is the
    // Worker-to-object protocol, so a client may not name it.
    if adapter_set(&env) == "durable" {
        if path.starts_with("/_do/") {
            let r: Response = Response::error("not found", 404)?;
            return Ok(r.into());
        }
        // The object's own probes (test routes + admin token): `/_test/do/<name>` -> `/_do/<name>`.
        if let Some(name) = path.strip_prefix("/_test/do/") {
            if !probes::admin_ok(&req, &env) {
                let r: Response = Response::error("not found", 404)?;
                return Ok(r.into());
            }
            let url = Url::parse(&req.url())?;
            let mut target = format!("https://app.internal/_do/{name}");
            if let Some(q) = url.query() {
                target.push('?');
                target.push_str(q);
            }
            let resp = app_do::stub(&env)?.fetch_with_str(&target).await?;
            return Ok(resp.into());
        }
        let resp = app_do::stub(&env)?.fetch_with_request(Request::from(req)).await?;
        return Ok(resp.into());
    }

    let app = build_app(&env, Arc::new(WaitUntilBackground(Arc::new(ctx))))?;
    let adapters = adapter_set(&env);
    serve(req, app, api_config(&env, &adapters)).await
}

/// Route one request through the shared router with `app` and `api` installed as its extensions. Used by the `fetch`
/// handler for the sets that build their `App` per request and by `AppDo` for the one it holds.
pub(crate) async fn serve(req: web_sys::Request, app: App, api: thrust_api::ApiConfig) -> Result<web_sys::Response> {
    let http_req = request_from_wasm(req)?;

    // Drain the request body BEFORE routing (measured: skipping this makes ~1 in 4 `wrangler dev`
    // runs of the suite fail). workers-rs' `Body` holds a reader on the JS request stream and
    // cancels it on drop. Handlers that answer early (401/403/404/409 before the JSON extractor
    // runs) drop the body AFTER the response is sent; workerd then rejects the cancel with
    // "TypeError: Can't read from request stream after response has been sent", an unhandled
    // async exception that `wrangler dev` answers by resetting other in-flight connections.
    // Cost: request bodies are buffered (bounded), so the Worker root has no streaming uploads.
    let (parts, body) = http_req.into_parts();
    let limited = http_body_util::Limited::new(body, MAX_REQUEST_BODY);
    let bytes = match limited.collect().await {
        Ok(c) => c.to_bytes(),
        Err(_) => {
            let r: Response = Response::error("request body too large or unreadable", 413)?;
            return Ok(r.into());
        }
    };
    let mut http_req = axum::http::Request::from_parts(parts, Body::from(bytes));
    http_req.extensions_mut().insert(app);
    http_req.extensions_mut().insert(api);

    let mut router = ROUTER.get_or_init(thrust_api::router).clone();
    let resp: axum::http::Response<Body> = match router.call(http_req).await {
        Ok(r) => r,
        Err(never) => match never {},
    };
    if let Some(InternalErrorDetail(d)) = resp.extensions().get::<InternalErrorDetail>() {
        console_error!("internal error: {d}");
    }
    response_to_wasm(resp)
}

// ---------------------------------------------------------------------------------------------
// queue consumer (ADR-0070): same `process_event` body as the native consumer
// ---------------------------------------------------------------------------------------------

#[event(queue)]
async fn queue(batch: MessageBatch<String>, env: Env, _ctx: Context) -> Result<()> {
    if adapter_set(&env) == "durable" {
        // The consumer writes the activity row, so it runs inside the object that owns the database.
        let stub = app_do::stub(&env)?;
        for msg in batch.messages()? {
            let mut init = RequestInit::new();
            init.with_method(Method::Post).with_body(Some(msg.body().into()));
            let req = Request::new_with_init("https://app.internal/_do/consume", &init)?;
            match stub.fetch_with_request(req).await {
                Ok(r) if r.status_code() == 200 => msg.ack(),
                Ok(r) => {
                    console_error!("consumer (object) answered {} for a message", r.status_code());
                    msg.retry_with_options(&QueueRetryOptionsBuilder::new().with_delay_seconds(5).build());
                }
                Err(e) => {
                    console_error!("consumer (object) failed: {e}");
                    msg.retry_with_options(&QueueRetryOptionsBuilder::new().with_delay_seconds(5).build());
                }
            }
        }
        return Ok(());
    }
    let app = build_app(&env, Arc::new(LocalBackground))?;
    let consumer = ActivityConsumer(app);
    for msg in batch.messages()? {
        match serde_json::from_str::<Envelope>(msg.body()) {
            Ok(envelope) => match consumer.handle(&envelope).await {
                Ok(()) => msg.ack(),
                Err(e) => {
                    console_error!("consumer failed for {}: {e}", envelope.event_id);
                    msg.retry_with_options(&QueueRetryOptionsBuilder::new().with_delay_seconds(5).build());
                }
            },
            Err(e) => {
                // A poison message must not loop forever; a real deployment routes it to a DLQ.
                console_error!("dropping undecodable queue message: {e}");
                msg.ack();
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// cron (ADR-0072): the outbox sweeper of ADR-0046
// ---------------------------------------------------------------------------------------------

#[event(scheduled)]
async fn scheduled(event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    console_error_panic_hook::set_once();
    if adapter_set(&env) == "durable" {
        let mut init = RequestInit::new();
        init.with_method(Method::Post);
        let outcome = match app_do::stub(&env).and_then(|s| Request::new_with_init("https://app.internal/_do/sweep", &init).map(|r| (s, r))) {
            Ok((stub, req)) => stub.fetch_with_request(req).await.map(|r| (r.status_code(), r)),
            Err(e) => Err(e),
        };
        match outcome {
            Ok((200, mut r)) => console_log!("scheduled cron='{}': sweep in the object: {}", event.cron(), r.text().await.unwrap_or_default()),
            Ok((code, _)) => console_error!("scheduled: the object answered {code}"),
            Err(e) => console_error!("scheduled: cannot reach the object: {e}"),
        }
        return;
    }
    let app = match build_app(&env, Arc::new(LocalBackground)) {
        Ok(v) => v,
        Err(e) => {
            console_error!("scheduled: cannot build app: {e}");
            return;
        }
    };
    match thrust_core::sweep_outbox(&app, 100).await {
        Ok(rep) => console_log!(
            "scheduled cron='{}': sweep found={} published={} failed={}",
            event.cron(),
            rep.found,
            rep.published,
            rep.failed
        ),
        Err(e) => console_error!("scheduled: sweep failed: {e}"),
    }
}
