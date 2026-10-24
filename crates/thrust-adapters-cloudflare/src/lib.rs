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
            self.0
                .send_raw(b.build_with_content_type(worker::QueueContentType::Text))
                .await
                .map_err(q_err)
        })
        .await
    }
}

// ---------------------------------------------------------------------------------------------
// Durable Object (SQLite storage) -> RateLimiter
// ---------------------------------------------------------------------------------------------

/// One DO instance per rate-limit key; the DO class lives in `thrust-worker` (the only crate
/// that exports DO classes). Wire protocol: `GET /hit?limit=&window=` -> JSON.
pub struct DoRateLimiter(pub worker::ObjectNamespace);

#[derive(serde::Deserialize)]
struct DoHit {
    allowed: bool,
    remaining: u32,
    retry_after_secs: u64,
}

#[async_trait]
impl RateLimiter for DoRateLimiter {
    async fn hit(&self, key: &str, limit: u32, window_secs: u64) -> Result<RateDecision, RateError> {
        let e = |x: worker::Error| RateError::Backend(x.to_string());
        SendFuture::new(async move {
            let stub = self.0.get_by_name(key).map_err(e)?;
            let url = format!("https://rate-limiter.internal/hit?limit={limit}&window={window_secs}");
            let req = worker::Request::new(&url, worker::Method::Get).map_err(e)?;
            let mut resp = stub.fetch_with_request(req).await.map_err(e)?;
            if resp.status_code() != 200 {
                return Err(RateError::Backend(format!("rate-limiter DO answered {}", resp.status_code())));
            }
            let hit: DoHit = resp.json().await.map_err(e)?;
            Ok(if hit.allowed {
                RateDecision::Allowed { remaining: hit.remaining }
            } else {
                RateDecision::Limited { retry_after_secs: hit.retry_after_secs }
            })
        })
        .await
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
            cfg.host(&hd.host()).port(hd.port()).user(&hd.user()).password(hd.password()).dbname(&hd.database());
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
        if let Some(c) = self.shared.lock().unwrap().clone() {
            return Ok(Box::new(c));
        }
        let fresh = Arc::new(self.connect().await?);
        let mut slot = self.shared.lock().unwrap();
        // Two first callers can race; keep the first connection, the loser is dropped (closed).
        let keep = slot.get_or_insert_with(|| fresh.clone()).clone();
        Ok(Box::new(keep))
    }
}
