//! Shared fixtures + HTTP client for the contract suite and the bench binary.
//!
//! The suite (`tests/contract.rs`) is target-agnostic: it only ever sees `THRUST_BASE_URL`.
//! Fixtures are created through `POST /internal/seed`, the same route on every target.

use std::time::Duration;

use serde_json::{json, Value};
use thrust_contracts::*;

pub struct Target {
    pub base: String,
    pub admin_token: String,
    pub http: reqwest::Client,
}

impl Target {
    pub fn from_env() -> Target {
        let base = std::env::var("THRUST_BASE_URL").expect("THRUST_BASE_URL must be set (see scripts/contract.sh)");
        let admin_token = std::env::var("THRUST_ADMIN_TOKEN").expect("THRUST_ADMIN_TOKEN must be set");
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .no_proxy()
            .build()
            .unwrap();
        Target { base: base.trim_end_matches('/').to_string(), admin_token, http }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    pub fn create_limit() -> u32 {
        std::env::var("THRUST_EXPECT_CREATE_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(30)
    }

    /// The target's rate-limit window (`THRUST_RATE_WINDOW_SECS` on the server, 60 unless set).
    pub fn rate_window_secs() -> u64 {
        std::env::var("THRUST_EXPECT_RATE_WINDOW_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(60)
    }

    pub async fn admin_post(&self, path: &str, body: &Value) -> reqwest::Response {
        self.http
            .post(self.url(path))
            .header("x-admin-token", &self.admin_token)
            .json(body)
            .send()
            .await
            .unwrap()
    }

    pub async fn admin_get(&self, path: &str) -> reqwest::Response {
        self.http.get(self.url(path)).header("x-admin-token", &self.admin_token).send().await.unwrap()
    }

    /// Revoke the session behind `token` through the test route. 204 whether or not the token is a session.
    pub async fn revoke_session(&self, token: &str) {
        let r = self.admin_post("/internal/revoke-session", &json!({ "token": token })).await;
        assert_eq!(r.status(), 204, "revoke-session");
    }

    pub async fn outbox_depth(&self) -> u64 {
        let r = self.admin_get("/internal/outbox-depth").await;
        assert_eq!(r.status(), 200, "outbox-depth");
        r.json::<Value>().await.unwrap()["undispatched"].as_u64().unwrap()
    }
}

fn uid() -> uuid::Uuid {
    uuid::Uuid::new_v4()
}

#[derive(Clone)]
pub struct User {
    pub id: UserId,
    pub token: String,
}

/// Where the slice's tenant routes are mounted. It must equal `thrust_api::SLICE_PREFIX` (this crate is black-box and
/// does not depend on it). The slice left `/api/v1` in #47; the one other place the suite names a prefix is
/// `RETIRED_SLICE_PREFIX` in `tests/contract.rs`, which shows the old one is gone.
pub const SLICE_PREFIX: &str = "/api/_slice";

/// A project addressed through a workspace slug: the two segments a slice route carries after the prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct At {
    pub slug: String,
    pub project: ProjectId,
}

impl At {
    /// The same project, addressed through another workspace's slug.
    pub fn via(&self, slug: &str) -> At {
        At { slug: slug.to_string(), project: self.project }
    }
}

/// Two tenants. Tenant A has one project with an admin, a member, a guest and an outsider
/// (same workspace, no membership). Tenant B has its own project and admin. `dual` belongs to both workspaces
/// and is a member of both projects: one session, two workspaces.
pub struct Fixture {
    pub ws_a: WorkspaceId,
    pub ws_b: WorkspaceId,
    /// Project A through workspace A's slug, and project B through workspace B's.
    pub a: At,
    pub b: At,
    pub admin_a: User,
    pub member_a: User,
    pub guest_a: User,
    pub outsider_a: User,
    pub admin_b: User,
    pub dual: User,
}

impl Fixture {
    pub async fn seed(t: &Target) -> Fixture {
        let mk = || User { id: UserId(uid()), token: format!("tok-{}", uid().simple()) };
        let ws_a = WorkspaceId(uid());
        let ws_b = WorkspaceId(uid());
        // Slugs are unique in the database, and one database serves every fixture of a run.
        let a = At { slug: format!("ws-a-{}", uid().simple()), project: ProjectId(uid()) };
        let b = At { slug: format!("ws-b-{}", uid().simple()), project: ProjectId(uid()) };
        let (admin_a, member_a, guest_a, outsider_a, admin_b, dual) = (mk(), mk(), mk(), mk(), mk(), mk());

        let in_ws = |workspace_id, u: &User| SeedWorkspaceMember { workspace_id, user_id: u.id };
        let on_a = |u: &User, role| SeedMembership { project_id: a.project, workspace_id: ws_a, user_id: u.id, role };
        let on_b = |u: &User, role| SeedMembership { project_id: b.project, workspace_id: ws_b, user_id: u.id, role };
        let all = [&admin_a, &member_a, &guest_a, &outsider_a, &admin_b, &dual];
        let fixture = SeedFixture {
            workspaces: vec![
                SeedWorkspace { id: ws_a, slug: WorkspaceSlug::new(&a.slug) },
                SeedWorkspace { id: ws_b, slug: WorkspaceSlug::new(&b.slug) },
            ],
            users: all.iter().map(|u| u.id).collect(),
            workspace_members: vec![
                in_ws(ws_a, &admin_a),
                in_ws(ws_a, &member_a),
                in_ws(ws_a, &guest_a),
                in_ws(ws_a, &outsider_a),
                in_ws(ws_b, &admin_b),
                in_ws(ws_a, &dual),
                in_ws(ws_b, &dual),
            ],
            projects: vec![
                SeedProject { id: a.project, workspace_id: ws_a, name: "A".into() },
                SeedProject { id: b.project, workspace_id: ws_b, name: "B".into() },
            ],
            memberships: vec![
                on_a(&admin_a, Role::Admin),
                on_a(&member_a, Role::Member),
                on_a(&guest_a, Role::Guest),
                on_b(&admin_b, Role::Admin),
                on_a(&dual, Role::Member),
                on_b(&dual, Role::Guest),
            ],
            sessions: all.iter().map(|u| SeedSession { token: u.token.clone(), user_id: u.id }).collect(),
        };
        let r = t.admin_post("/internal/seed", &serde_json::to_value(&fixture).unwrap()).await;
        assert_eq!(r.status(), 204, "seed failed: {:?}", r.text().await);
        Fixture { ws_a, ws_b, a, b, admin_a, member_a, guest_a, outsider_a, admin_b, dual }
    }

    /// A fresh member of project A with its own rate-limit bucket.
    pub async fn extra_member(&self, t: &Target) -> User {
        let id = UserId(uid());
        let token = format!("tok-{}", uid().simple());
        let f = SeedFixture {
            workspaces: vec![SeedWorkspace { id: self.ws_a, slug: WorkspaceSlug::new(&self.a.slug) }],
            users: vec![id],
            workspace_members: vec![SeedWorkspaceMember { workspace_id: self.ws_a, user_id: id }],
            projects: vec![],
            memberships: vec![SeedMembership {
                project_id: self.a.project,
                workspace_id: self.ws_a,
                user_id: id,
                role: Role::Member,
            }],
            sessions: vec![SeedSession { token: token.clone(), user_id: id }],
        };
        let r = t.admin_post("/internal/seed", &serde_json::to_value(&f).unwrap()).await;
        assert_eq!(r.status(), 204);
        User { id, token }
    }
}

/// `{SLICE_PREFIX}/workspaces/{slug}/projects/{project}`.
pub fn project_path(at: &At) -> String {
    format!("{SLICE_PREFIX}/workspaces/{}/projects/{}", at.slug, at.project)
}

pub fn wi_path(at: &At) -> String {
    format!("{}/work-items", project_path(at))
}

pub async fn create(t: &Target, who: &User, at: &At, body: Value) -> reqwest::Response {
    t.http
        .post(t.url(&wi_path(at)))
        .bearer_auth(&who.token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

pub async fn create_ok(t: &Target, who: &User, at: &At, name: &str) -> WorkItem {
    let r = create(t, who, at, json!({ "name": name })).await;
    assert_eq!(r.status(), 201, "create '{name}' failed: {:?}", r.text().await);
    r.json::<WorkItem>().await.unwrap()
}

pub async fn list(t: &Target, who: &User, at: &At, scope: Option<&str>) -> reqwest::Response {
    let mut u = t.url(&wi_path(at));
    if let Some(s) = scope {
        u.push_str(&format!("?scope={s}"));
    }
    t.http.get(u).bearer_auth(&who.token).send().await.unwrap()
}

pub async fn activity(t: &Target, who: &User, at: &At, item: WorkItemId) -> Vec<Activity> {
    let r = t
        .http
        .get(t.url(&format!("{}/{}/activity", wi_path(at), item)))
        .bearer_auth(&who.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "activity");
    r.json::<Vec<Activity>>().await.unwrap()
}

/// Poll until `pred` holds (queue consumers are asynchronous by design). Fails loudly on timeout.
pub async fn eventually<F, Fut, T>(what: &str, timeout: Duration, mut f: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let start = std::time::Instant::now();
    loop {
        if let Some(v) = f().await {
            return v;
        }
        if start.elapsed() > timeout {
            panic!("timed out after {:?} waiting for: {what}", timeout);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
