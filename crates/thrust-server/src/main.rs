//! Native composition root. Reads its configuration from the environment (secrets only from
//! the environment, never from a file) and wires one adapter set into `thrust_core::App`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Response, StatusCode};
use axum::Extension;
use thrust_adapters_memory::*;
use thrust_adapters_oidc::{OidcConfig, OpenIdProvider};
use thrust_adapters_postgres::{Connector, CountingConnector, PgDatabase, RoundTrips};
use thrust_adapters_selfhosted::*;
use thrust_api::{set_db_round_trips, ApiConfig, InternalErrorDetail};
use thrust_core::{ActivityConsumer, App, Argon2Params, Argon2idHasher, CoreConfig, Ports, SystemScope};
use thrust_ports::*;
use tower_http::catch_panic::CatchPanicLayer;

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_flag(name: &str, default: bool) -> bool {
    match std::env::var(name).as_deref() {
        Ok("1") | Ok("true") | Ok("on") => true,
        Ok("0") | Ok("false") | Ok("off") => false,
        _ => default,
    }
}

fn env_num<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// The OIDC provider, when `THRUST_OIDC_ISSUER`, `THRUST_OIDC_CLIENT_ID` and `THRUST_OIDC_REDIRECT_URI` are all set
/// (`THRUST_OIDC_CLIENT_SECRET` too for a confidential client; a secret, so only from the environment). Some of the
/// three and not all is a mistake, not a reason to start without a login nobody notices is missing.
fn oidc_provider() -> Option<Arc<dyn OidcProvider>> {
    let get = |n: &str| std::env::var(n).ok().filter(|v| !v.is_empty());
    let (issuer, client_id, redirect_uri) =
        (get("THRUST_OIDC_ISSUER"), get("THRUST_OIDC_CLIENT_ID"), get("THRUST_OIDC_REDIRECT_URI"));
    match (issuer, client_id, redirect_uri) {
        (Some(issuer), Some(client_id), Some(redirect_uri)) => {
            let cfg = OidcConfig { issuer, client_id, client_secret: get("THRUST_OIDC_CLIENT_SECRET"), redirect_uri };
            Some(Arc::new(OpenIdProvider::new(cfg, Arc::new(ReqwestFetch::new()))))
        }
        (None, None, None) => None,
        _ => panic!("THRUST_OIDC_ISSUER, THRUST_OIDC_CLIENT_ID and THRUST_OIDC_REDIRECT_URI must be set together"),
    }
}

fn panic_response(err: Box<dyn std::any::Any + Send + 'static>) -> Response<Body> {
    let msg = err
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| err.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into());
    eprintln!("panic caught by CatchPanicLayer: {msg}");
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"error":{"code":"internal","message":"internal error"}}"#))
        .unwrap()
}

