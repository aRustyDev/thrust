//! Cloudflare Worker composition root.
//!
//! Compile-time adapter set = this crate's dependency/feature list (ADR-0007):
//!   * `cloudflare` (always): Hyperdrive/Postgres (sessions included), R2, Queues, the Workers Rate Limiting bindings (one per
//!     kind of key, ADR-0033), and the Durable Object classes (`RoomDo`, the P1 collab template)
//!   * `memory` (default, dev/test only): in-isolate adapters
//!
//! One Worker (ADR-0090, FR-11): `fetch`, `queue` and `scheduled` in this module, and no other entrypoint, Workflow, service
//! binding or JavaScript.
//!
//! `THRUST_ADAPTERS` only chooses between sets that were compiled in.

use std::sync::Arc;

use axum::body::Body;
use http_body_util::BodyExt;
use thrust_adapters_cloudflare::*;
use thrust_adapters_oidc::{OidcConfig, OpenIdProvider};
use thrust_adapters_postgres::{Connector, CountingConnector, PgDatabase, RoundTrips};
use thrust_api::{set_db_round_trips, set_version, ApiConfig, InternalErrorDetail, OperatorAudit};
use thrust_contracts::Envelope;
use thrust_core::{ActivityConsumer, App, Argon2Params, Argon2idHasher, CoreConfig, Ports, SystemScope};
use thrust_ports::{Background, Clock, EventConsumer, IdGen, OidcProvider, PasswordHasher, Rng};
use tower_service::Service;
use worker::worker_sys::web_sys;
use worker::*;

mod durable;
#[cfg(feature = "memory")]
mod memory;
mod probes;

static ROUTER: std::sync::OnceLock<axum::Router> = std::sync::OnceLock::new();

/// The shared router with Plane's internal API (`/api/`, `/auth/`, issue #44) and its public one (`/api/v1`, issue #58) mounted
/// beside it.
fn router() -> axum::Router {
    thrust_api::router().merge(thrust_compat_plane::routes()).merge(thrust_api_v1::router())
}

/// A key's allowance on `/api/v1`: `THRUST_V1_RATE_LIMIT` in Plane's spelling (`100000/minute`), Plane's `60/minute` when it is not
/// set. One that does not read is logged and not used: a misspelt setting must not take the whole Worker down.
fn v1_config(env: &Env) -> thrust_api_v1::V1Config {
    match text(env, "THRUST_V1_RATE_LIMIT") {
        None => thrust_api_v1::V1Config::default(),
        Some(spec) => thrust_api_v1::V1Config::parse(&spec).unwrap_or_else(|e| {
            console_error!("THRUST_V1_RATE_LIMIT is not used: {e}");
            thrust_api_v1::V1Config::default()
        }),
    }
}

/// Same ceiling as the router's `DefaultBodyLimit` (8 MiB).
const MAX_REQUEST_BODY: usize = 8 * 1024 * 1024;

// ---------------------------------------------------------------------------------------------
// Configuration (vars + secrets from the Worker env; secrets never come from files)
// ---------------------------------------------------------------------------------------------

