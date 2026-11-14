//! Cloudflare (workers-rs) adapters.
//!
//! Every adapter that touches a JS object returns a `Send` future by wrapping the JS-backed part
//! in `worker::send::SendFuture`. That is sound because a Worker isolate is single-threaded
//! (workers-rs relies on the same argument for its own `unsafe impl Send` on bindings). It is
//! what lets one `Send`-bounded set of ports serve both a shared axum router and the Worker.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use thrust_adapters_postgres::{ClientHandle, Connector};
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_ports::*;
use worker::send::SendFuture;

// ---------------------------------------------------------------------------------------------
// Ambient: clock, ids, background
// ---------------------------------------------------------------------------------------------

/// Wall clock from the JS `Date` (workerd freezes it during pure CPU work; that is fine here).
pub struct JsClock;

impl Clock for JsClock {
    #[allow(
        clippy::expect_used,
        reason = "the JS clock is always inside chrono's range (about 262,000 years either side of 1970), and the port returns no \
                  error. A fallback such as the epoch would make every session expiry look far off, so a clock that cannot be \
                  read fails the request instead of answering wrongly"
    )]
    fn now(&self) -> Timestamp {
        let ms = worker::Date::now().as_millis() as i64;
        DateTime::from_timestamp_millis(ms).expect("valid epoch millis")
    }
}

/// UUID v7 from the JS clock + `crypto.getRandomValues` (uuid's `js` feature).
pub struct JsIdGen;

impl IdGen for JsIdGen {
    fn new_uuid(&self) -> Uuid {
        let ms = worker::Date::now().as_millis();
        let ts = uuid::Timestamp::from_unix(uuid::NoContext, ms / 1000, ((ms % 1000) * 1_000_000) as u32);
        Uuid::new_v7(ts)
    }
}

/// The `Rng` port on `crypto.getRandomValues` (through `getrandom`'s `wasm_js` backend).
pub struct JsRng;

impl Rng for JsRng {
    fn fill(&self, buf: &mut [u8]) -> Result<(), RngError> {
        getrandom::fill(buf).map_err(|e| RngError(e.to_string()))
    }
}

/// `worker::Fetch` as the `Fetch` port: a subrequest from the Worker. `redirect: manual`, so a redirect is an answer
/// like any other.
pub struct WorkerFetch;

#[async_trait]
impl Fetch for WorkerFetch {
    async fn send(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        let e = |x: worker::Error| FetchError(x.to_string());
        SendFuture::new(async move {
            let headers = worker::Headers::new();
            for (k, v) in &req.headers {
                headers.append(k, v).map_err(e)?;
            }
            let mut init = worker::RequestInit::new();
            init.with_method(worker::Method::from(req.method.clone()))
                .with_headers(headers)
                .with_redirect(worker::RequestRedirect::Manual);
            if !req.body.is_empty() {
                init.with_body(Some(worker::js_sys::Uint8Array::from(req.body.as_slice()).into()));
            }
            let request = worker::Request::new_with_init(&req.url, &init).map_err(e)?;
            let mut res = worker::Fetch::Request(request).send().await.map_err(e)?;
            let status = res.status_code();
            let headers = res.headers().entries().collect();
            let body = res.bytes().await.map_err(e)?;
            Ok(FetchResponse { status, headers, body })
        })
        .await
    }
}

/// `Background` on `ctx.waitUntil`: the isolate stays alive until the future completes.
pub struct WaitUntilBackground(pub Arc<worker::Context>);

impl Background for WaitUntilBackground {
    fn spawn(&self, fut: BoxFuture<()>) {
        self.0.wait_until(fut);
    }
}

/// `Background` for entrypoints without a `fetch` context (queue consumer, cron): a local task.
/// Neither entrypoint relies on it today (the consumer and the sweeper await their I/O inline).
pub struct LocalBackground;

impl Background for LocalBackground {
    fn spawn(&self, fut: BoxFuture<()>) {
        wasm_bindgen_futures::spawn_local(fut);
    }
}

// ---------------------------------------------------------------------------------------------
// R2 -> BlobStore
// ---------------------------------------------------------------------------------------------

pub struct R2BlobStore(pub worker::Bucket);

fn r2_err(e: impl std::fmt::Display) -> BlobError {
    BlobError::Backend(e.to_string())
}

