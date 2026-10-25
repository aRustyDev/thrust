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
    /// `user_profiles` (migration 0008): one per user, keyed by the user.
    profiles: HashMap<UserId, UserProfile>,
    /// (provider, provider_account_id) -> the user it is linked to.
    accounts: HashMap<(String, String), UserId>,
    api_keys: Vec<ApiKeyRow>,
    oidc_logins: HashMap<TokenHash, OidcLogin>,
    /// Slug -> workspace (the `workspaces.slug` unique index).
    workspaces: HashMap<WorkspaceSlug, WorkspaceId>,
    /// The `workspaces` table by id (migration 0009); `workspaces` above is its slug index.
    workspace_rows: HashMap<WorkspaceId, Workspace>,
    workspace_members: HashMap<(WorkspaceId, UserId), WorkspaceMember>,
    projects: HashMap<ProjectId, WorkspaceId>,
    /// The `projects` table's other columns (migration 0010); `projects` above is its id-to-workspace index.
    project_rows: HashMap<ProjectId, ProjectRow>,
    /// `project_user_properties` (migrations 0010 and 0012): a member's own settings for a project, and where their list puts it.
    project_user_props: HashMap<(ProjectId, UserId), ProjectUserProperties>,
    /// `states` and `labels` (migration 0012), in the order they were written.
    states: Vec<StateRow>,
    labels: Vec<LabelRow>,
    /// `workspace_user_preferences`, `workspace_home_preferences` and `workspace_user_properties` (migration 0011): rows
    /// a member's first visit to a workspace makes.
    sidebar_preferences: HashMap<(WorkspaceId, UserId, String), (bool, f64)>,
    home_preferences: HashMap<(WorkspaceId, UserId, String), HomePreference>,
    user_properties: HashMap<(WorkspaceId, UserId), UserProperties>,
    /// The member row keeps the workspace it was written with; `membership` counts it only when that
    /// is the project's workspace (the Postgres join on `m.workspace_id`).
    memberships: HashMap<(ProjectId, UserId), ProjectMemberRow>,
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

