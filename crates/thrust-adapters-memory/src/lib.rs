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
    /// The `sessions` table: keyed by the token hash, no workspace (a directory table).
    sessions: HashMap<TokenHash, SessionRow>,
    /// The identity tables (migration 0005), all directory tables like `sessions`.
    users: HashMap<UserId, UserRecord>,
    /// (provider, provider_account_id) -> the user it is linked to.
    accounts: HashMap<(String, String), UserId>,
    api_keys: Vec<ApiKeyRow>,
    oidc_logins: HashMap<TokenHash, OidcLogin>,
    /// Slug -> workspace (the `workspaces.slug` unique index).
    workspaces: HashMap<WorkspaceSlug, WorkspaceId>,
    workspace_members: HashSet<(WorkspaceId, UserId)>,
    projects: HashMap<ProjectId, WorkspaceId>,
    /// The member row keeps the workspace it was written with; `membership` counts it only when that
    /// is the project's workspace (the Postgres join on `m.workspace_id`).
    memberships: HashMap<(ProjectId, UserId), (WorkspaceId, Role)>,
    /// (workspace, project) -> (last sequence_id handed out, last sort_order handed out). Never
    /// decrements.
    counters: HashMap<(WorkspaceId, ProjectId), (i64, i64)>,
    work_items: Vec<WorkItem>,
    outbox: Vec<OutboxRow>,
    activity: Vec<Activity>,
    activity_events: HashSet<EventId>,
}

struct SessionRow {
    user_id: UserId,
    expires_at: Timestamp,
    revoked_at: Option<Timestamp>,
    last_seen_at: Timestamp,
}

struct ApiKeyRow {
    key: NewApiKey,
    revoked_at: Option<Timestamp>,
    last_used_at: Option<Timestamp>,
}

