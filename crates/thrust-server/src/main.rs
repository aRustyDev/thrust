//! Native composition root. Reads its configuration from the environment (secrets only from
//! the environment, never from a file) and wires one adapter set into `thrust_core::App`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Response, StatusCode};
use axum::Extension;
use thrust_adapters_memory::*;
use thrust_adapters_selfhosted::*;
use thrust_api::{ApiConfig, InternalErrorDetail};
use thrust_core::{ActivityConsumer, App, CoreConfig, Ports};
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

    let db: Arc<dyn Database> = match adapters.as_str() {
        "memory" => Arc::new(MemoryDatabase::new()),
        other => panic!("unknown THRUST_ADAPTERS '{other}' (this build supports: memory)"),
    };

    let cfg = CoreConfig {
        create_rate_limit: env_num("THRUST_CREATE_RATE_LIMIT", 1000),
        rate_window_secs: env_num("THRUST_RATE_WINDOW_SECS", 60),
        outbox_fastpath: env_flag("THRUST_OUTBOX_FASTPATH", true),
        ..CoreConfig::default()
    };

    let app = App {
        ports: Ports {
            db,
            queue,
            sessions: Arc::new(MemorySessionStore::new(clock.clone())),
            blobs: Arc::new(MemoryBlobStore::new()),
            limiter: Arc::new(MemoryRateLimiter::new(clock.clone())),
            clock,
            ids,
            background,
        },
        cfg,
    };
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

    let router = thrust_api::router()
        .layer(axum::middleware::from_fn(log_internal_errors))
        .layer(Extension(app))
        .layer(Extension(api_cfg))
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

async fn log_internal_errors(req: axum::extract::Request, next: axum::middleware::Next) -> Response<Body> {
    let resp = next.run(req).await;
    if let Some(InternalErrorDetail(d)) = resp.extensions().get::<InternalErrorDetail>() {
        eprintln!("internal error: {d}");
    }
    resp
}
