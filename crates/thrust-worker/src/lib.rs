//! Cloudflare Worker composition root.
//!
//! Compile-time adapter set = this crate's dependency/feature list (ADR-0007):
//!   * `cloudflare` (always): Hyperdrive/Postgres (sessions included), R2, Queues, Durable Objects
//!   * `memory` (default, dev/test only): in-isolate adapters
//! `THRUST_ADAPTERS` only chooses between sets that were compiled in.

use std::sync::Arc;

use axum::body::Body;
use thrust_adapters_cloudflare::*;
use thrust_adapters_postgres::PgDatabase;
use thrust_api::{ApiConfig, InternalErrorDetail};
use thrust_contracts::Envelope;
use thrust_core::{ActivityConsumer, Argon2Params, Argon2idHasher, App, CoreConfig, Ports, SystemScope};
use thrust_ports::{Background, Clock, EventConsumer, IdGen, PasswordHasher, Rng};
use http_body_util::BodyExt;
use tower_service::Service;
use worker::worker_sys::web_sys;
use worker::*;

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

fn core_config(env: &Env) -> CoreConfig {
    let defaults = CoreConfig::default();
    CoreConfig {
        create_rate_limit: num(env, "THRUST_CREATE_RATE_LIMIT", 1000),
        rate_window_secs: num(env, "THRUST_RATE_WINDOW_SECS", 60),
        outbox_fastpath: flag(env, "THRUST_OUTBOX_FASTPATH", true),
        sign_in_rate_limit: num(env, "THRUST_SIGNIN_RATE_LIMIT", defaults.sign_in_rate_limit),
        // OWASP's minimum unless a var says otherwise (a test target sets a cheap cost)
        argon2: Argon2Params {
            m_cost_kib: num(env, "THRUST_ARGON2_M_KIB", defaults.argon2.m_cost_kib),
            t_cost: num(env, "THRUST_ARGON2_T", defaults.argon2.t_cost),
            p_cost: num(env, "THRUST_ARGON2_P", defaults.argon2.p_cost),
        },
        ..defaults
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
                },
                cfg,
                SystemScope::mint(),
            )
        }
        #[cfg(feature = "memory")]
        "memory" => memory::app(clock, ids, background, rng, hasher, cfg),
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

    let app = build_app(&env, Arc::new(WaitUntilBackground(Arc::new(ctx))))?;
    let adapters = adapter_set(&env);
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
    http_req.extensions_mut().insert(api_config(&env, &adapters));

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
