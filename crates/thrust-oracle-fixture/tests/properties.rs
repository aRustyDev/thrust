//! What must hold of the fixture whatever instant it is made at (`prop_`): the ids do not move, the dates follow the day, no row is
//! made in the future or tied with another, every reference resolves, the numbers and places are the ones a project would hand
//! out, and the JSON that `/internal/seed` receives gives the fixture back.

mod common;

use std::collections::{HashMap, HashSet};

use proptest::prelude::*;
use serde_json::Value;
use thrust_contracts::chrono::{DateTime, Duration, Utc};
use thrust_contracts::*;
use thrust_oracle_fixture::*;

/// Any instant from 2001 to 2099, at any fraction of a second (the dates are of its day, so midnight's edges are in range).
fn arb_now() -> impl Strategy<Value = Timestamp> {
    (1_000_000_000_i64..4_070_000_000, 0_u32..1_000_000_000)
        .prop_map(|(secs, nanos)| DateTime::<Utc>::from_timestamp(secs, nanos).expect("an instant in range"))
}

/// The fixture as JSON with every field that depends on the instant set to a placeholder.
fn without_time(f: &SeedFixture) -> Value {
    fn strip(v: &mut Value) {
        match v {
            Value::Object(map) => {
                for (k, v) in map.iter_mut() {
                    if matches!(k.as_str(), "created_at" | "start_date" | "target_date" | "completed_at" | "expires_at")
                    {
                        // whether it is set is behaviour; when is the input
                        *v = if v.is_null() { Value::Null } else { Value::String("<time>".into()) };
                    } else {
                        strip(v);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut v = serde_json::to_value(f).expect("a fixture is JSON");
    strip(&mut v);
    v
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    #[test]
    fn prop_ids_names_and_tokens_do_not_depend_on_the_time(a in arb_now(), b in arb_now()) {
        let (fa, fb) = (oracle_seed(a), oracle_seed(b));
        prop_assert_eq!(without_time(&fa.fixture), without_time(&fb.fixture));
        prop_assert_eq!(fa.manifest, fb.manifest);
    }

    #[test]
    fn prop_dates_follow_the_day_and_no_row_is_made_in_the_future(now in arb_now()) {
        let f = oracle_seed(now).fixture;
        let today = now.date_naive();
        let items: HashMap<&str, &Item> = ALPHA_ITEMS.iter().chain(BETA_ITEMS.iter()).map(|i| (i.key, i)).collect();
        for w in &f.work_items {
            let it = items[w.name_key()];
            prop_assert_eq!(w.start_date, it.start.map(|d| today + Duration::days(d)));
            prop_assert_eq!(w.target_date, it.target.map(|d| today + Duration::days(d)));
            prop_assert!(w.created_at < now, "{} is made at {} and the load is at {}", w.name, w.created_at, now);
        }
        for d in &f.project_details {
            prop_assert!(d.created_at.is_some_and(|t| t < now));
        }
        // each work item has a time of its own, and a project's list in creation order has no ties; the projects too
        let mut seen = HashSet::new();
        for t in f.work_items.iter().map(|w| w.created_at).chain(f.project_details.iter().filter_map(|d| d.created_at)) {
            prop_assert!(seen.insert(t), "two rows made at {t}");
        }
    }

    /// The fixture lists rows in the order the oracle made them, and a list "in creation order" (newest first, and the projects
    /// oldest first) must come out the same: a later row has a later time.
    #[test]
    fn prop_rows_are_made_in_the_order_they_are_listed(now in arb_now()) {
        let f = oracle_seed(now).fixture;
        let items: Vec<Timestamp> = f.work_items.iter().map(|w| w.created_at).collect();
        prop_assert!(items.windows(2).all(|pair| pair[0] < pair[1]), "work items are made one after another: {items:?}");
        let projects: Vec<Timestamp> = f.project_details.iter().filter_map(|d| d.created_at).collect();
        prop_assert_eq!(projects.len(), 2);
        prop_assert!(projects[0] < projects[1], "Alpha is made before Beta");
    }

    #[test]
    fn prop_a_token_has_expired_only_when_it_is_the_expired_one(now in arb_now()) {
        let f = oracle_seed(now).fixture;
        prop_assert_eq!(f.api_keys.len(), TOKENS.len());
        for (key, token, owner, expired, inactive) in TOKENS {
            let k = f.api_keys.iter().find(|k| k.token == token).expect("every token is seeded");
            prop_assert_eq!(k.user_id.0, uid(&format!("user:{owner}")));
            prop_assert_eq!(&k.label, &format!("oracle-{key}"));
            prop_assert_eq!(k.revoked, inactive);
            match (expired, k.expires_at) {
                (true, Some(at)) => prop_assert!(at < now),
                (false, None) => {}
                (want, got) => prop_assert!(false, "{key}: expired should be {want}, expires_at is {got:?}"),
            }
        }
    }

    #[test]
    fn prop_every_reference_of_the_fixture_resolves(now in arb_now()) {
        let f = oracle_seed(now).fixture;
        let users: HashSet<UserId> = f.users.iter().copied().collect();
        let projects: HashMap<ProjectId, WorkspaceId> = f.projects.iter().map(|p| (p.id, p.workspace_id)).collect();
        let states: HashMap<Uuid, ProjectId> = f.states.iter().map(|s| (s.id, s.project_id)).collect();
        let labels: HashMap<Uuid, ProjectId> = f.labels.iter().map(|l| (l.id, l.project_id)).collect();
        let workspaces: HashSet<WorkspaceId> = f.workspaces.iter().map(|w| w.id).collect();
        prop_assert!(projects.values().all(|w| workspaces.contains(w)));
        prop_assert!(f.logins.iter().all(|l| users.contains(&l.user_id)));
        prop_assert!(f.profiles.iter().all(|p| users.contains(&p.user_id)));
        prop_assert!(f.api_keys.iter().all(|k| users.contains(&k.user_id)));
        prop_assert!(f.workspace_roles.iter().all(|r| users.contains(&r.user_id) && workspaces.contains(&r.workspace_id)));
        prop_assert!(f.project_details.iter().all(|d| projects.contains_key(&d.project_id)));
        prop_assert!(f.project_details.iter().all(|d| d.project_lead.is_none_or(|u| users.contains(&u))));
        // a project member is a member of its workspace, and a member of Alpha has the role the oracle gave them
        for m in &f.memberships {
            prop_assert_eq!(projects.get(&m.project_id), Some(&m.workspace_id));
            prop_assert!(f.workspace_roles.iter().any(|r| r.user_id == m.user_id && r.workspace_id == m.workspace_id));
        }
        let mut seen_items: HashMap<Uuid, &SeedIssue> = HashMap::new();
        for w in &f.work_items {
            prop_assert_eq!(projects.get(&w.project_id), Some(&w.workspace_id));
            prop_assert_eq!(w.state_id.and_then(|s| states.get(&s)), Some(&w.project_id), "the state is of the item's project");
            for l in &w.label_ids {
                prop_assert_eq!(labels.get(l), Some(&w.project_id), "a label is of the item's project");
            }
            for a in &w.assignee_ids {
                prop_assert!(
                    f.memberships.iter().any(|m| m.project_id == w.project_id && m.user_id == *a),
                    "an assignee is a member of the item's project"
                );
            }
            // a parent is in the same project and comes first, which the row's foreign key needs
            if let Some(p) = w.parent_id {
                let parent = seen_items.get(&p).expect("a parent comes before its children");
                prop_assert_eq!(parent.project_id, w.project_id);
            }
            prop_assert!(seen_items.insert(w.id.0, w).is_none(), "an id is used once");
        }
        // exactly one default state in a project, and the names of its states and labels are its own
        for p in &f.projects {
            prop_assert_eq!(f.states.iter().filter(|s| s.project_id == p.id && s.is_default).count(), 1);
            let names: HashSet<&str> = f.states.iter().filter(|s| s.project_id == p.id).map(|s| s.name.as_str()).collect();
            prop_assert_eq!(names.len(), STATES_PER_PROJECT);
        }
    }

    #[test]
    fn prop_a_project_hands_out_the_next_number_and_place_after_the_fixtures(now in arb_now()) {
        let f = oracle_seed(now).fixture;
        for p in &f.projects {
            let items: Vec<&SeedIssue> = f.work_items.iter().filter(|w| w.project_id == p.id).collect();
            // numbers are 1..n in the order the items are made
            let numbers: Vec<i64> = items.iter().map(|w| w.sequence_id).collect();
            prop_assert_eq!(numbers, (1..=items.len() as i64).collect::<Vec<_>>());
            // `Issue.save()`: 65535 for the first item of a state, then the largest of that state plus 10000
            let mut by_state: HashMap<Option<Uuid>, Vec<i64>> = HashMap::new();
            for w in &items {
                by_state.entry(w.state_id).or_default().push(w.sort_order);
            }
            for (state, places) in by_state {
                let want: Vec<i64> = (0..places.len() as i64).map(|k| 65_535 + 10_000 * k).collect();
                prop_assert_eq!(places, want, "the places of the items in state {:?}", state);
            }
        }
    }

    #[test]
    fn prop_the_json_that_seed_receives_gives_the_fixture_back(now in arb_now()) {
        let f = oracle_seed(now).fixture;
        let sent = serde_json::to_value(&f).expect("a fixture is JSON");
        let received: SeedFixture = serde_json::from_value(sent.clone()).expect("the route can read what the fixture sends");
        prop_assert_eq!(serde_json::to_value(&received).expect("JSON"), sent);
    }
}

/// The number of states a project has in the seed: the five of the corpus and the triage state that it hides.
const STATES_PER_PROJECT: usize = 6;

/// A work item's name is `seed.py`'s, so the table row it came from is found by it.
trait NameKey {
    fn name_key(&self) -> &str;
}

impl NameKey for SeedIssue {
    fn name_key(&self) -> &str {
        ALPHA_ITEMS
            .iter()
            .chain(BETA_ITEMS.iter())
            .find(|i| i.name == self.name)
            .map(|i| i.key)
            .expect("a seeded item's name is in the table")
    }
}
