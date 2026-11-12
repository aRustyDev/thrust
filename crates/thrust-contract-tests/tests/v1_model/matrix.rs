//! Every actor × slug × project × route, once, against the access table in the parent module (`standing`, `gate`) and the slug rule
//! below.

use super::*;

#[derive(Debug, Clone, Copy)]
enum Route {
    Create,
    List,
    Detail,
    /// A delete of a work item the caller made (where the caller may make one).
    DeleteMine,
    /// A delete of a work item the project's admin made: only the admin may, and a member is refused in the handler's words.
    DeleteAdmins,
}

const ALL_ROUTES: [Route; 5] = [Route::Create, Route::List, Route::Detail, Route::DeleteMine, Route::DeleteAdmins];

fn min_role(r: Route) -> Role {
    match r {
        Route::Create | Route::DeleteMine | Route::DeleteAdmins => Role::Member,
        Route::List | Route::Detail => Role::Guest,
    }
}

fn success_status(r: Route) -> u16 {
    match r {
        Route::Create => 201,
        Route::List | Route::Detail => 200,
        Route::DeleteMine | Route::DeleteAdmins => 204,
    }
}

/// The status a request that names `slug` gets when it needs `min`, or `None` when it passes: the parent module's `gate` (a missing key,
/// a key that is not valid, a slug that is not the project's own workspace's, then the project and the role).
fn rejection(actor: Actor, slug: Slug, proj: Proj, min: Role) -> Option<u16> {
    gate(actor, proj, slug, min).map(|(refusal, _)| refusal.status)
}

/// What the table says a request gets: a rejection, or the route's success. A member's delete of what the admin made is the one request
/// that passes the permission and is refused after it (in the handler).
fn expected(actor: Actor, slug: Slug, proj: Proj, route: Route) -> u16 {
    let project_admin = matches!(standing(actor, proj), Standing::Member(Role::Admin));
    match rejection(actor, slug, proj, min_role(route)) {
        Some(status) => status,
        None if matches!(route, Route::DeleteAdmins) && !project_admin => 403,
        None => success_status(route),
    }
}

/// Every combination is checked (7 × 3 × 3 × 5 = 315) and every disagreement is reported, not only the first. A delete that is to
/// succeed targets a work item that the caller made just before, so an allowed delete is observable; one that is to be refused aims at
/// a work item the project's admin made, which must still be there. Afterwards both projects must hold exactly the work items the
/// table predicts (no rejected request had an effect). The refusals before the handler are not counted against a key's 60 requests a
/// minute, so no caller comes near the limit.
#[tokio::test]
async fn every_actor_project_and_route_gets_the_status_of_the_v1_access_table() {
    let t = Target::from_env();
    let w = World::seed(&t).await;
    let admin = |s: usize| if s == 0 { &w.f.admin_a } else { &w.f.admin_b };
    // a work item per project that every detail asks for (and that is never deleted), and one that every refused delete aims at (and
    // that must still be there at the end)
    let host = [
        v1_create_ok(&t, admin(0), w.at_slot(0), "host").await.id(),
        v1_create_ok(&t, admin(1), w.at_slot(1), "host").await.id(),
    ];
    let shared_victim = [
        v1_create_ok(&t, admin(0), w.at_slot(0), "shared victim").await.id(),
        v1_create_ok(&t, admin(1), w.at_slot(1), "shared victim").await.id(),
    ];
    // per project: the names of the live work items the table says must exist at the end
    let mut want_live: [Vec<String>; 2] = [Vec::new(), Vec::new()];
    for live in &mut want_live {
        live.push("host".to_string());
        live.push("shared victim".to_string());
    }
    let mut mismatches = Vec::new();
    let mut checked = 0;
    for actor in ALL_ACTORS {
        for slug in ALL_SLUGS {
            for proj in ALL_PROJECTS {
                for route in ALL_ROUTES {
                    let label = format!("{actor:?} {route:?} on {proj:?} via {slug:?}");
                    let want = expected(actor, slug, proj, route);
                    let allowed = want == success_status(route);
                    let req = match route {
                        Route::Create => {
                            Req { body: Some(json!({ "name": label })), slug, ..Req::bare(actor, Method::POST, proj) }
                        }
                        Route::List => Req { slug, ..Req::bare(actor, Method::GET, proj) },
                        Route::Detail => {
                            let item = slot(proj).map_or_else(|| unknown_item(checked), |s| host[s].clone());
                            Req { item: Some(item), slug, ..Req::bare(actor, Method::GET, proj) }
                        }
                        Route::DeleteMine | Route::DeleteAdmins => {
                            // a delete that is to succeed is of a work item the caller made (an admin may delete any, so for the
                            // admin's own route it is the admin's; a member may delete only its own); every other delete aims at the
                            // shared victim, which the admin made, or at a work item that is not there
                            let item = match slot(proj) {
                                Some(s) if allowed => {
                                    let by = w.user(actor).expect("a request that succeeds has a caller");
                                    v1_create_ok(&t, by, w.at_slot(s), &format!("victim of {label}")).await.id()
                                }
                                Some(s) => shared_victim[s].clone(),
                                None => unknown_item(checked),
                            };
                            Req { item: Some(item), slug, ..Req::bare(actor, Method::DELETE, proj) }
                        }
                    };
                    let status = w.send(&req).await.status;
                    if status != want {
                        mismatches.push(format!("{label}: got {status}, the table says {want}"));
                    }
                    if matches!(route, Route::Create) && allowed {
                        want_live[slot(proj).expect("a create that passes is in a seeded project")].push(label.clone());
                    }
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(
        checked,
        ALL_ACTORS.len() * ALL_SLUGS.len() * ALL_PROJECTS.len() * ALL_ROUTES.len(),
        "the matrix did not run"
    );
    for (s, want) in want_live.iter_mut().enumerate() {
        let items = v1_list_all(&t, admin(s), w.at_slot(s)).await;
        let mut got: Vec<String> = items.iter().map(|i| i.name().to_string()).collect();
        got.sort();
        want.sort();
        if got != *want {
            mismatches.push(format!("project {s}: work items after the matrix\n  got  {got:?}\n  want {want:?}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} disagreement(s) with the access table over {checked} combinations:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}