impl DbState {
    fn user_is_active(&self, user: UserId) -> bool {
        self.users.get(&user).is_some_and(|u| u.is_active)
    }
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
        slug: &WorkspaceSlug,
        project: ProjectId,
        user: UserId,
    ) -> Result<Membership, DbError> {
        let s = self.state.lock().unwrap();
        // The workspace the caller may act in: the slug names it, the caller belongs to it and the
        // project is in it (the three inner joins of the Postgres query).
        // MUTANT (control run only): forget the tenant predicates and trust the project's own
        // workspace. The cross-tenant tests must catch it.
        #[cfg(feature = "mutant_ignore_tenant")]
        let resolved = s.projects.get(&project).copied();
        #[cfg(not(feature = "mutant_ignore_tenant"))]
        let resolved = s
            .workspaces
            .get(slug)
            .copied()
            .filter(|ws| s.workspace_members.contains(&(*ws, user)) && s.projects.get(&project) == Some(ws));
        let Some(ws) = resolved else {
            return Ok(Membership::NoSuchProject);
        };
        Ok(match s.memberships.get(&(project, user)) {
            Some((m_ws, role)) if *m_ws == ws => Membership::Member { workspace_id: ws, role: *role },
            _ => Membership::NotAMember,
        })
    }

    async fn session(&self, hash: &TokenHash, now: Timestamp) -> Result<Option<LiveSession>, DbError> {
        let s = self.state.lock().unwrap();
        // MUTANT (control run only): a revoked session still resolves. The revocation test must catch it.
        #[cfg(feature = "mutant_ignore_revoked")]
        let revoked = |_: &SessionRow| false;
        #[cfg(not(feature = "mutant_ignore_revoked"))]
        let revoked = |r: &SessionRow| r.revoked_at.is_some();
        Ok(s.sessions
            .get(hash)
            .filter(|r| !revoked(r) && r.expires_at > now && s.user_is_active(r.user_id))
            .map(|r| LiveSession { user_id: r.user_id, last_seen_at: r.last_seen_at }))
    }

    async fn touch_session(&self, hash: &TokenHash, at: Timestamp) -> Result<(), DbError> {
        if let Some(r) = self.state.lock().unwrap().sessions.get_mut(hash) {
            r.last_seen_at = at;
        }
        Ok(())
    }

    async fn revoke_session(&self, hash: &TokenHash, at: Timestamp) -> Result<(), DbError> {
        if let Some(r) = self.state.lock().unwrap().sessions.get_mut(hash) {
            r.revoked_at.get_or_insert(at);
        }
        Ok(())
    }

    async fn insert_session(&self, new: &NewSession) -> Result<(), DbError> {
        self.state.lock().unwrap().sessions.entry(new.token_hash).or_insert(SessionRow {
            user_id: new.user_id,
            expires_at: new.expires_at,
            revoked_at: None,
            last_seen_at: new.created_at,
        });
        Ok(())
    }

    async fn revoke_user_sessions(&self, user: UserId, at: Timestamp) -> Result<u64, DbError> {
        let mut s = self.state.lock().unwrap();
        let mut n = 0;
        for r in s.sessions.values_mut().filter(|r| r.user_id == user && r.revoked_at.is_none()) {
            r.revoked_at = Some(at);
            n += 1;
        }
        Ok(n)
    }

    async fn user_by_email(&self, email: &str) -> Result<Option<UserRecord>, DbError> {
        let s = self.state.lock().unwrap();
        Ok(s.users.values().find(|u| u.email.as_deref() == Some(email)).cloned())
    }

    async fn user_for_account(&self, provider: &str, provider_account_id: &str) -> Result<Option<UserRecord>, DbError> {
        let s = self.state.lock().unwrap();
        let key = (provider.to_string(), provider_account_id.to_string());
        Ok(s.accounts.get(&key).and_then(|u| s.users.get(u)).cloned())
    }

    async fn link_account(&self, a: &NewAccount) -> Result<(), DbError> {
        let mut s = self.state.lock().unwrap();
        if !s.users.contains_key(&a.user_id) {
            // the foreign key of Postgres
            return Err(DbError::Backend("account of a user that does not exist".into()));
        }
        s.accounts
            .entry((a.provider.clone(), a.provider_account_id.clone()))
            .or_insert(a.user_id);
        Ok(())
    }

    async fn insert_api_key(&self, k: &NewApiKey) -> Result<(), DbError> {
        let mut s = self.state.lock().unwrap();
        if !s.users.contains_key(&k.user_id) {
            return Err(DbError::Backend("api key of a user that does not exist".into()));
        }
        if k.hash_version <= 0 {
            // the CHECK on `api_keys.hash_version`
            return Err(DbError::Backend("api key hash_version must be positive".into()));
        }
        if s.api_keys.iter().any(|r| r.key.token_hash == k.token_hash || r.key.id == k.id) {
            return Err(DbError::Conflict("api key already exists".into()));
        }
        s.api_keys.push(ApiKeyRow { key: k.clone(), revoked_at: None, last_used_at: None });
        Ok(())
    }

    async fn api_key(&self, hash: &TokenHash, now: Timestamp) -> Result<Option<LiveApiKey>, DbError> {
        let s = self.state.lock().unwrap();
        // MUTANT (control run only): a revoked key still resolves. The revocation test must catch it.
        #[cfg(feature = "mutant_ignore_revoked")]
        let revoked = |_: &ApiKeyRow| false;
        #[cfg(not(feature = "mutant_ignore_revoked"))]
        let revoked = |r: &ApiKeyRow| r.revoked_at.is_some();
        Ok(s.api_keys
            .iter()
            .find(|r| r.key.token_hash == *hash)
            .filter(|r| !revoked(r) && r.key.expires_at.is_none_or(|e| e > now) && s.user_is_active(r.key.user_id))
            .map(|r| LiveApiKey {
                id: r.key.id,
                user_id: r.key.user_id,
                last_used_at: r.last_used_at,
                hash_version: r.key.hash_version,
            }))
    }

    async fn touch_api_key(&self, id: ApiKeyId, at: Timestamp) -> Result<(), DbError> {
        let mut s = self.state.lock().unwrap();
        if let Some(r) = s.api_keys.iter_mut().find(|r| r.key.id == id) {
            r.last_used_at = Some(at);
        }
        Ok(())
    }

    async fn rehash_api_key(
        &self,
        id: ApiKeyId,
        from_version: i16,
        new_hash: &TokenHash,
        new_version: i16,
    ) -> Result<bool, DbError> {
        let mut s = self.state.lock().unwrap();
        // `UPDATE ... WHERE id = $1 AND hash_version = $2`: a row that does not match is not touched, so neither
        // constraint below is looked at for it
        let Some(at) = s.api_keys.iter().position(|r| r.key.id == id && r.key.hash_version == from_version) else {
            return Ok(false);
        };
        if new_version <= 0 {
            // the CHECK on `api_keys.hash_version`
            return Err(DbError::Backend("api key hash_version must be positive".into()));
        }
        // the UNIQUE index on `token_hash`
        if s.api_keys.iter().any(|r| r.key.id != id && r.key.token_hash == *new_hash) {
            return Err(DbError::Conflict("api key already exists".into()));
        }
        let row = &mut s.api_keys[at];
        row.key.token_hash = *new_hash;
        row.key.hash_version = new_version;
        Ok(true)
    }

    async fn list_api_keys(&self, user: UserId) -> Result<Vec<ApiKeyInfo>, DbError> {
        let s = self.state.lock().unwrap();
        let mut v: Vec<ApiKeyInfo> = s
            .api_keys
            .iter()
            .filter(|r| r.key.user_id == user)
            .map(|r| ApiKeyInfo {
                id: r.key.id,
                prefix: r.key.prefix.clone(),
                label: r.key.label.clone(),
                created_at: r.key.created_at,
                expires_at: r.key.expires_at,
                revoked_at: r.revoked_at,
                last_used_at: r.last_used_at,
            })
            .collect();
        v.sort_by_key(|k| (k.created_at, k.id));
        Ok(v)
    }

    async fn revoke_api_key(&self, user: UserId, id: ApiKeyId, at: Timestamp) -> Result<bool, DbError> {
        let mut s = self.state.lock().unwrap();
        match s.api_keys.iter_mut().find(|r| r.key.id == id && r.key.user_id == user) {
            Some(r) => {
                r.revoked_at.get_or_insert(at);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn insert_oidc_login(&self, l: &NewOidcLogin) -> Result<(), DbError> {
        let mut s = self.state.lock().unwrap();
        s.oidc_logins.retain(|_, v| v.expires_at > l.created_at);
        if s.oidc_logins.contains_key(&l.state_hash) {
            return Err(DbError::Conflict("oidc login already exists".into()));
        }
        s.oidc_logins.insert(
            l.state_hash,
            OidcLogin { nonce: l.nonce.clone(), code_verifier: l.code_verifier.clone(), expires_at: l.expires_at },
        );
        Ok(())
    }

    async fn take_oidc_login(&self, state: &TokenHash) -> Result<Option<OidcLogin>, DbError> {
        Ok(self.state.lock().unwrap().oidc_logins.remove(state))
    }

    async fn set_user_active(&self, _sys: &SystemScope, user: UserId, active: bool) -> Result<bool, DbError> {
        match self.state.lock().unwrap().users.get_mut(&user) {
            Some(u) => {
                u.is_active = active;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn seed_users(&self, _sys: &SystemScope, users: &[UserRecord]) -> Result<(), DbError> {
        let mut s = self.state.lock().unwrap();
        for u in users {
            // the UNIQUE index on `users.email`
            if u.email.is_some() && s.users.values().any(|o| o.id != u.id && o.email == u.email) {
                return Err(DbError::Conflict("users_email_key".into()));
            }
            s.users.insert(u.id, u.clone());
        }
        Ok(())
    }

    async fn begin(&self, _scope: &Scope) -> Result<Box<dyn Tx>, DbError> {
        Ok(Box::new(MemoryTx {
            state: self.state.clone(),
            pending_items: Vec::new(),
            pending_outbox: Vec::new(),
            pending_deletes: Vec::new(),
        }))
    }

    async fn list_work_items(&self, scope: &Scope, vis: Visibility) -> Result<Vec<WorkItem>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
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
        scope: &Scope,
        id: WorkItemId,
        vis: Visibility,
    ) -> Result<Option<WorkItem>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
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

    async fn insert_activity(&self, _sys: &SystemScope, a: &Activity) -> Result<bool, DbError> {
        let mut s = self.state.lock().unwrap();
        // MUTANT (control run only): no idempotency on event_id. The redelivery test must catch it.
        #[cfg(feature = "mutant_no_activity_dedupe")]
        let duplicate = false;
        #[cfg(not(feature = "mutant_no_activity_dedupe"))]
        let duplicate = !s.activity_events.insert(a.event_id);
        if duplicate {
            return Ok(false);
        }
        s.activity.push(a.clone());
        Ok(true)
    }

    async fn list_activity(&self, scope: &Scope, entity: Option<WorkItemId>) -> Result<Vec<Activity>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
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

    async fn undispatched_outbox(&self, _sys: &SystemScope, limit: u32) -> Result<Vec<OutboxRow>, DbError> {
        let s = self.state.lock().unwrap();
        Ok(s.outbox
            .iter()
            .filter(|r| r.dispatched_at.is_none())
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn mark_outbox_dispatched(&self, _sys: &SystemScope, event: EventId, at: Timestamp) -> Result<(), DbError> {
        let mut s = self.state.lock().unwrap();
        for r in s.outbox.iter_mut().filter(|r| r.envelope.event_id == event) {
            r.dispatched_at = Some(at);
            r.attempts += 1;
        }
        Ok(())
    }

    async fn outbox_depth(&self, _sys: &SystemScope) -> Result<u64, DbError> {
        let s = self.state.lock().unwrap();
        Ok(s.outbox.iter().filter(|r| r.dispatched_at.is_none()).count() as u64)
    }

    async fn seed(&self, _sys: &SystemScope, f: &SeedFixture) -> Result<(), DbError> {
        let mut s = self.state.lock().unwrap();
        for u in &f.users {
            // a user that exists already (with an email, say) is left as it is
            s.users.entry(*u).or_insert(UserRecord { id: *u, email: None, password_hash: None, is_active: true });
        }
        for w in &f.workspaces {
            s.workspaces.insert(w.slug.clone(), w.id);
        }
        for m in &f.workspace_members {
            s.workspace_members.insert((m.workspace_id, m.user_id));
        }
        for p in &f.projects {
            s.projects.insert(p.id, p.workspace_id);
        }
        for m in &f.memberships {
            s.memberships.insert((m.project_id, m.user_id), (m.workspace_id, m.role));
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
    /// A scope whose project is outside its workspace is `Backend`, as the Postgres foreign key
    /// makes it. `authorize` never mints one; a test can.
    async fn allocate(&mut self, scope: &Scope) -> Result<Allocation, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let mut s = self.state.lock().unwrap();
        if s.projects.get(&project) != Some(&ws) {
            return Err(DbError::Backend("project is not in this workspace".into()));
        }
        let c = s.counters.entry((ws, project)).or_insert((0, 0));
        // MUTANT (control run only): hand out the same sequence number every time.
        #[cfg(not(feature = "mutant_dup_sequence"))]
        {
            c.0 += 1;
        }
        #[cfg(feature = "mutant_dup_sequence")]
        {
            c.0 = 1;
        }
        c.1 += 10_000;
        Ok(Allocation { sequence_id: c.0, sort_order: c.1 })
    }

    async fn insert_work_item(&mut self, scope: &Scope, item: &WorkItem) -> Result<(), DbError> {
        check_tenant(scope, item.workspace_id, Some(item.project_id))?;
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

    async fn soft_delete_work_item(&mut self, scope: &Scope, id: WorkItemId, at: Timestamp) -> Result<bool, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
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

    async fn insert_outbox(&mut self, scope: &Scope, row: &OutboxRow) -> Result<(), DbError> {
        check_tenant(scope, row.envelope.workspace_id, row.envelope.project_id())?;
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
// Blob store, rate limiter
// ---------------------------------------------------------------------------------------------

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
