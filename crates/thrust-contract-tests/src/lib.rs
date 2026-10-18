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

    pub async fn outbox_depth(&self) -> u64 {
        let r = self.admin_get("/internal/outbox-depth").await;
        assert_eq!(r.status(), 200, "outbox-depth");
        r.json::<Value>().await.unwrap()["undispatched"].as_u64().unwrap()
    }
}

fn uid() -> uuid::Uuid {
    uuid::Uuid::new_v4()
}

pub struct User {
    pub id: UserId,
    pub token: String,
}

/// Two tenants. Tenant A has one project with an admin, a member, a guest and an outsider
/// (same workspace, no membership). Tenant B has its own project and admin.
pub struct Fixture {
    pub ws_a: WorkspaceId,
    pub ws_b: WorkspaceId,
    pub project_a: ProjectId,
    pub project_b: ProjectId,
    pub admin_a: User,
    pub member_a: User,
    pub guest_a: User,
    pub outsider_a: User,
    pub admin_b: User,
}

impl Fixture {
    pub async fn seed(t: &Target) -> Fixture {
        let mk = |ws: WorkspaceId| {
            let id = UserId(uid());
            (User { id, token: format!("tok-{}", uid().simple()) }, ws)
        };
        let ws_a = WorkspaceId(uid());
        let ws_b = WorkspaceId(uid());
        let project_a = ProjectId(uid());
        let project_b = ProjectId(uid());
        let (admin_a, _) = mk(ws_a);
        let (member_a, _) = mk(ws_a);
        let (guest_a, _) = mk(ws_a);
        let (outsider_a, _) = mk(ws_a);
        let (admin_b, _) = mk(ws_b);

        let fixture = SeedFixture {
            workspaces: vec![ws_a, ws_b],
            users: vec![admin_a.id, member_a.id, guest_a.id, outsider_a.id, admin_b.id],
            projects: vec![
                SeedProject { id: project_a, workspace_id: ws_a, name: "A".into() },
                SeedProject { id: project_b, workspace_id: ws_b, name: "B".into() },
            ],
            memberships: vec![
                SeedMembership { project_id: project_a, workspace_id: ws_a, user_id: admin_a.id, role: Role::Admin },
                SeedMembership { project_id: project_a, workspace_id: ws_a, user_id: member_a.id, role: Role::Member },
                SeedMembership { project_id: project_a, workspace_id: ws_a, user_id: guest_a.id, role: Role::Guest },
                SeedMembership { project_id: project_b, workspace_id: ws_b, user_id: admin_b.id, role: Role::Admin },
            ],
            sessions: vec![
                SeedSession { token: admin_a.token.clone(), principal: Principal { user_id: admin_a.id, workspace_id: ws_a } },
                SeedSession { token: member_a.token.clone(), principal: Principal { user_id: member_a.id, workspace_id: ws_a } },
                SeedSession { token: guest_a.token.clone(), principal: Principal { user_id: guest_a.id, workspace_id: ws_a } },
                SeedSession { token: outsider_a.token.clone(), principal: Principal { user_id: outsider_a.id, workspace_id: ws_a } },
                SeedSession { token: admin_b.token.clone(), principal: Principal { user_id: admin_b.id, workspace_id: ws_b } },
            ],
        };
        let r = t.admin_post("/internal/seed", &serde_json::to_value(&fixture).unwrap()).await;
        assert_eq!(r.status(), 204, "seed failed: {:?}", r.text().await);
        Fixture { ws_a, ws_b, project_a, project_b, admin_a, member_a, guest_a, outsider_a, admin_b }
    }

    /// A fresh member of project A with its own rate-limit bucket.
    pub async fn extra_member(&self, t: &Target) -> User {
        let id = UserId(uid());
        let token = format!("tok-{}", uid().simple());
        let f = SeedFixture {
            workspaces: vec![self.ws_a],
            users: vec![id],
            projects: vec![],
            memberships: vec![SeedMembership {
                project_id: self.project_a,
                workspace_id: self.ws_a,
                user_id: id,
                role: Role::Member,
            }],
            sessions: vec![SeedSession { token: token.clone(), principal: Principal { user_id: id, workspace_id: self.ws_a } }],
        };
        let r = t.admin_post("/internal/seed", &serde_json::to_value(&f).unwrap()).await;
        assert_eq!(r.status(), 204);
        User { id, token }
    }
}

pub fn wi_path(project: ProjectId) -> String {
    format!("/api/v1/projects/{project}/work-items")
}

pub async fn create(t: &Target, who: &User, project: ProjectId, body: Value) -> reqwest::Response {
    t.http
        .post(t.url(&wi_path(project)))
        .bearer_auth(&who.token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

pub async fn create_ok(t: &Target, who: &User, project: ProjectId, name: &str) -> WorkItem {
    let r = create(t, who, project, json!({ "name": name })).await;
    assert_eq!(r.status(), 201, "create '{name}' failed: {:?}", r.text().await);
    r.json::<WorkItem>().await.unwrap()
}

pub async fn list(t: &Target, who: &User, project: ProjectId, scope: Option<&str>) -> reqwest::Response {
    let mut u = t.url(&wi_path(project));
    if let Some(s) = scope {
        u.push_str(&format!("?scope={s}"));
    }
    t.http.get(u).bearer_auth(&who.token).send().await.unwrap()
}

pub async fn activity(t: &Target, who: &User, project: ProjectId, item: WorkItemId) -> Vec<Activity> {
    let r = t
        .http
        .get(t.url(&format!("{}/{}/activity", wi_path(project), item)))
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
