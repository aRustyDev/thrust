//! Cloudflare Worker composition root.
//!
//! Compile-time adapter set = this crate's dependency list (ADR-0007); the `THRUST_ADAPTERS`
//! var only chooses between sets that were compiled in (memory | postgres).

use std::sync::{Arc, OnceLock};

use axum::body::Body;
use axum::http::Response;
use thrust_adapters_cloudflare::*;
use thrust_adapters_memory::*;
use thrust_api::{ApiConfig, InternalErrorDetail};
use thrust_core::{ActivityConsumer, App, CoreConfig, Ports};
use thrust_ports::*;
use tower_service::Service;
use worker::*;

// Explicit imports win over the two globs: both `worker` and `thrust_ports` export a `Queue`.
use thrust_ports::Queue;

/// Process-lifetime state of the in-memory adapter set (one isolate == one "server").
struct MemShared {
    db: MemoryDatabase,
    sessions: Arc<MemorySessionStore>,
    blobs: Arc<MemoryBlobStore>,
    limiter: Arc<MemoryRateLimiter>,
}

static MEM: OnceLock<MemShared> = OnceLock::new();
static ROUTER: OnceLock<axum::Router> = OnceLock::new();

fn text(env: &Env, name: &str) -> Option<String> {
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
    CoreConfig {
        create_rate_limit: num(env, "THRUST_CREATE_RATE_LIMIT", 1000),
        rate_window_secs: num(env, "THRUST_RATE_WINDOW_SECS", 60),
        outbox_fastpath: flag(env, "THRUST_OUTBOX_FASTPATH", true),
        ..CoreConfig::default()
    }
}

fn api_config(env: &Env, adapters: &str) -> ApiConfig {
    ApiConfig {
        test_routes: flag(env, "THRUST_TEST_ROUTES", false),
        admin_token: text(env, "THRUST_ADMIN_TOKEN"),
        target: "worker".into(),
        adapters: adapters.to_string(),
    }
}

/// Build the per-request `App`. Request-scoped because `ctx.waitUntil` and (later) Hyperdrive
/// sockets belong to one request in workerd.
fn build_app(env: &Env, ctx: Arc<Context>) -> Result<(App, String)> {
    let adapters = text(env, "THRUST_ADAPTERS").unwrap_or_else(|| "memory".into());
    let clock: Arc<dyn Clock> = Arc::new(JsClock);
    let ids: Arc<dyn IdGen> = Arc::new(JsIdGen);
    let background: Arc<dyn Background> = Arc::new(WaitUntilBackground(ctx));
    let slot: ConsumerSlot = Default::default();
    let queue: Arc<dyn Queue> = Arc::new(MemoryQueue::new(background.clone(), slot.clone()));

    let app = match adapters.as_str() {
        "memory" => {
            let m = MEM.get_or_init(|| MemShared {
                db: MemoryDatabase::new(),
                sessions: Arc::new(MemorySessionStore::new(Arc::new(JsClock))),
                blobs: Arc::new(MemoryBlobStore::new()),
                limiter: Arc::new(MemoryRateLimiter::new(Arc::new(JsClock))),
            });
            App {
                ports: Ports {
                    db: Arc::new(m.db.clone()),
                    queue,
                    sessions: m.sessions.clone(),
                    blobs: m.blobs.clone(),
                    limiter: m.limiter.clone(),
                    clock,
                    ids,
                    background,
                },
                cfg: core_config(env),
            }
        }
        other => return Err(Error::RustError(format!("unknown THRUST_ADAPTERS '{other}'"))),
    };
    slot.set(Arc::new(ActivityConsumer(app.clone()))).ok();
    Ok((app, adapters))
}

#[event(fetch)]
async fn fetch(mut req: HttpRequest, env: Env, ctx: Context) -> Result<Response<Body>> {
    console_error_panic_hook::set_once();
    let (app, adapters) = build_app(&env, Arc::new(ctx))?;
    req.extensions_mut().insert(app);
    req.extensions_mut().insert(api_config(&env, &adapters));

    let mut router = ROUTER.get_or_init(thrust_api::router).clone();
    let resp = match router.call(req).await {
        Ok(r) => r,
        Err(never) => match never {},
    };
    if let Some(InternalErrorDetail(d)) = resp.extensions().get::<InternalErrorDetail>() {
        console_error!("internal error: {d}");
    }
    Ok(resp)
}