#[tokio::main]
async fn main() {
    let listen: SocketAddr = env_or("THRUST_LISTEN", "127.0.0.1:8080").parse().expect("THRUST_LISTEN");
    let adapters = env_or("THRUST_ADAPTERS", "memory");

    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let ids: Arc<dyn IdGen> = Arc::new(NativeIdGen);
    let background: Arc<dyn Background> = Arc::new(TokioBackground);
    let consumer_slot: ConsumerSlot = Default::default();
    let queue: Arc<dyn Queue> = Arc::new(MemoryQueue::new(background.clone(), consumer_slot.clone()));

    // The pool, kept beside the database: with test routes on, each request counts its statements over a connector of its
    // own (issue #39), because this one `PgDatabase` serves every request and a counter inside it would count them all.
    let mut pool: Option<Arc<dyn Connector>> = None;
    let db: Arc<dyn Database> = match adapters.as_str() {
        "memory" => Arc::new(MemoryDatabase::new()),
        "postgres" => {
            // the URL carries the password: environment only
            let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for THRUST_ADAPTERS=postgres");
            let connector =
                thrust_adapters_selfhosted::pg::PoolConnector::from_url(&url, env_num("THRUST_PG_POOL", 16))
                    .expect("pool");
            let connector: Arc<dyn Connector> = Arc::new(connector);
            pool = Some(connector.clone());
            Arc::new(PgDatabase::new(connector))
        }
        other => panic!("unknown THRUST_ADAPTERS '{other}' (this build supports: memory, postgres)"),
    };

    let defaults = CoreConfig::default();
    let cfg = CoreConfig {
        create_rate_limit: env_num("THRUST_CREATE_RATE_LIMIT", 1000),
        rate_window_secs: env_num("THRUST_RATE_WINDOW_SECS", 60),
        outbox_fastpath: env_flag("THRUST_OUTBOX_FASTPATH", true),
        sign_in_rate_limit: env_num("THRUST_SIGNIN_RATE_LIMIT", defaults.sign_in_rate_limit),
        // OFF unless the environment says on: the stock Plane frontend sends the CSRF token only on its auth form posts, so
        // enforcing it on JSON mutations would break it (Q-0050.8). The flip is when the custom frontend replaces it.
        csrf_enforce: env_flag("THRUST_CSRF_ENFORCE", false),
        // OWASP's minimum unless the environment says otherwise (a test target sets a cheap cost)
        argon2: Argon2Params {
            m_cost_kib: env_num("THRUST_ARGON2_M_KIB", defaults.argon2.m_cost_kib),
            t_cost: env_num("THRUST_ARGON2_T", defaults.argon2.t_cost),
            p_cost: env_num("THRUST_ARGON2_P", defaults.argon2.p_cost),
        },
        ..defaults
    };
    let hasher = Arc::new(Argon2idHasher::new(cfg.argon2));

    // This composition root mints the proof for the relay, sweeper, consumer and seed.
    let app = App::new(
        Ports {
            db,
            queue,
            blobs: Arc::new(MemoryBlobStore::new()),
            limiter: Arc::new(MemoryRateLimiter::new(clock.clone())),
            clock,
            ids,
            background,
            rng: Arc::new(NativeRng),
            hasher,
            oidc: oidc_provider(),
        },
        cfg,
        SystemScope::mint(),
    );
    consumer_slot.set(Arc::new(ActivityConsumer(app.clone()))).ok().expect("consumer slot");

    // Cron stand-in: ADR-0072 says the self-hosted Scheduler is an in-process timer.
    let sweep_every: u64 = env_num("THRUST_SWEEP_INTERVAL_SECS", 0);
    if sweep_every > 0 {
        let app = app.clone();
        tokio::spawn(async move {
            let mut t = tokio::time::interval(Duration::from_secs(sweep_every));
            loop {
                t.tick().await;
                if let Err(e) = thrust_core::sweep_outbox(&app, 100).await {
                    eprintln!("sweep failed: {e}");
                }
            }
        });
    }

    let api_cfg = ApiConfig {
        test_routes: env_flag("THRUST_TEST_ROUTES", false),
        admin_token: std::env::var("THRUST_ADMIN_TOKEN").ok(),
        target: "native".into(),
        adapters: adapters.clone(),
    };

    // Plane's `API_KEY_RATE_LIMIT` for `/api/v1` (a key's allowance), in its own spelling: `100000/minute`. Unset is Plane's `60/minute`.
    let v1_cfg = match std::env::var("THRUST_V1_RATE_LIMIT") {
        Ok(spec) => thrust_api_v1::V1Config::parse(&spec)
            .expect("THRUST_V1_RATE_LIMIT is a rate such as 100000/minute (second, minute, hour or day)"),
        Err(_) => thrust_api_v1::V1Config::default(),
    };

    let router = thrust_api::router()
        .merge(thrust_compat_plane::routes())
        .merge(thrust_api_v1::router())
        .layer(axum::middleware::from_fn(log_internal_errors));
    // Where there is a Postgres to count and test routes ask for the count. The layer goes inside `Extension(app)`, so it runs
    // after the extension is in the request and can swap it.
    let router = match pool.filter(|_| api_cfg.test_routes) {
        Some(pool) => router.layer(axum::middleware::from_fn_with_state(pool, count_round_trips)),
        None => router,
    };
    let router = router
        .layer(Extension(app))
        .layer(Extension(api_cfg))
        .layer(Extension(v1_cfg))
        .layer(CatchPanicLayer::custom(panic_response));

    let listener = tokio::net::TcpListener::bind(listen).await.expect("bind");
    eprintln!("thrust-server listening on {listen} adapters={adapters}");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("serve");
}

/// The test routes' statement counter (issue #39). The request gets a clone of the `App` whose database sends its statements
/// through a connector of this request's own, over the shared pool, and the response says how many it had sent when the
/// router answered. Background work a handler leaves running holds the same database and may count after that.
async fn count_round_trips(
    State(pool): State<Arc<dyn Connector>>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response<Body> {
    let mut counted = None;
    if let Some(app) = req.extensions().get::<App>() {
        let rt = RoundTrips::new();
        let mut app = app.clone();
        app.ports.db = Arc::new(PgDatabase::new(Arc::new(CountingConnector::new(pool, rt.clone()))));
        req.extensions_mut().insert(app);
        counted = Some(rt);
    }
    let mut resp = next.run(req).await;
    // no count without a swapped `App`: a request that was not counted has no header, not a zero
    if let Some(rt) = counted {
        set_db_round_trips(&mut resp, rt.get());
    }
    resp
}

async fn log_internal_errors(req: axum::extract::Request, next: axum::middleware::Next) -> Response<Body> {
    let resp = next.run(req).await;
    if let Some(InternalErrorDetail(d)) = resp.extensions().get::<InternalErrorDetail>() {
        eprintln!("internal error: {d}");
    }
    resp
}
