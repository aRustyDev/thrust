//! Scale probe for spike 0002 (issue #37): list, search and create latency with ~100k work items in the database.
//!
//!   THRUST_BASE_URL=... THRUST_ADMIN_TOKEN=... bench_scale [total_items] [items_per_project]
//!
//! It makes its own workspace with `total / per_project` projects through `/internal/seed`, fills each through
//! `/internal/seed-work-items` (5,000 rows a request) and then measures, from this machine, through `/api/v1`:
//!
//! * the seed itself: rows per second, which is what the engine's bulk write path costs;
//! * `GET list` of one project of `per_project` items: the first page of a hundred, which is what a client asks for (`/api/v1`'s
//!   list is paged; the slice's, which this probe measured until the vehicle tests moved (G-26), was not, so 100k items in one
//!   project was one 45 MB response). `THRUST_BENCH_BIG_PROJECT` adds one project of that many items, 10,000 by default, to show
//!   that a page does not grow with the project, at c=1 and c=8, with the response size;
//! * `GET search` of the whole workspace (`/api/v1`'s search spans the caller's projects, the slice's was one project's) for a
//!   common word (hits the 100-result cap), a rare word, and a 2-character query, at c=1;
//! * `POST create` into a project of that size, c=1 and c=8, as `bench` does.
//!
//! The numbers are measured from one client machine: they are not the origin region's, see research/0011, and not comparable to
//! the ones recorded for the slice's routes. Every request counts against an API key's allowance (`Target::v1_limit`, Plane's 60 a
//! minute by default), so the probe makes as many admin keys as the requests need and takes them in turn.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use thrust_contract_tests::*;
use thrust_contracts::*;

/// How many requests the probe makes through `/api/v1` after the seed: the size probes of the two lists (1 each), the timed lists (50 and
/// 20 at two concurrencies), four searches of 40, and two rounds of 200 creates. Each counts against a key's allowance.
const REQUESTS: usize = 2 + 2 * (50 + 20) + 4 * 40 + 2 * 200;

/// The admins of the probe's workspace, each with an API key, taken in turn: the allowance is a key's, so with as many keys as the
/// requests need, none of them is asked for more than it may give in a window, however long the probe takes.
struct Keys {
    admins: Vec<User>,
    turn: AtomicUsize,
}

impl Keys {
    fn next(&self) -> User {
        self.admins[self.turn.fetch_add(1, Ordering::Relaxed) % self.admins.len()].clone()
    }
}

const VERBS: &[&str] =
    &["Fix", "Add", "Remove", "Refactor", "Document", "Investigate", "Migrate", "Review", "Rename", "Test"];
