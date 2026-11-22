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
use thrust_contracts::DeadLetterMessage;
use thrust_core::{ActivityConsumer, App, Argon2Params, Argon2idHasher, CoreConfig, Ports, SystemScope, EVENTS_TOPIC};
use thrust_ports::{
    settle, settle_dead_letter, Background, Clock, EmailSender, IdGen, NewDeadLetter, OidcProvider, PasswordHasher,
    Rng, Settled,
};
use tower_service::Service;
use worker::worker_sys::web_sys;
use worker::*;

mod durable;
#[cfg(feature = "memory")]
mod memory;
mod probes;
// Tests of the glue that exists only on wasm32 (G-6): run through wasm-bindgen-test by the `workers` job of test.yml (scripts/wasm-tests.sh).
#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests;

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

/// The Worker's `CoreConfig` from its vars and secrets. An error stops the Worker from building its app (every request, queue batch and
/// cron run builds one), which is how a retention window that is not a number of days is refused (issue #59): read as the default, it
/// would delete rows the person meant to keep.
fn core_config(env: &Env) -> Result<CoreConfig> {
    let defaults = CoreConfig::default();
    let cfg = CoreConfig {
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
    };
    cfg.with_retention_from(|name| text(env, name)).map_err(|e| Error::RustError(e.to_string()))
}

/// The mail service, when `THRUST_EMAIL_FROM` is set: the Worker's `send_email` binding (`EMAIL`) with that sender (and the name
/// in `THRUST_EMAIL_FROM_NAME`, which the adapter documents), for the password reset by email (E3, D-041). Unset is "no mail
/// service" (the app says so, and draws no link), which is every Worker until a deployment names a sender on a domain its account
/// has Email Sending enabled for. A sender that is not usable, a name with no sender, and a sender with no binding are errors and not
/// a Worker that quietly has no reset: the OIDC provider's reasoning, a reset nobody notices is not on is worse than a Worker that
/// says so. (The reset also needs `THRUST_PUBLIC_BASE_URL`, `core_config`'s: the app reports mail as on only with both.)
fn email_sender(env: &Env) -> Result<Option<Arc<dyn EmailSender>>> {
    let Some(cfg) = EmailConfig::from_lookup(|name| text(env, name)).map_err(|e| Error::RustError(e.to_string()))?
    else {
        return Ok(None);
    };
    let binding = env.send_email(EMAIL_BINDING).map_err(|_| {
        Error::RustError(format!("THRUST_EMAIL_FROM is set but the Worker has no `{EMAIL_BINDING}` send_email binding"))
    })?;
    // MUTANT (control run only): the sender is configured and the binding is there, and the mail service is not handed to the app, as if
    // the composition root had forgotten to. The `mail` pass must refuse to run against it (its vacuity guard: the app says mail is off).
    #[cfg(feature = "mutant_worker_email_unwired")]
    {
        let _ = (binding, cfg);
        Ok(None)
    }
    #[cfg(not(feature = "mutant_worker_email_unwired"))]
    Ok(Some(Arc::new(CfEmail::new(binding, cfg))))
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

    let cfg = core_config(env)?;
    let rng: Arc<dyn Rng> = Arc::new(JsRng);
    let hasher: Arc<dyn PasswordHasher> = Arc::new(Argon2idHasher::new(cfg.argon2));
    let oidc = oidc_provider(env)?;
    let email = email_sender(env)?;

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
                    email,
                },
                cfg,
                SystemScope::mint(),
            )
        }
        #[cfg(feature = "memory")]
        "memory" => memory::app(clock, ids, background, rng, hasher, memory::Optional { oidc, email }, cfg),
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
// queue consumer (ADR-0070, FR-21): the events queue and the dead-letter queue behind it
// ---------------------------------------------------------------------------------------------

/// The Worker consumes two queues. The decisions about a message (is it an event, what did the consumer answer, retry or set aside, and
/// what is a dead letter) are `thrust_ports::settle` and `settle_dead_letter`, which are tested natively and are the same policy as the
/// in-process queue's. What is here is only what needs the platform: reading a message, and acknowledging it, delivering it again or
/// sending its dead letter.
///
/// A message the consumer gives up on is set aside by sending a `DeadLetterMessage` to the dead-letter queue, and the Worker's consumer
/// of that queue writes the row. One table then holds what the Worker set aside and what Cloudflare moved there itself after its own
/// retries, and a failed write is simply delivered again.
#[event(queue)]
async fn queue(batch: MessageBatch<String>, env: Env, _ctx: Context) -> Result<()> {
    let (app, _) = build_app(&env, Arc::new(LocalBackground))?;
    let consumer = ActivityConsumer(app);
    // Which of the two queues this batch is from: by the name `THRUST_EVENTS_DLQ` gives the dead-letter queue (the deploy's parity test
    // holds it equal to the `dead_letter_queue` of the events consumer). A Worker without the name treats every batch as the events
    // queue's, and sets nothing aside (see `send_dead_letter`), so that a misconfiguration cannot make a message go round.
    let dead_letter_queue = text(&env, "THRUST_EVENTS_DLQ").filter(|name| !name.is_empty());
    let from_dead_letter_queue = dead_letter_queue.as_deref() == Some(batch.queue().as_str());
    // `raw_iter`, not `messages`: `messages` reads the whole batch as strings and, when one body is not a string (a producer's JSON or
    // V8 message), fails the batch, so that every message in it is delivered again for one bad one. A raw message is read on its own.
    for msg in batch.raw_iter() {
        if from_dead_letter_queue {
            dead_letter_queue_message(&consumer, &msg).await;
        } else {
            events_queue_message(&consumer, &env, &msg).await;
        }
    }
    Ok(())
}

