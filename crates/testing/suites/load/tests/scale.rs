//! The scale probe of spike 0002 (issue #37), as a load test: list, search and create latency with ~100k work items in the
//! database. It needs a running target (`THRUST_BASE_URL`, `THRUST_ADMIN_TOKEN`): `scripts/contract.sh <target> scale` starts one
//! and runs it with the `load` profile, release-built. `THRUST_SCALE_TOTAL` (100,000) and `THRUST_SCALE_PER_PROJECT` (1,000) set
//! the size.
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
//! * `POST create` into a project of that size, c=1 and c=8, as the latency test does.
//!
//! The numbers are measured from one client machine: they are not the origin region's, see research/0011, and not comparable to
//! the ones recorded for the slice's routes. A test passes when every request it made succeeded; a number is not a pass or a fail.
//! Every request counts against an API key's allowance (`Target::v1_limit`, Plane's 60 a minute by default), so the probe makes as
//! many admin keys as the requests need and takes them in turn.

use std::sync::Arc;
use std::time::Instant;

use serde_json::json;
use thrust_contract_tests::*;
use thrust_contracts::*;
use thrust_load_tests::{env_usize, keys_needed, report, scale_item, timed, Rotation};

/// How many requests the probe makes through `/api/v1` after the seed: the size probes of the two lists (1 each), the timed lists (50 and
/// 20 at two concurrencies), four searches of 40, and two rounds of 200 creates. Each counts against a key's allowance.
const REQUESTS: usize = 2 + 2 * (50 + 20) + 4 * 40 + 2 * 200;

#[tokio::test(flavor = "multi_thread")]
async fn load_scale_list_search_and_create() {
    let total = env_usize("THRUST_SCALE_TOTAL", 100_000);
    let per_project = env_usize("THRUST_SCALE_PER_PROJECT", 1_000);
    let big = env_usize("THRUST_BENCH_BIG_PROJECT", 10_000);
    let t = Arc::new(Target::from_env());
    let projects = total / per_project;

    // one workspace, and as many admins of every project as the requests need keys (one, when the allowance is as high as a bench
    // wants it)
    let uid = || uuid::Uuid::new_v4();
    let ws = WorkspaceId(uid());
    let slug = format!("scale-{}", uid().simple());
    let admins: Vec<User> = (0..keys_needed(REQUESTS, Target::v1_limit()))
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
    let keys = Arc::new(Rotation::new(admins).expect("at least one admin"));

    // seed: `projects` projects of `per_project` items, then one big project of `big` items
    let seed_into = |project: ProjectId, n: usize, base: usize| {
        let t = t.clone();
        async move {
            let mut rows = 0usize;
            let started = Instant::now();
            while rows < n {
                let chunk = 5_000.min(n - rows);
                let items: Vec<SeedWorkItemSpec> = (0..chunk)
                    .map(|k| scale_item(base + rows + k))
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
            report(
                &mut std::io::stdout(),
                &format!("GET list, first page of 100, project of {size} items, c={conc} ({} KB)", bytes / 1024),
                v,
            )
            .unwrap();
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
        report(&mut std::io::stdout(), &format!("GET search {label}"), v).unwrap();
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
        report(&mut std::io::stdout(), &format!("POST create into a project of {per_project}+, c={conc}"), v).unwrap();
    }
}