pub(crate) fn text(env: &Env, name: &str) -> Option<String> {
    env.secret(name).ok().map(|s| s.to_string()).or_else(|| env.var(name).ok().map(|v| v.to_string()))
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
        // The sign-in limit below is what the in-isolate (`memory`) set counts with. On the `cloudflare` set the allowance is the
        // `[[ratelimits]]` binding's: its window is checked against the bindings' periods (`check_rate_windows`), and its limit
        // only gives the upper bound that `CfRateLimiter` reports as `remaining`.
        outbox_fastpath: flag(env, "THRUST_OUTBOX_FASTPATH", true),
        sign_in_rate_limit: num(env, "THRUST_SIGNIN_RATE_LIMIT", defaults.sign_in_rate_limit),
        // OFF unless a var says on, as in the native server (Q-0050.8): the stock frontend sends the CSRF token only on its
        // auth form posts. The flip is when the custom frontend replaces it.
        csrf_enforce: flag(env, "THRUST_CSRF_ENFORCE", false),
        // where the links in emails point (D-041): a var, never taken from a request. Unset: no link can be made.
        public_base_url: text(env, "THRUST_PUBLIC_BASE_URL")
            .filter(|v| !v.is_empty())
            .map(|v| v.trim_end_matches('/').to_string()),
        password_reset_ttl_secs: num(env, "THRUST_PASSWORD_RESET_TTL_SECS", defaults.password_reset_ttl_secs),
        password_reset_cooldown_secs: num(
            env,
            "THRUST_PASSWORD_RESET_COOLDOWN_SECS",
            defaults.password_reset_cooldown_secs,
        ),
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
fn oidc_provider(env: &Env) -> Result<Option<Arc<dyn OidcProvider>>> {
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

/// Refuse to start with a window the rate-limit bindings cannot count over (ADR-0033, D-007). A binding counts over 10 s or 60 s,
/// its own `period` decides which, and the app reports the window it was configured with (`Retry-After`, `X-RateLimit-Reset`), so
/// that window has to be one of the two. Two are in play: sign-in's (a constant of the core's config) and the period of
/// `THRUST_V1_RATE_LIMIT`, when it is set. A v1 setting that does not read is not checked:
/// `v1_config` logs it and uses Plane's default, which is a minute.
fn check_rate_windows(env: &Env, cfg: &CoreConfig) -> Result<()> {
    check_binding_period("the sign-in rate window", cfg.sign_in_rate_window_secs).map_err(Error::RustError)?;
    if let Some(v1) = text(env, "THRUST_V1_RATE_LIMIT").and_then(|spec| thrust_api_v1::V1Config::parse(&spec).ok()) {
        check_binding_period("the period of THRUST_V1_RATE_LIMIT", v1.rate_window_secs).map_err(Error::RustError)?;
    }
    Ok(())
}

/// The tag and the id of the version that is running, from the `CF_VERSION_METADATA` binding (`[version_metadata]` in the wrangler
/// config). Both are empty when the binding is not declared, and `set_version` then sends no header (issue #49).
///
/// Local `wrangler dev` does declare it, with no tag and a random UUID for the id (measured with wrangler 4.123.0), so the header is
/// that id there. A deploy made without `--tag` has the same shape: the id, not the commit, which the deploy's smoke refuses.
fn version(env: &Env) -> (String, String) {
    env.get_binding::<WorkerVersionMetadata>("CF_VERSION_METADATA").map(|v| (v.tag(), v.id())).unwrap_or_default()
}

fn adapter_set(env: &Env) -> String {
    let default = if cfg!(feature = "memory") { "memory" } else { "cloudflare" };
    text(env, "THRUST_ADAPTERS").unwrap_or_else(|| default.into())
}

fn api_config(env: &Env, adapters: &str) -> ApiConfig {
    ApiConfig {
        test_routes: flag(env, "THRUST_TEST_ROUTES", false),
        admin_token: text(env, "THRUST_ADMIN_TOKEN"),
        operator_routes: flag(env, "THRUST_OPERATOR_ROUTES", false),
        operator_token: text(env, "THRUST_OPERATOR_TOKEN"),
        target: "worker".into(),
        adapters: adapters.to_string(),
    }
}

/// Build the per-request `App`. Request-scoped because `ctx.waitUntil` and the Hyperdrive socket
/// belong to one request in workerd.
///
/// With test routes on, the Postgres set also comes back with the counter of the statements this `App` sends (issue #39).
/// The set that wired it is the one that returns it, so a set that cannot count (`memory`) returns none and `fetch` sends
/// no header, instead of a count of zero.
fn build_app(env: &Env, background: Arc<dyn Background>) -> Result<(App, Option<RoundTrips>)> {
    let adapters = adapter_set(env);
    let clock: Arc<dyn Clock> = Arc::new(JsClock);
    let ids: Arc<dyn IdGen> = Arc::new(JsIdGen);

    let cfg = core_config(env);
    let rng: Arc<dyn Rng> = Arc::new(JsRng);
    let hasher: Arc<dyn PasswordHasher> = Arc::new(Argon2idHasher::new(cfg.argon2));
    let oidc = oidc_provider(env)?;

    let mut round_trips = None;
    let app = match adapters.as_str() {
        "cloudflare" => {
            check_rate_windows(env, &cfg)?;
            let connector: Arc<dyn Connector> = Arc::new(HyperdriveConnector::new(env.hyperdrive("HYPERDRIVE")?));
            // The connector is built for this request, so the count is this request's. Only test routes ask for it: the count
            // leaves in a response header that a real deployment does not send.
            let connector: Arc<dyn Connector> = if flag(env, "THRUST_TEST_ROUTES", false) {
                let rt = RoundTrips::new();
                round_trips = Some(rt.clone());
                Arc::new(CountingConnector::new(connector, rt))
            } else {
                connector
            };
            // This composition root mints the proof for the relay, sweeper, consumer and seed.
            App::new(
                Ports {
                    db: Arc::new(PgDatabase::new(connector)),
                    queue: Arc::new(CfQueue(env.queue("EVENTS")?)),
                    blobs: Arc::new(R2BlobStore(env.bucket("BLOBS")?)),
                    limiter: Arc::new(CfRateLimiter {
                        signin: env.rate_limiter("RL_SIGNIN")?,
                        v1: env.rate_limiter("RL_V1")?,
                    }),
                    clock,
                    ids,
                    background,
                    rng,
                    hasher,
                    oidc,
                    // the `send_email` binding is wired in E3, with a sender domain the account owns: until then no reset is offered
                    email: None,
                },
                cfg,
                SystemScope::mint(),
            )
        }
        #[cfg(feature = "memory")]
        "memory" => memory::app(clock, ids, background, rng, hasher, oidc, cfg),
        other => return Err(Error::RustError(format!("THRUST_ADAPTERS '{other}' is not compiled into this bundle"))),
    };
    Ok((app, round_trips))
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

    let (app, round_trips) = build_app(&env, Arc::new(WaitUntilBackground(Arc::new(ctx))))?;
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
    http_req.extensions_mut().insert(v1_config(&env));

    let mut router = ROUTER.get_or_init(router).clone();
    let mut resp: axum::http::Response<Body> = match router.call(http_req).await {
        Ok(r) => r,
        Err(never) => match never {},
    };
    if let Some(InternalErrorDetail(d)) = resp.extensions().get::<InternalErrorDetail>() {
        console_error!("internal error: {d}");
    }
    if let Some(OperatorAudit(line)) = resp.extensions().get::<OperatorAudit>() {
        console_log!("{line}");
    }
    // Test routes on a Postgres set only (`build_app`): how many statements this request had sent when the router answered.
    if let Some(rt) = round_trips {
        set_db_round_trips(&mut resp, rt.get());
    }
    // Always, so a handler cannot make the Worker report another build: the deploy's smoke compares this with what it deployed.
    let (tag, id) = version(&env);
    set_version(&mut resp, &tag, &id);
    response_to_wasm(resp)
}

// ---------------------------------------------------------------------------------------------
// queue consumer (ADR-0070): same `process_event` body as the native consumer
// ---------------------------------------------------------------------------------------------

#[event(queue)]
async fn queue(batch: MessageBatch<String>, env: Env, _ctx: Context) -> Result<()> {
    let (app, _) = build_app(&env, Arc::new(LocalBackground))?;
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
        Ok((v, _)) => v,
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