#[async_trait]
impl BlobStore for R2BlobStore {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: Option<String>) -> Result<(), BlobError> {
        let key = key.to_string();
        SendFuture::new(async move {
            let mut b = self.0.put(key, worker::Data::Bytes(bytes));
            if let Some(ct) = content_type {
                b = b.http_metadata(worker::HttpMetadata { content_type: Some(ct), ..Default::default() });
            }
            b.execute().await.map(|_| ()).map_err(r2_err)
        })
        .await
    }

    async fn head(&self, key: &str) -> Result<Option<BlobHead>, BlobError> {
        let key = key.to_string();
        SendFuture::new(async move {
            let Some(obj) = self.0.head(key).await.map_err(r2_err)? else {
                return Ok(None);
            };
            Ok(Some(BlobHead { size: obj.size(), content_type: obj.http_metadata().content_type }))
        })
        .await
    }

    async fn get(&self, key: &str) -> Result<Option<Blob>, BlobError> {
        let key = key.to_string();
        SendFuture::new(async move {
            let Some(obj) = self.0.get(key).execute().await.map_err(r2_err)? else {
                return Ok(None);
            };
            let content_type = obj.http_metadata().content_type;
            let Some(body) = obj.body() else {
                return Ok(None);
            };
            let bytes = body.bytes().await.map_err(r2_err)?;
            Ok(Some(Blob { bytes, content_type }))
        })
        .await
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        let key = key.to_string();
        SendFuture::new(async move { self.0.delete(key).await.map_err(r2_err) }).await
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>, BlobError> {
        let prefix = prefix.to_string();
        SendFuture::new(async move {
            let objs = self.0.list().prefix(prefix).execute().await.map_err(r2_err)?;
            Ok(objs.objects().into_iter().map(|o| o.key()).collect())
        })
        .await
    }
}

// ---------------------------------------------------------------------------------------------
// Queues producer -> Queue
// ---------------------------------------------------------------------------------------------

pub struct CfQueue(pub worker::Queue);

fn q_err(e: impl std::fmt::Display) -> QueueError {
    QueueError::Backend(e.to_string())
}

/// The envelope is sent as a JSON *text* message. Sending the struct directly goes through
/// serde-wasm-bindgen, which turns `serde_json::Value` objects into JS `Map`s that
/// `JSON.stringify` then renders as `{}` (a silent payload loss); text content type avoids it.
pub fn encode_envelope(env: &Envelope) -> Result<String, QueueError> {
    serde_json::to_string(env).map_err(q_err)
}

#[async_trait]
impl Queue for CfQueue {
    async fn send(&self, _topic: &str, msg: &Envelope, opts: SendOpts) -> Result<(), QueueError> {
        let text = encode_envelope(msg)?;
        let delay = opts.delay;
        SendFuture::new(async move {
            let mut b = worker::RawMessageBuilder::new(worker::wasm_bindgen::JsValue::from_str(&text));
            if let Some(d) = delay {
                b = b.delay_seconds(d.as_secs().min(86_400) as u32);
            }
            self.0.send_raw(b.build_with_content_type(worker::QueueContentType::Text)).await.map_err(q_err)
        })
        .await
    }
}

// ---------------------------------------------------------------------------------------------
// Workers Rate Limiting binding -> RateLimiter
// ---------------------------------------------------------------------------------------------

/// The periods a Workers Rate Limiting binding counts over, in seconds. The binding takes no other (ADR-0033, D-007), so a window
/// the app is configured with has to be one of these for the binding's own `period` to be the window the app reports.
pub const BINDING_PERIODS_SECS: [u64; 2] = [10, 60];

/// `Ok` when `secs` is a period the binding can count over, and the refusal to start with when it is not. `setting` names where
/// the window came from, so the message says which variable to fix.
pub fn check_binding_period(setting: &str, secs: u64) -> Result<(), String> {
    if BINDING_PERIODS_SECS.contains(&secs) {
        Ok(())
    } else {
        Err(format!(
            "{setting} is a window of {secs} s, but a Workers Rate Limiting binding counts over 10 s or 60 s only (ADR-0033, D-007)"
        ))
    }
}

/// What a limiter key counts, which is the binding that holds its allowance. The core and the v1 routes build their keys as
/// `signin:<email>` and `v1:<api key>`: the text before the first `:` is the class. (A `create:<user>` class, the slice's work item
/// creates, went with that use-case in step 5c-3 of #58: a key that starts with it is a key of no class now.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyClass {
    /// Sign-in attempts, per email.
    SignIn,
    /// A request on `/api/v1`, per API key.
    V1,
}

impl KeyClass {
    /// The class of `key`, or `None` for a key no binding is configured for.
    pub fn of(key: &str) -> Option<Self> {
        match key.split_once(':')?.0 {
            "signin" => Some(Self::SignIn),
            "v1" => Some(Self::V1),
            _ => None,
        }
    }
}

