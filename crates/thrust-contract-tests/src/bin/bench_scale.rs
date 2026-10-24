//! Scale probe for spike 0002 (issue #37): list, search and create latency with ~100k work items in the database.
//!
//!   THRUST_BASE_URL=... THRUST_ADMIN_TOKEN=... bench_scale [total_items] [items_per_project]
//!
//! It makes its own workspace with `total / per_project` projects through `/internal/seed`, fills each through
//! `/internal/seed-work-items` (5,000 rows a request) and then measures, from this machine:
//!
//! * the seed itself: rows per second, which is what the engine's bulk write path costs;
//! * `GET list` of one project of `per_project` items (the realistic list: the slice's list has no paging, so 100k items
//!   in one project would be one 45 MB response; `THRUST_BENCH_BIG_PROJECT` adds one project of that many items, 10,000 by
//!   default, to show how the unpaged list grows) at c=1 and c=8, with the response size;
//! * `GET search` for a common word (hits the 100-result cap), a rare word, and a 2-character query (a scan below three
//!   characters), at c=1;
//! * `POST create` into a project of that size, c=1 and c=8, as `bench` does.
//!
//! The numbers are measured from one client machine: they are not the origin region's, see research/0011.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use thrust_contract_tests::*;
use thrust_contracts::*;

const VERBS: &[&str] = &["Fix", "Add", "Remove", "Refactor", "Document", "Investigate", "Migrate", "Review", "Rename", "Test"];
const NOUNS: &[&str] = &[
    "login flow", "billing page", "search index", "queue consumer", "retry policy", "cycle view", "module board", "export job",
    "webhook handler", "email digest", "rate limiter", "audit log", "session cookie", "api key", "import wizard", "kanban column",
];

fn pct(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() as f64) * p).ceil() as usize;
    sorted[idx.saturating_sub(1).min(sorted.len() - 1)]
}

fn report(name: &str, mut v: Vec<Duration>) {
    v.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!(
        "{name:<44} n={:<5} p50={:>8.2}ms p95={:>8.2}ms p99={:>8.2}ms max={:>8.2}ms",
        v.len(),
        ms(pct(&v, 0.50)),
        ms(pct(&v, 0.95)),
        ms(pct(&v, 0.99)),
        ms(*v.last().unwrap_or(&Duration::ZERO)),
    );
}

fn item(i: usize) -> (String, Option<String>) {
    let name = format!("{} {} #{i}", VERBS[i % VERBS.len()], NOUNS[(i / VERBS.len()) % NOUNS.len()]);
    let description = (i % 5 != 0).then(|| {
        format!(
            "{} the {} so that the {} no longer {}; see {} for context (ref {:06})",
            VERBS[(i * 7) % VERBS.len()],
            NOUNS[(i * 3) % NOUNS.len()],
            NOUNS[(i * 5) % NOUNS.len()],
            ["fails", "stalls", "duplicates work", "loses events"][i % 4],
            NOUNS[(i * 11) % NOUNS.len()],
            i * 37 % 1_000_000
        )
    });
    (name, description)
}

/// Run `n` requests of `f` with `conc` in flight, timing each.
async fn timed<F, Fut>(n: usize, conc: usize, f: F) -> Vec<Duration>
where
    F: Fn(usize) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let f = Arc::new(f);
    let mut out = Vec::with_capacity(n);
    let mut done = 0;
    while done < n {
        let batch = conc.min(n - done);
        let handles: Vec<_> = (0..batch)
            .map(|k| {
                let f = f.clone();
                let i = done + k;
                tokio::spawn(async move {
                    let s = Instant::now();
                    f(i).await;
                    s.elapsed()
                })
            })
            .collect();
        for h in handles {
            out.push(h.await.unwrap());
        }
        done += batch;
    }
    out
}

