//! In-memory adapters. They exist to (1) prove the dual build before any platform crate is
//! involved and (2) give the contract suite a reference implementation to compare the real
//! adapters against. State lives behind `std::sync::Mutex` (single-threaded on wasm32).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use thrust_contracts::*;
use thrust_ports::*;

// ---------------------------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct DbState {
    projects: HashMap<ProjectId, WorkspaceId>,
    memberships: HashMap<(ProjectId, UserId), Role>,
    /// project -> (last sequence_id handed out, last sort_order handed out). Never decrements.
    counters: HashMap<ProjectId, (i64, i64)>,
    work_items: Vec<WorkItem>,
    outbox: Vec<OutboxRow>,
    activity: Vec<Activity>,
    activity_events: HashSet<EventId>,
}

impl DbState {
    fn unique_violation(&self, item: &WorkItem) -> bool {
        match (&item.external_source, &item.external_id) {
            (Some(src), Some(ext)) => self.work_items.iter().any(|w| {
                w.project_id == item.project_id
                    && w.deleted_at.is_none()
                    && w.external_source.as_ref() == Some(src)
                    && w.external_id.as_ref() == Some(ext)
            }),
            _ => false,
        }
    }
}

#[derive(Clone, Default)]
pub struct MemoryDatabase {
    state: Arc<Mutex<DbState>>,
}

impl MemoryDatabase {
    pub fn new() -> Self {
        Self::default()
    }
}

fn is_visible(w: &WorkItem) -> bool {
    w.deleted_at.is_none() && w.archived_at.is_none() && !w.is_draft
}

#[async_trait]
impl Database for MemoryDatabase {
    async fn membership(
        &self,
        ws: WorkspaceId,
        project: ProjectId,
        user: UserId,
    ) -> Result<Membership, DbError> {
        let s = self.state.lock().unwrap();
        if s.projects.get(&project) != Some(&ws) {
            return Ok(Membership::NoSuchProject);
        }
        Ok(match s.memberships.get(&(project, user)) {
            Some(r) => Membership::Member(*r),
            None => Membership::NotAMember,
        })
    }

    async fn begin(&self) -> Result<Box<dyn Tx>, DbError> {
        Ok(Box::new(MemoryTx {
            state: self.state.clone(),
            pending_items: Vec::new(),
            pending_outbox: Vec::new(),
            pending_deletes: Vec::new(),
        }))
    }

    async fn list_work_items(
        &self,
        ws: WorkspaceId,
        project: ProjectId,
        vis: Visibility,
    ) -> Result<Vec<WorkItem>, DbError> {
        let s = self.state.lock().unwrap();
        let mut v: Vec<WorkItem> = s
            .work_items
            .iter()
            .filter(|w| w.workspace_id == ws && w.project_id == project)
            .filter(|w| vis == Visibility::WithDeleted || is_visible(w))
            .cloned()
            .collect();
        v.sort_by_key(|w| w.sequence_id);
        Ok(v)
    }

    async fn get_work_item(
        &self,
        ws: WorkspaceId,
        project: ProjectId,
        id: WorkItemId,
        vis: Visibility,
    ) -> Result<Option<WorkItem>, DbError> {
        let s = self.state.lock().unwrap();
        Ok(s.work_items
            .iter()
            .find(|w| {
                w.workspace_id == ws
                    && w.project_id == project
                    && w.id == id
                    && (vis == Visibility::WithDeleted || is_visible(w))
            })
            .cloned())
    }

    async fn insert_activity(&self, a: &Activity) -> Result<bool, DbError> {
        let mut s = self.state.lock().unwrap();
        if !s.activity_events.insert(a.event_id) {
            return Ok(false);
        }
        s.activity.push(a.clone());
        Ok(true)
    }

    async fn list_activity(
        &self,
        ws: WorkspaceId,
        project: ProjectId,
        entity: Option<WorkItemId>,
    ) -> Result<Vec<Activity>, DbError> {
        let s = self.state.lock().unwrap();
        let mut v: Vec<Activity> = s
            .activity
            .iter()
            .filter(|a| a.workspace_id == ws && a.project_id == project)
            .filter(|a| entity.map_or(true, |e| a.entity_id == e.as_uuid()))
            .cloned()
            .collect();
        v.sort_by_key(|a| a.created_at);
        Ok(v)
    }