/// What the binding's answer means to the port.
///
/// The binding says only whether the request is let through. A refusal is `Limited`, and the caller is told to wait one whole
/// period (`window_secs`): the binding reports no time left in its window, so the period is the one honest upper bound.
///
/// An allowance is `Allowed`, and `remaining` is `limit - 1`: the allowance as it stands if this were the first request of the
/// window. The binding reports no count, so any number here is a guess. This one is an upper bound, so it never tells a client
/// that its allowance is spent when it is not, and the client finds out it is by the 429 and its `Retry-After` (D-007).
pub fn decision(success: bool, limit: u32, window_secs: u64) -> RateDecision {
    if success {
        RateDecision::Allowed { remaining: limit.saturating_sub(1) }
    } else {
        RateDecision::Limited { retry_after_secs: window_secs.max(1) }
    }
}

/// The [`RateLimiter`] port on two Workers Rate Limiting bindings, one per [`KeyClass`].
///
/// A binding's limit and period are in its `[[ratelimits]]` block in the wrangler config, not in the request, so the binding is
/// the source of truth for the allowance and the `limit` argument of [`RateLimiter::hit`] is not used here. Counting is per
/// Cloudflare location and approximate (D-007): a limit of 3 has let 4 through. No `SendFuture` here: workers-rs wraps the JS
/// future inside `RateLimiter::limit` itself, so the call is already `Send`.
pub struct CfRateLimiter {
    pub signin: worker::RateLimiter,
    pub v1: worker::RateLimiter,
}

impl CfRateLimiter {
    fn binding(&self, class: KeyClass) -> &worker::RateLimiter {
        match class {
            KeyClass::SignIn => &self.signin,
            KeyClass::V1 => &self.v1,
        }
    }
}

#[async_trait]
impl RateLimiter for CfRateLimiter {
    async fn hit(&self, key: &str, limit: u32, window_secs: u64) -> Result<RateDecision, RateError> {
        // A key of no class is a caller this adapter was not given a binding for: an error, and never a free pass. The key is not
        // in the message, because a sign-in key holds an email address.
        let class = KeyClass::of(key)
            .ok_or_else(|| RateError::Backend("a rate-limit key must start with `signin:` or `v1:`".to_string()))?;
        let outcome =
            self.binding(class).limit(key.to_string()).await.map_err(|e| RateError::Backend(e.to_string()))?;
        Ok(decision(outcome.success, limit, window_secs))
    }
}

// ---------------------------------------------------------------------------------------------
// Hyperdrive -> Postgres connection strategy
// ---------------------------------------------------------------------------------------------

/// Opens `tokio_postgres` connections over the Hyperdrive binding's `Socket`.
///
/// workerd ties a socket to the request that opened it, so a `HyperdriveConnector` (and the
/// `App` holding it) is created per request. Reads share one lazily-opened connection;
/// transactions open their own (Hyperdrive pools the upstream side).
pub struct HyperdriveConnector {
    hd: worker::Hyperdrive,
    shared: Mutex<Option<Arc<tokio_postgres::Client>>>,
}

impl HyperdriveConnector {
    pub fn new(hd: worker::Hyperdrive) -> Self {
        Self { hd, shared: Mutex::new(None) }
    }

    async fn connect(&self) -> Result<tokio_postgres::Client, DbError> {
        let be = |m: String| DbError::Backend(m);
        let hd = &self.hd;
        SendFuture::new(async move {
            let socket = hd.connect().map_err(|e| be(format!("hyperdrive connect: {e}")))?;
            let mut cfg = tokio_postgres::Config::new();
            cfg.host(hd.host()).port(hd.port()).user(hd.user()).password(hd.password()).dbname(hd.database());
            let (client, conn) = cfg
                .connect_raw(socket, tokio_postgres::NoTls)
                .await
                .map_err(|e| be(format!("postgres handshake: {e}")))?;
            // The connection driver runs until the last `Client` clone is dropped.
            wasm_bindgen_futures::spawn_local(async move {
                let _ = conn.await;
            });
            Ok(client)
        })
        .await
    }
}

#[async_trait]
impl Connector for HyperdriveConnector {
    async fn checkout(&self, exclusive: bool) -> Result<Box<dyn ClientHandle>, DbError> {
        if exclusive {
            return Ok(Box::new(Arc::new(self.connect().await?)));
        }
        // A Worker isolate is single-threaded and `panic=abort`, so this mutex is never poisoned; if it ever were, the slot is
        // an `Option<Arc<Client>>` that no panic can leave half-written, and carrying on is right.
        if let Some(c) = self.shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone() {
            return Ok(Box::new(c));
        }
        let fresh = Arc::new(self.connect().await?);
        let mut slot = self.shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // Two first callers can race; keep the first connection, the loser is dropped (closed).
        let keep = slot.get_or_insert_with(|| fresh.clone()).clone();
        Ok(Box::new(keep))
    }
}