#[tokio::main]
async fn main() {
    let total: usize = std::env::args().nth(1).and_then(|v| v.parse().ok()).unwrap_or(100_000);
    let per_project: usize = std::env::args().nth(2).and_then(|v| v.parse().ok()).unwrap_or(1_000);
    let big: usize = std::env::var("THRUST_BENCH_BIG_PROJECT").ok().and_then(|v| v.parse().ok()).unwrap_or(10_000);
    let t = Arc::new(Target::from_env());
    let projects = total / per_project;

    // one workspace, one admin member of every project, and a session for it
    let uid = || uuid::Uuid::new_v4();
    let ws = WorkspaceId(uid());
    let slug = format!("scale-{}", uid().simple());
    let admin = User { id: UserId(uid()), token: format!("tok-{}", uid().simple()), email: String::new(), password: String::new() };
    let project_ids: Vec<ProjectId> = (0..projects + 1).map(|_| ProjectId(uid())).collect();
    let fixture = SeedFixture {
        workspaces: vec![SeedWorkspace { id: ws, slug: WorkspaceSlug::new(&slug) }],
        users: vec![admin.id],
        workspace_members: vec![SeedWorkspaceMember { workspace_id: ws, user_id: admin.id }],
        projects: project_ids.iter().enumerate().map(|(i, p)| SeedProject { id: *p, workspace_id: ws, name: format!("scale {i}") }).collect(),
        memberships: project_ids
            .iter()
            .map(|p| SeedMembership { project_id: *p, workspace_id: ws, user_id: admin.id, role: Role::Admin })
            .collect(),
        sessions: vec![SeedSession { token: admin.token.clone(), user_id: admin.id }],
        ..SeedFixture::default()
    };
    let r = t.admin_post("/internal/seed", &serde_json::to_value(&fixture).unwrap()).await;
    assert_eq!(r.status(), 204, "seed fixture: {:?}", r.text().await);

    // seed: `projects` projects of `per_project` items, then one big project of `big` items
    let seed_into = |project: ProjectId, n: usize, base: usize| {
        let t = t.clone();
        let admin = admin.id;
        async move {
            let mut rows = 0usize;
            let started = Instant::now();
            while rows < n {
                let chunk = 5_000.min(n - rows);
                let items: Vec<SeedWorkItemSpec> =
                    (0..chunk).map(|k| item(base + rows + k)).map(|(name, description)| SeedWorkItemSpec { name, description }).collect();
                let req = SeedWorkItemsRequest { workspace_id: ws, project_id: project, created_by: admin, items };
                let r = t.admin_post("/internal/seed-work-items", &serde_json::to_value(&req).unwrap()).await;
                assert_eq!(r.status(), 200, "seed-work-items: {:?}", r.text().await);
                rows += chunk;
            }
            started.elapsed()
        }
    };
    let started = Instant::now();
    let mut seeded = 0usize;
    for (i, p) in project_ids.iter().take(projects).enumerate() {
        seed_into(*p, per_project, i * per_project).await;
        seeded += per_project;
    }
    let seed_elapsed = started.elapsed();
    println!(
        "seeded {seeded} work items in {projects} projects: {:.1}s, {:.0} rows/s",
        seed_elapsed.as_secs_f64(),
        seeded as f64 / seed_elapsed.as_secs_f64()
    );
    let big_project = project_ids[projects];
    let big_elapsed = seed_into(big_project, big, 0).await;
    println!("seeded the big project ({big} items): {:.1}s, {:.0} rows/s", big_elapsed.as_secs_f64(), big as f64 / big_elapsed.as_secs_f64());

    let at = |p: ProjectId| At { slug: slug.clone(), project: p };
    let small = at(project_ids[0]);
    let bigat = at(big_project);

    // list
    for (target, size, n) in [(small.clone(), per_project, 50usize), (bigat.clone(), big, 20usize)] {
        let bytes = list(&t, &admin, &target, None).await.bytes().await.unwrap().len();
        for conc in [1usize, 8] {
            let (t2, admin2, target2) = (t.clone(), admin.clone(), target.clone());
            let v = timed(n, conc, move |_| {
                let (t, admin, target) = (t2.clone(), admin2.clone(), target2.clone());
                async move {
                    let r = list(&t, &admin, &target, None).await;
                    assert_eq!(r.status(), 200);
                    let _ = r.bytes().await.unwrap();
                }
            })
            .await;
            report(&format!("GET list, project of {size} items, c={conc} ({} KB)", bytes / 1024), v);
        }
    }

    // search at scale: a common word (cap), a rarer one, an index miss, and 2 characters (a scan)
    for (label, q) in [
        ("common word 'Fix' (hits the cap)", "Fix"),
        ("phrase 'billing page #4' (few hits)", "billing page #4"),
        ("no match 'zzzzqq'", "zzzzqq"),
        ("2 characters 'ou' (scan)", "ou"),
    ] {
        let (t2, admin2, small2) = (t.clone(), admin.clone(), small.clone());
        let q = q.to_string();
        let v = timed(40, 1, move |_| {
            let (t, admin, small, q) = (t2.clone(), admin2.clone(), small2.clone(), q.clone());
            async move {
                let r = search(&t, &admin, &small, &q).await;
                assert_eq!(r.status(), 200);
                let _ = r.bytes().await.unwrap();
            }
        })
        .await;
        report(&format!("GET search {label}"), v);
    }

    // create into a project of that size
    for conc in [1usize, 8] {
        let per_user = (Target::create_limit() as usize).saturating_sub(5).max(1);
        let mut users = Vec::new();
        for _ in 0..(200 / per_user + 2) {
            let id = UserId(uid());
            let token = format!("tok-{}", uid().simple());
            let f = SeedFixture {
                workspaces: vec![SeedWorkspace { id: ws, slug: WorkspaceSlug::new(&slug) }],
                users: vec![id],
                workspace_members: vec![SeedWorkspaceMember { workspace_id: ws, user_id: id }],
                projects: vec![],
                memberships: vec![SeedMembership { project_id: project_ids[0], workspace_id: ws, user_id: id, role: Role::Member }],
                sessions: vec![SeedSession { token: token.clone(), user_id: id }],
                ..SeedFixture::default()
            };
            let r = t.admin_post("/internal/seed", &serde_json::to_value(&f).unwrap()).await;
            assert_eq!(r.status(), 204);
            users.push(User { id, token, email: String::new(), password: String::new() });
        }
        let users = Arc::new(users);
        let (t2, small2) = (t.clone(), small.clone());
        let v = timed(200, conc, move |i| {
            let (t, users, small) = (t2.clone(), users.clone(), small2.clone());
            async move {
                let r = create(&t, &users[i / per_user], &small, json!({ "name": format!("scale-bench-{i}") })).await;
                assert_eq!(r.status(), 201);
                let _ = r.bytes().await.unwrap();
            }
        })
        .await;
        report(&format!("POST create into a project of {per_project}+, c={conc}"), v);
    }
}