    async fn undispatched_outbox(&self, limit: u32) -> Result<Vec<OutboxRow>, DbError> {
        let s = self.state.lock().unwrap();
        Ok(s.outbox
            .iter()
            .filter(|r| r.dispatched_at.is_none())
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn mark_outbox_dispatched(&self, event: EventId, at: Timestamp) -> Result<(), DbError> {
        let mut s = self.state.lock().unwrap();
        for r in s.outbox.iter_mut().filter(|r| r.envelope.event_id == event) {
            r.dispatched_at = Some(at);
            r.attempts += 1;
        }
        Ok(())
    }

    async fn outbox_depth(&self) -> Result<u64, DbError> {
        let s = self.state.lock().unwrap();
        Ok(s.outbox.iter().filter(|r| r.dispatched_at.is_none()).count() as u64)
    }

    async fn seed(&self, f: &SeedFixture) -> Result<(), DbError> {
        let mut s = self.state.lock().unwrap();
        for p in &f.projects {
            s.projects.insert(p.id, p.workspace_id);
        }
        for m in &f.memberships {
            s.memberships.insert((m.project_id, m.user_id), m.role);
        }
        Ok(())
    }
}

/// Buffers writes and applies them atomically on `commit`. `allocate` is *not* transactional
/// (like a Postgres sequence, a rollback leaves a gap; documented in ADR-0043).
struct MemoryTx {
    state: Arc<Mutex<DbState>>,
    pending_items: Vec<WorkItem>,
    pending_outbox: Vec<OutboxRow>,
    pending_deletes: Vec<(ProjectId, WorkItemId, Timestamp)>,
}

#[async_trait]
impl Tx for MemoryTx {
    async fn allocate(&mut self, _ws: WorkspaceId, project: ProjectId) -> Result<Allocation, DbError> {
        let mut s = self.state.lock().unwrap();
        let c = s.counters.entry(project).or_insert((0, 0));
        c.0 += 1;
        c.1 += 10_000;
        Ok(Allocation { sequence_id: c.0, sort_order: c.1 })
    }

    async fn insert_work_item(&mut self, item: &WorkItem) -> Result<(), DbError> {
        let s = self.state.lock().unwrap();
        let clash_pending = self.pending_items.iter().any(|w| {
            w.project_id == item.project_id
                && item.external_id.is_some()
                && w.external_source == item.external_source
                && w.external_id == item.external_id
        });
        if s.unique_violation(item) || clash_pending {
            return Err(DbError::Conflict("work item external_id already exists".into()));
        }
        drop(s);
        self.pending_items.push(item.clone());
        Ok(())
    }

    async fn soft_delete_work_item(
        &mut self,
        ws: WorkspaceId,
        project: ProjectId,
        id: WorkItemId,
        at: Timestamp,
    ) -> Result<bool, DbError> {
        let s = self.state.lock().unwrap();
        let exists = s
            .work_items
            .iter()
            .any(|w| w.workspace_id == ws && w.project_id == project && w.id == id && w.deleted_at.is_none());
        drop(s);
        if exists {
            self.pending_deletes.push((project, id, at));
        }
        Ok(exists)
    }

    async fn insert_outbox(&mut self, row: &OutboxRow) -> Result<(), DbError> {
        self.pending_outbox.push(row.clone());
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<(), DbError> {
        let this = *self;
        let mut s = this.state.lock().unwrap();
        // Re-check uniqueness under the lock (the check at insert time raced other commits).
        for item in &this.pending_items {
            if s.unique_violation(item) {
                return Err(DbError::Conflict("work item external_id already exists".into()));
            }
        }
        for item in this.pending_items {
            s.work_items.push(item);
        }
        for (project, id, at) in this.pending_deletes {
            if let Some(w) = s
                .work_items
                .iter_mut()
                .find(|w| w.project_id == project && w.id == id && w.deleted_at.is_none())
            {
                w.deleted_at = Some(at);
            }
        }
        s.outbox.extend(this.pending_outbox);
        Ok(())
    }

    async fn rollback(self: Box<Self>) -> Result<(), DbError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Queue: producer pushes to an in-process consumer through `Background`
// ---------------------------------------------------------------------------------------------

/// Late-bound consumer (breaks the App <-> Queue construction cycle).
pub type ConsumerSlot = Arc<OnceLock<Arc<dyn EventConsumer>>>;

pub struct MemoryQueue {
    background: Arc<dyn Background>,
    consumer: ConsumerSlot,
    sent: Mutex<Vec<Envelope>>,
}

impl MemoryQueue {
    pub fn new(background: Arc<dyn Background>, consumer: ConsumerSlot) -> Self {
        Self { background, consumer, sent: Mutex::new(Vec::new()) }
    }

    pub fn sent_count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }
}

#[async_trait]
impl Queue for MemoryQueue {
    async fn send(&self, _topic: &str, msg: &Envelope, _opts: SendOpts) -> Result<(), QueueError> {
        self.sent.lock().unwrap().push(msg.clone());
        if let Some(c) = self.consumer.get().cloned() {
            let env = msg.clone();
            self.background.spawn(Box::pin(async move {
                let _ = c.handle(&env).await;
            }));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Session store, blob store, rate limiter
// ---------------------------------------------------------------------------------------------

pub struct MemorySessionStore {
    clock: Arc<dyn Clock>,
    map: Mutex<HashMap<String, (Principal, Timestamp)>>,
}

impl MemorySessionStore {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock, map: Mutex::new(HashMap::new()) }
    }
}

#[async_trait]
impl SessionStore for MemorySessionStore {
    async fn get(&self, token: &str) -> Result<Option<Principal>, CacheError> {
        let now = self.clock.now();
        let m = self.map.lock().unwrap();
        Ok(m.get(token).filter(|(_, exp)| *exp > now).map(|(p, _)| p.clone()))
    }
    async fn put(&self, token: &str, principal: &Principal, ttl_secs: u64) -> Result<(), CacheError> {
        let exp = self.clock.now() + chrono::TimeDelta::seconds(ttl_secs as i64);
        self.map.lock().unwrap().insert(token.to_string(), (principal.clone(), exp));
        Ok(())
    }
}

#[derive(Default)]
pub struct MemoryBlobStore {
    map: Mutex<BTreeMap<String, Blob>>,
}

impl MemoryBlobStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl BlobStore for MemoryBlobStore {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: Option<String>) -> Result<(), BlobError> {
        self.map.lock().unwrap().insert(key.to_string(), Blob { bytes, content_type });
        Ok(())
    }
    async fn get(&self, key: &str) -> Result<Option<Blob>, BlobError> {
        Ok(self.map.lock().unwrap().get(key).cloned())
    }
    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        self.map.lock().unwrap().remove(key);
        Ok(())
    }
    async fn list(&self, prefix: &str) -> Result<Vec<String>, BlobError> {
        Ok(self
            .map
            .lock()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
}

pub struct MemoryRateLimiter {
    clock: Arc<dyn Clock>,
    /// key -> (window_start_secs, count)
    map: Mutex<HashMap<String, (i64, u32)>>,
}

impl MemoryRateLimiter {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock, map: Mutex::new(HashMap::new()) }
    }
}

#[async_trait]
impl RateLimiter for MemoryRateLimiter {
    async fn hit(&self, key: &str, limit: u32, window_secs: u64) -> Result<RateDecision, RateError> {
        let now = self.clock.now().timestamp();
        let w = window_secs.max(1) as i64;
        let start = now - now.rem_euclid(w);
        let mut m = self.map.lock().unwrap();
        let e = m.entry(key.to_string()).or_insert((start, 0));
        if e.0 != start {
            *e = (start, 0);
        }
        if e.1 >= limit {
            return Ok(RateDecision::Limited { retry_after_secs: (start + w - now).max(1) as u64 });
        }
        e.1 += 1;
        Ok(RateDecision::Allowed { remaining: limit - e.1 })
    }
}