/// What a member list says about a person: their email and, when they have a profile, its names and avatar (empty when not).
fn member_user(s: &DbState, user: UserId) -> MemberUser {
    let profile = s.profiles.get(&user);
    MemberUser {
        id: user,
        email: s.users.get(&user).and_then(|u| u.email.clone()),
        first_name: profile.map(|p| p.first_name.clone()).unwrap_or_default(),
        last_name: profile.map(|p| p.last_name.clone()).unwrap_or_default(),
        display_name: profile.map(|p| p.display_name.clone()).unwrap_or_default(),
        avatar: profile.map(|p| p.avatar.clone()).unwrap_or_default(),
    }
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
            .filter(|ws| {
                s.workspace_members.get(&(*ws, user)).is_some_and(|m| m.is_active) && s.projects.get(&project) == Some(ws)
            });
        let Some(ws) = resolved else {
            return Ok(Membership::NoSuchProject);
        };
        Ok(match s.memberships.get(&(project, user)) {
            Some(m) if m.workspace_id == ws => Membership::Member { workspace_id: ws, role: m.role },
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

    async fn insert_workspace(&self, w: &NewWorkspace) -> Result<(), DbError> {
        let mut s = self.state.lock().unwrap();
        let ws = &w.workspace;
        // the constraints of the two INSERTs, checked before either row is written (a transaction that fails leaves nothing)
        if s.workspace_rows.contains_key(&ws.id) {
            return Err(DbError::Conflict("workspaces_pkey".into()));
        }
        if s.workspaces.contains_key(&ws.slug) {
            return Err(DbError::Conflict("workspaces_slug_key".into()));
        }
        // a foreign key violation is a backend error, not a `Conflict`, as in Postgres
        if ws.owner_id.is_some_and(|o| !s.users.contains_key(&o)) {
            return Err(DbError::Backend("workspaces_owner_id_fkey".into()));
        }
        if s.workspace_members.values().any(|m| m.id == w.owner.id) {
            return Err(DbError::Conflict("workspace_members_id_key".into()));
        }
        s.workspaces.insert(ws.slug.clone(), ws.id);
        s.workspace_rows.insert(ws.id, ws.clone());
        s.workspace_members.insert((w.owner.workspace_id, w.owner.user_id), w.owner.clone());
        Ok(())
    }

    async fn workspace_slug_taken(&self, slug: &str) -> Result<bool, DbError> {
        Ok(self.state.lock().unwrap().workspaces.contains_key(&WorkspaceSlug::new(slug)))
    }

    async fn any_workspace(&self) -> Result<bool, DbError> {
        Ok(!self.state.lock().unwrap().workspace_rows.is_empty())
    }

    async fn workspaces_of(&self, user: UserId) -> Result<Vec<WorkspaceListing>, DbError> {
        let s = self.state.lock().unwrap();
        let mut out: Vec<WorkspaceListing> = s
            .workspace_members
            .values()
            .filter(|m| m.user_id == user && m.is_active)
            .filter_map(|m| {
                let workspace = s.workspace_rows.get(&m.workspace_id)?.clone();
                let total = s.workspace_members.values().filter(|n| n.workspace_id == m.workspace_id && n.is_active).count();
                Some(WorkspaceListing { workspace, role: m.role, total_members: total as u64 })
            })
            .collect();
        out.sort_by(|a, b| (b.workspace.created_at, b.workspace.id.0).cmp(&(a.workspace.created_at, a.workspace.id.0)));
        Ok(out)
    }

    async fn workspace_access(&self, slug: &WorkspaceSlug, user: UserId) -> Result<Option<WorkspaceAccess>, DbError> {
        let s = self.state.lock().unwrap();
        let Some(ws) = s.workspaces.get(slug).copied() else { return Ok(None) };
        Ok(s.workspace_members
            .get(&(ws, user))
            .filter(|m| m.is_active)
            .map(|m| WorkspaceAccess { workspace_id: ws, role: m.role }))
    }

    async fn is_workspace_member(&self, scope: &WorkspaceScope, user: UserId) -> Result<bool, DbError> {
        Ok(self.state.lock().unwrap().workspace_members.get(&(scope.workspace_id(), user)).is_some_and(|m| m.is_active))
    }

    async fn set_workspace_member_active(
        &self,
        _sys: &SystemScope,
        workspace: WorkspaceId,
        user: UserId,
        active: bool,
    ) -> Result<bool, DbError> {
        let mut s = self.state.lock().unwrap();
        Ok(match s.workspace_members.get_mut(&(workspace, user)) {
            Some(m) => {
                m.is_active = active;
                true
            }
            None => false,
        })
    }

    async fn workspace_member(&self, scope: &WorkspaceScope) -> Result<Option<WorkspaceMember>, DbError> {
        Ok(self.state.lock().unwrap().workspace_members.get(&(scope.workspace_id(), scope.user_id())).cloned())
    }

    async fn workspace_members(&self, scope: &WorkspaceScope) -> Result<Vec<MemberWithUser>, DbError> {
        let s = self.state.lock().unwrap();
        let mut out: Vec<MemberWithUser> = s
            .workspace_members
            .values()
            .filter(|m| m.workspace_id == scope.workspace_id())
            .map(|m| MemberWithUser { member: m.clone(), user: member_user(&s, m.user_id) })
            .collect();
        out.sort_by(|a, b| (b.member.created_at, b.member.id).cmp(&(a.member.created_at, a.member.id)));
        Ok(out)
    }

    async fn project_roles(&self, scope: &WorkspaceScope) -> Result<Vec<(ProjectId, Role)>, DbError> {
        let s = self.state.lock().unwrap();
        let mut out: Vec<(ProjectId, Role)> = s
            .memberships
            .iter()
            // the project must really be in the workspace the member row names (the join on `projects` in Postgres)
            .filter(|((project, user), m)| {
                *user == scope.user_id() && m.workspace_id == scope.workspace_id() && s.projects.get(project) == Some(&m.workspace_id)
            })
            .map(|((project, _), m)| (*project, m.role))
            .collect();
        out.sort_by_key(|(p, _)| p.0);
        Ok(out)
    }

    async fn workspace_projects(&self, scope: &WorkspaceScope) -> Result<Vec<ProjectListing>, DbError> {
        let (ws, user) = (scope.workspace_id(), scope.user_id());
        let s = self.state.lock().unwrap();
        let mut out: Vec<ProjectListing> = s
            .project_rows
            .values()
            .filter(|p| p.workspace_id == ws)
            .map(|p| ProjectListing {
                project: p.clone(),
                // the member row must name the project's own workspace (the join on `m.workspace_id` in Postgres)
                role: s.memberships.get(&(p.id, user)).filter(|m| m.workspace_id == ws).map(|m| m.role),
                sort_order: s.project_user_props.get(&(p.id, user)).map(|up| up.sort_order),
            })
            .filter(|l| match scope.role() {
                Role::Admin => true,
                Role::Member => l.role.is_some() || l.project.network == ProjectNetwork::Public,
                Role::Guest => l.role.is_some(),
            })
            .collect();
        out.sort_by(|a, b| b.project.created_at.cmp(&a.project.created_at).then(b.project.id.cmp(&a.project.id)));
        Ok(out)
    }

    async fn ensure_sidebar_preferences(
        &self,
        scope: &WorkspaceScope,
        defaults: &[SidebarPreference],
        _at: Timestamp,
    ) -> Result<Vec<SidebarPreference>, DbError> {
        let (ws, user) = (scope.workspace_id(), scope.user_id());
        let mut s = self.state.lock().unwrap();
        for d in defaults {
            s.sidebar_preferences.entry((ws, user, d.key.clone())).or_insert((d.is_pinned, d.sort_order));
        }
        let mut out: Vec<SidebarPreference> = s
            .sidebar_preferences
            .iter()
            .filter(|((w, u, _), _)| *w == ws && *u == user)
            .map(|((_, _, key), (is_pinned, sort_order))| SidebarPreference { key: key.clone(), is_pinned: *is_pinned, sort_order: *sort_order })
            .collect();
        out.sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order).then_with(|| a.key.cmp(&b.key)));
        Ok(out)
    }

    async fn ensure_home_preferences(&self, scope: &WorkspaceScope, defaults: &[HomePreference]) -> Result<Vec<HomePreference>, DbError> {
        let (ws, user) = (scope.workspace_id(), scope.user_id());
        let mut s = self.state.lock().unwrap();
        for d in defaults {
            s.home_preferences.entry((ws, user, d.key.clone())).or_insert_with(|| d.clone());
        }
        let mut out: Vec<HomePreference> =
            s.home_preferences.iter().filter(|((w, u, _), _)| *w == ws && *u == user).map(|(_, p)| p.clone()).collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }

    async fn ensure_user_properties(&self, scope: &WorkspaceScope, new: &UserProperties) -> Result<UserProperties, DbError> {
        let mut s = self.state.lock().unwrap();
        let row = s.user_properties.entry((scope.workspace_id(), scope.user_id())).or_insert_with(|| new.clone());
        Ok(row.clone())
    }

    async fn insert_project(&self, scope: &WorkspaceScope, p: &NewProject) -> Result<(), DbError> {
        let ws = scope.workspace_id();
        let mut s = self.state.lock().unwrap();
        let row = &p.project;
        if row.workspace_id != ws {
            return Err(DbError::Backend("project is not in this workspace".into()));
        }
        // the primary key and the two unique indexes (in the order Postgres creates them), then the foreign keys of the users the
        // rows name, then the states' own unique indexes: all of it checked before anything is written, as a transaction is
        if s.projects.contains_key(&row.id) {
            return Err(DbError::Conflict("projects_pkey".into()));
        }
        if s.project_rows.values().any(|o| o.workspace_id == ws && o.identifier == row.identifier) {
            return Err(DbError::Conflict("projects_workspace_identifier_key".into()));
        }
        if s.project_rows.values().any(|o| o.workspace_id == ws && o.name == row.name) {
            return Err(DbError::Conflict("projects_workspace_name_key".into()));
        }
        for (column, user) in [("created_by", row.created_by), ("updated_by", row.updated_by), ("project_lead", row.project_lead), ("default_assignee", row.default_assignee)] {
            if user.is_some_and(|u| !s.users.contains_key(&u)) {
                return Err(DbError::Backend(format!("projects_{column}_fkey")));
            }
        }
        if p.members.iter().any(|m| !s.users.contains_key(&m.member.user_id)) {
            return Err(DbError::Backend("project_user_properties_user_id_fkey".into()));
        }
        // a project member is a member of the workspace the member row names (migration 0003: the row must exist, active or not)
        if p.members.iter().any(|m| !s.workspace_members.contains_key(&(m.member.workspace_id, m.member.user_id))) {
            return Err(DbError::Backend("project_members_workspace_member_fkey".into()));
        }
        let mut names = HashSet::new();
        if !p.states.iter().all(|st| names.insert(st.name.as_str())) {
            return Err(DbError::Conflict("states_project_id_name_key".into()));
        }
        if p.states.iter().filter(|st| st.is_default).count() > 1 {
            return Err(DbError::Conflict("states_one_default_per_project".into()));
        }
        s.projects.insert(row.id, ws);
        s.project_rows.insert(row.id, row.clone());
        for m in &p.members {
            // the place the pin gives a project its member has just joined: 10000 less than the lowest they hold in the workspace
            let lowest = s
                .project_user_props
                .values()
                .filter(|up| up.workspace_id == ws && up.user_id == m.member.user_id)
                .map(|up| up.sort_order)
                .reduce(f64::min);
            let mut props = m.properties.clone();
            props.sort_order = lowest.map_or(65535.0, |l| l - 10000.0);
            s.memberships.insert((row.id, m.member.user_id), m.member.clone());
            s.project_user_props.insert((row.id, m.member.user_id), props);
        }
        s.states.extend(p.states.iter().cloned());
        Ok(())
    }

    async fn project_clashes(
        &self,
        scope: &WorkspaceScope,
        name: Option<&str>,
        identifier: Option<&str>,
        except: Option<ProjectId>,
    ) -> Result<ProjectClashes, DbError> {
        let s = self.state.lock().unwrap();
        let others = || s.project_rows.values().filter(|r| r.workspace_id == scope.workspace_id() && Some(r.id) != except);
        Ok(ProjectClashes {
            name: name.is_some_and(|n| others().any(|r| r.name == n)),
            identifier: identifier.is_some_and(|i| others().any(|r| r.identifier == i)),
        })
    }

    async fn update_project(&self, scope: &WorkspaceScope, row: &ProjectRow) -> Result<bool, DbError> {
        let ws = scope.workspace_id();
        let mut s = self.state.lock().unwrap();
        let Some(current) = s.project_rows.get(&row.id).filter(|r| r.workspace_id == ws).cloned() else { return Ok(false) };
        let others = || s.project_rows.values().filter(|r| r.workspace_id == ws && r.id != row.id);
        if others().any(|r| r.identifier == row.identifier) {
            return Err(DbError::Conflict("projects_workspace_identifier_key".into()));
        }
        if others().any(|r| r.name == row.name) {
            return Err(DbError::Conflict("projects_workspace_name_key".into()));
        }
        for (column, user) in [("updated_by", row.updated_by), ("project_lead", row.project_lead), ("default_assignee", row.default_assignee)] {
            if user.is_some_and(|u| !s.users.contains_key(&u)) {
                return Err(DbError::Backend(format!("projects_{column}_fkey")));
            }
        }
        let mut new = row.clone();
        new.workspace_id = current.workspace_id;
        new.created_at = current.created_at;
        new.created_by = current.created_by;
        s.project_rows.insert(row.id, new);
        Ok(true)
    }

    async fn project_detail(&self, scope: &WorkspaceScope, id: ProjectId) -> Result<Option<ProjectDetail>, DbError> {
        let (ws, user) = (scope.workspace_id(), scope.user_id());
        let s = self.state.lock().unwrap();
        let Some(project) = s.project_rows.get(&id).filter(|r| r.workspace_id == ws).cloned() else { return Ok(None) };
        let mut rows: Vec<&ProjectMemberRow> = s.memberships.values().filter(|m| m.project_id == id && m.workspace_id == ws).collect();
        rows.sort_by(|a, b| (b.created_at, b.id).cmp(&(a.created_at, a.id)));
        Ok(Some(ProjectDetail {
            listing: ProjectListing {
                project,
                role: s.memberships.get(&(id, user)).filter(|m| m.workspace_id == ws).map(|m| m.role),
                sort_order: s.project_user_props.get(&(id, user)).map(|up| up.sort_order),
            },
            members: rows.iter().map(|m| m.user_id).collect(),
            next_sequence: s.counters.get(&(ws, id)).map_or(0, |c| c.0) + 1,
        }))
    }

    async fn project_member(&self, scope: &WorkspaceScope, project: ProjectId) -> Result<Option<ProjectMemberWithUser>, DbError> {
        let (ws, user) = (scope.workspace_id(), scope.user_id());
        let s = self.state.lock().unwrap();
        let (Some(row), Some(workspace)) = (s.project_rows.get(&project).filter(|r| r.workspace_id == ws), s.workspace_rows.get(&ws)) else {
            return Ok(None);
        };
        Ok(s.memberships.get(&(project, user)).filter(|m| m.workspace_id == ws).map(|m| ProjectMemberWithUser {
            member: m.clone(),
            user: member_user(&s, user),
            project: row.clone(),
            workspace: workspace.clone(),
        }))
    }

    async fn project_members(&self, scope: &Scope) -> Result<Vec<ProjectMemberRow>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let s = self.state.lock().unwrap();
        let mut out: Vec<ProjectMemberRow> = s
            .memberships
            .values()
            .filter(|m| m.project_id == project && m.workspace_id == ws && s.projects.get(&project) == Some(&ws))
            .filter(|m| s.workspace_members.get(&(ws, m.user_id)).is_some_and(|w| w.is_active))
            .cloned()
            .collect();
        out.sort_by(|a, b| (b.created_at, b.id).cmp(&(a.created_at, a.id)));
        Ok(out)
    }

    async fn ensure_project_user_properties(&self, scope: &Scope, new: &ProjectUserProperties) -> Result<ProjectUserProperties, DbError> {
        let (ws, project, user) = (scope.workspace_id(), scope.project_id(), scope.user_id());
        let mut s = self.state.lock().unwrap();
        if s.projects.get(&project) != Some(&ws) {
            return Err(DbError::Backend("project is not in this workspace".into()));
        }
        let row = s.project_user_props.entry((project, user)).or_insert_with(|| ProjectUserProperties {
            project_id: project,
            workspace_id: ws,
            user_id: user,
            ..new.clone()
        });
        Ok(row.clone())
    }

    async fn project_labels(&self, scope: &Scope) -> Result<Vec<LabelRow>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let s = self.state.lock().unwrap();
        let mut out: Vec<LabelRow> = s.labels.iter().filter(|l| l.project_id == project && l.workspace_id == ws).cloned().collect();
        out.sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    async fn project_states(&self, scope: &Scope) -> Result<Vec<StateRow>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let s = self.state.lock().unwrap();
        let mut out: Vec<StateRow> =
            s.states.iter().filter(|st| st.project_id == project && st.workspace_id == ws && st.group != StateGroup::Triage).cloned().collect();
        out.sort_by(|a, b| a.sequence.total_cmp(&b.sequence).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    async fn workspace_states(&self, scope: &WorkspaceScope) -> Result<Vec<StateRow>, DbError> {
        let (ws, user) = (scope.workspace_id(), scope.user_id());
        let s = self.state.lock().unwrap();
        let mut out: Vec<StateRow> = s
            .states
            .iter()
            .filter(|st| st.workspace_id == ws && st.group != StateGroup::Triage)
            // a project of this workspace that is not archived, and that the caller is in (their row names the project's workspace)
            .filter(|st| s.project_rows.get(&st.project_id).is_some_and(|p| p.workspace_id == ws && p.archived_at.is_none()))
            .filter(|st| s.memberships.get(&(st.project_id, user)).is_some_and(|m| m.workspace_id == ws))
            .cloned()
            .collect();
        out.sort_by(|a, b| a.sequence.total_cmp(&b.sequence).then(a.project_id.0.cmp(&b.project_id.0)).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    async fn triage_state(&self, scope: &Scope) -> Result<Option<StateRow>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let s = self.state.lock().unwrap();
        let mut found: Vec<&StateRow> =
            s.states.iter().filter(|st| st.project_id == project && st.workspace_id == ws && st.group == StateGroup::Triage).collect();
        found.sort_by(|a, b| a.sequence.total_cmp(&b.sequence).then(a.id.cmp(&b.id)));
        Ok(found.first().map(|st| (*st).clone()))
    }

    async fn project_stats(&self, scope: &WorkspaceScope, only: Option<&[ProjectId]>) -> Result<Vec<ProjectStat>, DbError> {
        let ws = scope.workspace_id();
        let s = self.state.lock().unwrap();
        let mut projects: Vec<&ProjectRow> =
            s.project_rows.values().filter(|p| p.workspace_id == ws && only.map_or(true, |ids| ids.contains(&p.id))).collect();
        projects.sort_by(|a, b| (b.created_at, b.id).cmp(&(a.created_at, a.id)));
        Ok(projects
            .iter()
            .map(|p| ProjectStat {
                id: p.id,
                // the work items of an archived project do not count
                total_issues: if p.archived_at.is_some() {
                    0
                } else {
                    s.work_items.iter().filter(|w| w.workspace_id == ws && w.project_id == p.id && is_visible(w)).count() as i64
                },
                total_members: s.memberships.values().filter(|m| m.project_id == p.id && m.workspace_id == ws).count() as i64,
            })
            .collect())
    }

    async fn user_by_id(&self, id: UserId) -> Result<Option<UserRecord>, DbError> {
        Ok(self.state.lock().unwrap().users.get(&id).cloned())
    }

    async fn insert_profile(&self, profile: &UserProfile) -> Result<bool, DbError> {
        let mut s = self.state.lock().unwrap();
        // the FOREIGN KEY on `user_profiles.user_id`, then the PRIMARY KEY, then the UNIQUE indexes on `id` and `username`
        if !s.users.contains_key(&profile.user_id) {
            // Postgres reports a foreign key violation as a backend error, not a `Conflict`
            return Err(DbError::Backend("user_profiles_user_id_fkey".into()));
        }
        if s.profiles.contains_key(&profile.user_id) {
            return Ok(false);
        }
        if s.profiles.values().any(|p| p.id == profile.id) {
            return Err(DbError::Conflict("user_profiles_id_key".into()));
        }
        if s.profiles.values().any(|p| p.username == profile.username) {
            return Err(DbError::Conflict("user_profiles_username_key".into()));
        }
        s.profiles.insert(profile.user_id, profile.clone());
        Ok(true)
    }

    async fn profile(&self, user: UserId) -> Result<Option<UserProfile>, DbError> {
        Ok(self.state.lock().unwrap().profiles.get(&user).cloned())
    }

    async fn update_profile(&self, user: UserId, patch: &ProfilePatch, at: Timestamp) -> Result<Option<UserProfile>, DbError> {
        let mut s = self.state.lock().unwrap();
        let Some(p) = s.profiles.get_mut(&user) else { return Ok(None) };
        if let Some(v) = &patch.first_name {
            p.first_name = v.clone();
        }
        if let Some(v) = &patch.last_name {
            p.last_name = v.clone();
        }
        if let Some(v) = &patch.display_name {
            p.display_name = v.clone();
        }
        if let Some(v) = &patch.avatar {
            p.avatar = v.clone();
        }
        if let Some(v) = &patch.cover_image {
            p.cover_image = v.clone();
        }
        if let Some(v) = &patch.timezone {
            p.timezone = v.clone();
        }
        if let Some(v) = patch.last_login_at {
            p.last_login_at = Some(v);
        }
        if let Some(v) = patch.is_onboarded {
            p.is_onboarded = v;
        }
        if let Some(v) = patch.is_tour_completed {
            p.is_tour_completed = v;
        }
        if let Some(v) = patch.last_workspace_id {
            p.last_workspace_id = v;
        }
        if let Some(v) = &patch.onboarding_step {
            p.onboarding_step = v.clone();
        }
        if !patch.preferences.is_empty() {
            // jsonb `||`: the patch's keys win, the others stay
            let mut merged = p.preferences.as_object().cloned().unwrap_or_default();
            for (k, v) in &patch.preferences {
                merged.insert(k.clone(), v.clone());
            }
            p.preferences = serde_json::Value::Object(merged);
        }
        p.updated_at = at;
        Ok(Some(p.clone()))
    }

    async fn insert_user(&self, user: &UserRecord) -> Result<(), DbError> {
        let mut s = self.state.lock().unwrap();
        // the PRIMARY KEY on `users.id` and the UNIQUE index on `users.email`, in that order, as Postgres reports them
        if s.users.contains_key(&user.id) {
            return Err(DbError::Conflict("users_pkey".into()));
        }
        if user.email.is_some() && s.users.values().any(|o| o.email == user.email) {
            return Err(DbError::Conflict("users_email_key".into()));
        }
        s.users.insert(user.id, user.clone());
        Ok(())
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

    /// The reference implementation of search: a scan, `to_lowercase` and `contains` (Plane's `icontains`).
    async fn search_work_items(&self, scope: &Scope, query: &str, limit: u32) -> Result<Vec<WorkItem>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        // MUTANT (control run only, feature `mutant_search_case_sensitive`): `contains` without folding case. The engine
        // pass's `icontains` comparison must catch it.
        #[cfg(feature = "mutant_search_case_sensitive")]
        let fold = |t: &str| t.to_string();
        #[cfg(not(feature = "mutant_search_case_sensitive"))]
        let fold = |t: &str| t.to_lowercase();
        let q = fold(query);
        let s = self.state.lock().unwrap();
        let mut v: Vec<WorkItem> = s
            .work_items
            .iter()
            .filter(|w| w.workspace_id == ws && w.project_id == project && is_visible(w))
            .filter(|w| fold(&w.name).contains(&q) || w.description.as_deref().is_some_and(|d| fold(d).contains(&q)))
            .cloned()
            .collect();
        v.sort_by_key(|w| w.sequence_id);
        v.truncate(limit as usize);
        Ok(v)
    }

    async fn seed_work_items(
        &self,
        _sys: &SystemScope,
        workspace: WorkspaceId,
        project: ProjectId,
        items: &[SeedWorkItem],
    ) -> Result<i64, DbError> {
        let mut s = self.state.lock().unwrap();
        if s.projects.get(&project) != Some(&workspace) {
            return Err(DbError::Backend("project is not in this workspace".into()));
        }
        let c = s.counters.entry((workspace, project)).or_insert((0, 0));
        let first = c.0 + 1;
        let mut rows = Vec::with_capacity(items.len());
        for i in items {
            c.0 += 1;
            c.1 += 10_000;
            rows.push(WorkItem {
                id: i.id,
                workspace_id: workspace,
                project_id: project,
                sequence_id: c.0,
                sort_order: c.1,
                name: i.name.clone(),
                description: i.description.clone(),
                external_source: None,
                external_id: None,
                created_by: i.created_by,
                created_at: i.created_at,
                archived_at: None,
                is_draft: false,
                deleted_at: None,
            });
        }
        s.work_items.extend(rows);
        Ok(first)
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
        // a seeded workspace is named after its slug and has no owner; one that exists keeps everything but its slug
        let epoch = thrust_contracts::chrono::DateTime::from_timestamp(0, 0).expect("the epoch");
        for w in &f.workspaces {
            s.workspaces.insert(w.slug.clone(), w.id);
            s.workspace_rows
                .entry(w.id)
                .and_modify(|row| row.slug = w.slug.clone())
                .or_insert_with(|| Workspace {
                    id: w.id,
                    slug: w.slug.clone(),
                    name: w.slug.to_string(),
                    owner_id: None,
                    organization_size: None,
                    timezone: "UTC".into(),
                    background_color: "#808080".into(),
                    created_at: epoch,
                    updated_at: epoch,
                });
        }
        // a seeded membership is an active `member` with no company role; one that exists is left as it is
        let seeded_member = |workspace_id: WorkspaceId, user_id: UserId| WorkspaceMember {
            id: Uuid::from_u128(workspace_id.0.as_u128() ^ user_id.0.as_u128().rotate_left(64)),
            workspace_id,
            user_id,
            role: Role::Member,
            company_role: None,
            is_active: true,
            props: serde_json::json!({}),
            created_at: epoch,
            updated_at: epoch,
        };
        for m in &f.workspace_members {
            s.workspace_members.entry((m.workspace_id, m.user_id)).or_insert_with(|| seeded_member(m.workspace_id, m.user_id));
        }
        // the role of a member the seed names, made when the seed did not have them
        for r in &f.workspace_roles {
            s.workspace_members.entry((r.workspace_id, r.user_id)).or_insert_with(|| seeded_member(r.workspace_id, r.user_id)).role = r.role;
        }
        for p in &f.projects {
            s.projects.insert(p.id, p.workspace_id);
            // a project with no `project_details` entry is what a project made with no options is
            let details = f.project_details.iter().find(|d| d.project_id == p.id).cloned().unwrap_or_else(|| SeedProjectDetails::of(p.id));
            let created_at = details.created_at.unwrap_or(epoch);
            let mut row = ProjectRow::bare(p.id, p.workspace_id, p.name.clone(), created_at);
            row.identifier = details.identifier.unwrap_or_else(|| default_project_identifier(p.id));
            row.network = details.network;
            row.archived_at = details.archived.then_some(created_at);
            row.project_lead = details.project_lead;
            row.created_by = details.created_by;
            s.project_rows.insert(p.id, row);
        }
        for m in &f.memberships {
            // a seeded member row has an id made from the project and the user, the settings the pin starts with unset
            // (`{}`), and the epoch for its time
            s.memberships.insert(
                (m.project_id, m.user_id),
                ProjectMemberRow {
                    id: Uuid::from_u128(m.project_id.0.as_u128() ^ m.user_id.0.as_u128().rotate_left(64)),
                    project_id: m.project_id,
                    workspace_id: m.workspace_id,
                    user_id: m.user_id,
                    role: m.role,
                    comment: None,
                    view_props: serde_json::json!({}),
                    default_props: serde_json::json!({}),
                    preferences: serde_json::json!({}),
                    sort_order: 65535.0,
                    created_by: None,
                    created_at: epoch,
                    updated_at: epoch,
                },
            );
        }
        for o in &f.project_sort_orders {
            // the settings row names the project's workspace; one for a project that is not seeded is the test's bug
            let workspace_id = s.projects.get(&o.project_id).copied().ok_or_else(|| DbError::Backend("project_user_properties_project_id_fkey".into()))?;
            s.project_user_props.insert(
                (o.project_id, o.user_id),
                ProjectUserProperties {
                    id: Uuid::from_u128(o.project_id.0.as_u128().rotate_left(32) ^ o.user_id.0.as_u128()),
                    project_id: o.project_id,
                    workspace_id,
                    user_id: o.user_id,
                    filters: serde_json::json!({}),
                    display_filters: serde_json::json!({}),
                    display_properties: serde_json::json!({}),
                    rich_filters: serde_json::json!({}),
                    preferences: serde_json::json!({}),
                    sort_order: o.sort_order,
                    created_at: epoch,
                    updated_at: epoch,
                },
            );
        }
        for st in &f.states {
            s.states.push(StateRow {
                id: st.id,
                workspace_id: st.workspace_id,
                project_id: st.project_id,
                name: st.name.clone(),
                description: String::new(),
                color: st.color.clone(),
                sequence: st.sequence,
                group: st.group,
                is_default: st.is_default,
                created_by: None,
                created_at: epoch,
                updated_at: epoch,
            });
        }
        for l in &f.labels {
            s.labels.push(LabelRow {
                id: l.id,
                workspace_id: l.workspace_id,
                project_id: l.project_id,
                parent_id: l.parent_id,
                name: l.name.clone(),
                description: String::new(),
                color: l.color.clone(),
                sort_order: l.sort_order,
                created_by: None,
                created_at: epoch,
                updated_at: epoch,
            });
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
