//! In-isolate adapter set (dev/test): proves the dual build before any binding is involved.

use std::sync::{Arc, OnceLock};

use thrust_adapters_cloudflare::JsClock;
use thrust_adapters_memory::*;
use thrust_core::{ActivityConsumer, App, CoreConfig, Ports};
use thrust_ports::*;

/// Process-lifetime state of the in-memory adapter set (one isolate == one "server").
struct MemShared {
    db: MemoryDatabase,
    sessions: Arc<MemorySessionStore>,
    blobs: Arc<MemoryBlobStore>,
    limiter: Arc<MemoryRateLimiter>,
}

static MEM: OnceLock<MemShared> = OnceLock::new();

pub fn app(clock: Arc<dyn Clock>, ids: Arc<dyn IdGen>, background: Arc<dyn Background>, cfg: CoreConfig) -> App {
    let m = MEM.get_or_init(|| MemShared {
        db: MemoryDatabase::new(),
        sessions: Arc::new(MemorySessionStore::new(Arc::new(JsClock))),
        blobs: Arc::new(MemoryBlobStore::new()),
        limiter: Arc::new(MemoryRateLimiter::new(Arc::new(JsClock))),
    });
    let slot: ConsumerSlot = Default::default();
    let queue: Arc<dyn Queue> = Arc::new(MemoryQueue::new(background.clone(), slot.clone()));
    let app = App {
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
        cfg,
    };
    slot.set(Arc::new(ActivityConsumer(app.clone()))).ok();
    app
}