/// A message's body as text. Everything this repository sends is text (`CfQueue` sends an envelope as a string); a body that some other
/// producer sent as JSON or V8 is written out as JSON, which is the nearest thing to what it was.
fn body_text(msg: &RawMessage) -> String {
    let body = msg.body();
    body.as_string()
        .or_else(|| worker::js_sys::JSON::stringify(&body).ok().map(String::from))
        .unwrap_or_else(|| format!("{body:?}"))
}

fn retry_in(delay: std::time::Duration) -> QueueRetryOptions {
    QueueRetryOptionsBuilder::new().with_delay_seconds(u32::try_from(delay.as_secs()).unwrap_or(u32::MAX)).build()
}

async fn events_queue_message(consumer: &ActivityConsumer, env: &Env, msg: &RawMessage) {
    let deliveries = msg.attempts();
    match settle(consumer, EVENTS_TOPIC, &msg.id(), &body_text(msg), deliveries).await {
        Settled::Ack => msg.ack(),
        Settled::Retry { delay, error } => {
            console_error!(
                "message {} failed (delivery {deliveries}), delivered again in {}s: {error}",
                msg.id(),
                delay.as_secs()
            );
            msg.retry_with_options(&retry_in(delay));
        }
        Settled::SetAside(letter) => match send_dead_letter(env, &letter).await {
            Ok(()) => msg.ack(),
            Err(e) => {
                // Not acknowledged, so delivered again; and if that keeps failing the queue's own `max_retries` moves the original
                // message to the dead-letter queue, where it becomes a row all the same.
                console_error!("message {} could not be set aside ({}): {e}", msg.id(), letter.error);
                msg.retry_with_options(&retry_in(thrust_ports::retry_delay(deliveries)));
            }
        },
    }
}

async fn dead_letter_queue_message(consumer: &ActivityConsumer, msg: &RawMessage) {
    match settle_dead_letter(consumer, EVENTS_TOPIC, &msg.id(), &body_text(msg)).await {
        Ok(()) => msg.ack(),
        Err(e) => {
            // No queue stands behind this one, so its own `max_retries` (set high) is all the patience there is.
            console_error!("dead letter {} could not be written: {e}", msg.id());
            msg.retry_with_options(&retry_in(thrust_ports::retry_delay(msg.attempts())));
        }
    }
}

/// Send a message's dead letter to the dead-letter queue. It fails when the Worker does not know the queue's name
/// (`THRUST_EVENTS_DLQ`), because without it the Worker cannot tell the dead-letter queue's batches from the events queue's and would
/// send what it consumed from the one back onto the same one.
async fn send_dead_letter(env: &Env, letter: &NewDeadLetter) -> Result<()> {
    if text(env, "THRUST_EVENTS_DLQ").filter(|name| !name.is_empty()).is_none() {
        return Err(Error::RustError("THRUST_EVENTS_DLQ is not set".into()));
    }
    let message = DeadLetterMessage::new(
        letter.source_id.clone().unwrap_or_default(),
        letter.topic.clone(),
        letter.event_id,
        letter.body.clone(),
        letter.error.clone(),
        letter.attempts.unwrap_or(1),
    );
    let text = message.to_text().map_err(|e| Error::RustError(e.to_string()))?;
    // as `CfQueue` sends an envelope: a string, so that the text arrives as it was written
    let builder = RawMessageBuilder::new(worker::wasm_bindgen::JsValue::from_str(&text));
    env.queue("EVENTS_DLQ")?.send_raw(builder.build_with_content_type(QueueContentType::Text)).await
}

// ---------------------------------------------------------------------------------------------
// cron (ADR-0072): the schedules of `thrust_core::SCHEDULES`: the outbox sweeper of ADR-0046 and the daily hard delete of issue #59
// ---------------------------------------------------------------------------------------------

/// Runs the job of the trigger that fired. Each trigger is one of the `triggers.crons` of the wrangler files, and the job is whatever
/// `SCHEDULES` says it runs: this handler reads the trigger's own cron string, so adding a schedule is a line in that table and in the
/// files (`deploy_parity.py` holds them to it). A string the table does not have runs nothing, and says so, so that a trigger nobody
/// wired is a line in the log and not a second sweep.
#[event(scheduled)]
async fn scheduled(event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    console_error_panic_hook::set_once();
    let cron = event.cron();
    let app = match build_app(&env, Arc::new(LocalBackground)) {
        Ok((v, _)) => v,
        Err(e) => {
            console_error!("scheduled cron='{cron}': cannot build app: {e}");
            return;
        }
    };
    match thrust_core::run_cron(&app, &cron).await {
        Some(Ok(report)) => console_log!("scheduled cron='{cron}': {report:?}"),
        Some(Err(e)) => console_error!("scheduled cron='{cron}': the job failed: {e}"),
        None => console_error!("scheduled: no schedule has the cron '{cron}', so nothing ran"),
    }
}