const NOUNS: &[&str] = &[
    "login flow",
    "billing page",
    "search index",
    "queue consumer",
    "retry policy",
    "cycle view",
    "module board",
    "export job",
    "webhook handler",
    "email digest",
    "rate limiter",
    "audit log",
    "session cookie",
    "api key",
    "import wizard",
    "kanban column",
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
    let description = (!i.is_multiple_of(5)).then(|| {
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

    // one workspace, and as many admins of every project as the requests need keys (one, when the allowance is as high as a bench
    // wants it)
    let uid = || uuid::Uuid::new_v4();
    let ws = WorkspaceId(uid());
    let slug = format!("scale-{}", uid().simple());
    let per_key = (Target::v1_limit() as usize).saturating_sub(5).max(1);
    let admins: Vec<User> = (0..REQUESTS.div_ceil(per_key))
        .map(|_| User {
            id: UserId(uid()),
            token: String::new(),
            key: format!("thrk_{}{}", uid().simple(), uid().simple()),
            email: String::new(),
            password: String::new(),
        })
        .collect();
    let admin = admins[0].id;
    let project_ids: Vec<ProjectId> = (0..projects + 1).map(|_| ProjectId(uid())).collect();
    let fixture = SeedFixture {
        workspaces: vec![SeedWorkspace { id: ws, slug: WorkspaceSlug::new(&slug) }],
        users: admins.iter().map(|a| a.id).collect(),
        workspace_members: admins.iter().map(|a| SeedWorkspaceMember { workspace_id: ws, user_id: a.id }).collect(),
        projects: project_ids
            .iter()
            .enumerate()
            .map(|(i, p)| SeedProject { id: *p, workspace_id: ws, name: format!("scale {i}") })
            .collect(),
        memberships: admins
            .iter()
            .flat_map(|a| {
                project_ids.iter().map(|p| SeedMembership {
                    project_id: *p,
                    workspace_id: ws,
                    user_id: a.id,
                    role: Role::Admin,
                })
            })
            .collect(),
        api_keys: admins
            .iter()
            .map(|a| SeedApiKey {
                token: a.key.clone(),
                user_id: a.id,
                label: "scale".into(),
                expires_at: None,
                revoked: false,
            })
            .collect(),
        ..SeedFixture::default()
    };
    let r = t.admin_post("/internal/seed", &serde_json::to_value(&fixture).unwrap()).await;
    assert_eq!(r.status(), 204, "seed fixture: {:?}", r.text().await);
    let keys = Arc::new(Keys { admins, turn: AtomicUsize::new(0) });

    // seed: `projects` projects of `per_project` items, then one big project of `big` items
    let seed_into = |project: ProjectId, n: usize, base: usize| {
        let t = t.clone();
        async move {
            let mut rows = 0usize;
            let started = Instant::now();
            while rows < n {
                let chunk = 5_000.min(n - rows);
                let items: Vec<SeedWorkItemSpec> = (0..chunk)
                    .map(|k| item(base + rows + k))
                    .map(|(name, description)| SeedWorkItemSpec { name, description })
                    .collect();
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
    println!(
        "seeded the big project ({big} items): {:.1}s, {:.0} rows/s",
        big_elapsed.as_secs_f64(),
        big as f64 / big_elapsed.as_secs_f64()
    );

    let at = |p: ProjectId| At { slug: slug.clone(), project: p };
    let small = at(project_ids[0]);
    let bigat = at(big_project);

    // list
    for (target, size, n) in [(small.clone(), per_project, 50usize), (bigat.clone(), big, 20usize)] {
        let bytes = v1_list(&t, &keys.next(), &target).await.bytes().await.unwrap().len();
        for conc in [1usize, 8] {
            let (t2, keys2, target2) = (t.clone(), keys.clone(), target.clone());
            let v = timed(n, conc, move |_| {
                let (t, keys, target) = (t2.clone(), keys2.clone(), target2.clone());
                async move {
                    let r = v1_list(&t, &keys.next(), &target).await;
                    assert_eq!(r.status(), 200);
                    let _ = r.bytes().await.unwrap();
                }
            })
            .await;
            report(&format!("GET list, first page of 100, project of {size} items, c={conc} ({} KB)", bytes / 1024), v);
        }
    }

    // search at scale, over every project of the workspace: a common word (cap), a rarer one, an index miss, and 2 characters
    for (label, q) in [
        ("common word 'Fix' (hits the cap)", "Fix"),
        ("phrase 'billing page #4' (few hits)", "billing page #4"),
        ("no match 'zzzzqq'", "zzzzqq"),
        ("2 characters 'ou' (scan)", "ou"),
    ] {
        let (t2, keys2, slug2) = (t.clone(), keys.clone(), slug.clone());
        let q = q.to_string();
        let v = timed(40, 1, move |_| {
            let (t, keys, slug, q) = (t2.clone(), keys2.clone(), slug2.clone(), q.clone());
            async move {
                let r = v1_search(&t, &keys.next(), &slug, &q, 100).await;
                assert_eq!(r.status(), 200);
                let _ = r.bytes().await.unwrap();
            }
        })
        .await;
        report(&format!("GET search {label}"), v);
    }

    // create into a project of that size
    for conc in [1usize, 8] {
        let (t2, keys2, small2) = (t.clone(), keys.clone(), small.clone());
        let v = timed(200, conc, move |i| {
            let (t, keys, small) = (t2.clone(), keys2.clone(), small2.clone());
            async move {
                let r = v1_create(&t, &keys.next(), &small, json!({ "name": format!("scale-bench-{i}") })).await;
                assert_eq!(r.status(), 201);
                let _ = r.bytes().await.unwrap();
            }
        })
        .await;
        report(&format!("POST create into a project of {per_project}+, c={conc}"), v);
    }
}
