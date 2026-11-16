//! The v1 oracle's seed as a Thrust fixture (issue #58).
//!
//! `tools/oracle-v1/scripts/seed.py` fills a Plane with a workspace, four people, two projects, their states, labels and work
//! items, and seven API tokens, under ids that are uuid5 over a fixed namespace, so that a rebuilt stack seeds byte-identical
//! ids. The oracle's golden corpus was recorded against that seed and its `seed-manifest.json` names what it made. This crate
//! makes the same rows for Thrust, as the [`SeedFixture`] that `POST /internal/seed` takes, so that the corpus and the manifest
//! apply to Thrust unchanged (the gate needs to point its requests, and compare its answers, at the same ids and tokens).
//!
//! **What is reproduced and what is not.** The 32 operations of #58 read projects, states, labels and work items, and what hangs
//! from them: the people a work item names and who the caller is. All of that is here. The oracle's seed also makes a cycle, a
//! module, two comments, a link and two activities on item three, and the project-member row ids (`pmember:*`); Thrust has no
//! cycles or modules (D-030), and the other three belong to operations that are not in the 32 (comments, links, activities,
//! members). They are not seeded, and the keys for them in the manifest are not checked.
//!
//! **Where each value comes from.** Every id, name, e-mail, token, colour, priority, offset and list order is copied from
//! `seed.py`. The three that `seed.py` does not state are derived from the Plane the corpus was recorded against (the tag
//! `oracle/v1-pin-20261018`) and checked against the corpus where the corpus shows them:
//!
//! * A state's `sequence` is not the `DEFAULT_STATES` number: `State.save()` gives a state it adds `largest + 15000`, over the
//!   project's states that are not triage. So the five visible states are 15000, 30000, 45000, 60000, 75000 (the corpus shows
//!   exactly these) and the triage state, saved last, 90000. The corpus never shows that one (the default manager hides it).
//! * A label's `sort_order` is 65535 for the first and `largest + 10000` for each after it (65535, 75535, 85535 in the corpus).
//! * A work item's `sort_order` is 65535 for the first of its state in the project and `largest + 10000` over the project's items
//!   of that state, **not** over the project (`Issue.save()`), and its number is the project's last plus one. The corpus shows the
//!   result (65535 to 105535); a project's counters therefore carry on from the largest number and place of its items.
//!
//! Time is an input: `now` makes the dates the seed gives relative to "today" (`start_date`, `target_date`), the creation time of
//! every row (so that a list in creation order has no ties), and the expiry of the one token that has expired.

use std::collections::HashMap;

use serde_json::{json, Map, Value};
use thrust_contracts::chrono::Duration;
use thrust_contracts::*;
use uuid::Uuid;

/// The namespace of every id `seed.py` makes (`NS`).
pub const NAMESPACE: Uuid = Uuid::from_u128(0x5a0a_0a0a_0000_4000_8000_0123_4567_89ab);

/// `seed.py`'s `uid`: the uuid5 of a name in [`NAMESPACE`].
pub fn uid(name: &str) -> Uuid {
    Uuid::new_v5(&NAMESPACE, name.as_bytes())
}

pub const WORKSPACE_SLUG: &str = "oracle-ws";

/// One of the oracle's four people. `workspace_role` and `alpha_role` are none for a person who is not a member of the workspace or
/// of project Alpha. A member of project Beta is its owner only, and every project member of Beta is an admin of it.
pub struct Person {
    pub key: &'static str,
    pub email: &'static str,
    pub first_name: &'static str,
    pub last_name: &'static str,
    pub workspace_role: Option<Role>,
    pub alpha_role: Option<Role>,
}

pub const USERS: [Person; 4] = [
    Person {
        key: "owner",
        email: "oracle-owner@example.test",
        first_name: "Olive",
        last_name: "Owner",
        workspace_role: Some(Role::Admin),
        alpha_role: Some(Role::Admin),
    },
    Person {
        key: "member",
        email: "oracle-member@example.test",
        first_name: "Milo",
        last_name: "Member",
        workspace_role: Some(Role::Member),
        alpha_role: Some(Role::Member),
    },
    Person {
        key: "guest",
        email: "oracle-guest@example.test",
        first_name: "Gwen",
        last_name: "Guest",
        workspace_role: Some(Role::Guest),
        alpha_role: Some(Role::Guest),
    },
    Person {
        key: "outsider",
        email: "oracle-outsider@example.test",
        first_name: "Otto",
        last_name: "Outsider",
        workspace_role: None,
        alpha_role: None,
    },
];

/// Key, name, identifier, network, the keys of its members. Alpha is public to the workspace; Beta is secret and has only the
/// owner, so that a workspace member's key gets a real "not allowed" answer from it.
pub const PROJECTS: [(&str, &str, &str, ProjectNetwork, &[&str]); 2] = [
    ("alpha", "Oracle Alpha", "ORA", ProjectNetwork::Public, &["owner", "member", "guest"]),
    ("beta", "Oracle Beta", "ORB", ProjectNetwork::Secret, &["owner"]),
];

/// Name, colour, sequence (see the module's note), group, whether it is the project's default.
const STATES: [(&str, &str, f64, StateGroup, bool); 6] = [
    ("Backlog", "#60646C", 15_000.0, StateGroup::Backlog, true),
    ("Todo", "#60646C", 30_000.0, StateGroup::Unstarted, false),
    ("In Progress", "#F59E0B", 45_000.0, StateGroup::Started, false),
    ("Done", "#46A758", 60_000.0, StateGroup::Completed, false),
    ("Cancelled", "#9AA4BC", 75_000.0, StateGroup::Cancelled, false),
    ("Triage", "#4E5355", 90_000.0, StateGroup::Triage, false),
];

/// Name and colour, in the order `seed.py` makes them (which is the order of their `sort_order`).
pub const LABELS: [(&str, &str); 3] = [("bug", "#E5484D"), ("feature", "#3E63DD"), ("docs", "#30A46C")];

/// One work item of the seed: key, name, state, priority, the keys of its assignees, the names of its labels, the key of its
/// parent, and the offsets in days from today of its start and its target. Made with [`Item::new`] and the methods that add to it,
/// so that a row of the table says only what it has.
#[derive(Clone, Copy)]
pub struct Item {
    pub key: &'static str,
    pub name: &'static str,
    pub state: &'static str,
    pub priority: Priority,
    pub assignees: &'static [&'static str],
    pub labels: &'static [&'static str],
    pub parent: Option<&'static str>,
    pub start: Option<i64>,
    pub target: Option<i64>,
}

impl Item {
    const fn new(key: &'static str, name: &'static str, state: &'static str, priority: Priority) -> Item {
        Item { key, name, state, priority, assignees: &[], labels: &[], parent: None, start: None, target: None }
    }
    /// Who it is assigned to, in the order `seed.py` assigns them.
    const fn assigned(self, assignees: &'static [&'static str]) -> Item {
        Item { assignees, ..self }
    }
    /// Its labels, in the order `seed.py` gives them.
    const fn labelled(self, labels: &'static [&'static str]) -> Item {
        Item { labels, ..self }
    }
    const fn under(self, parent: &'static str) -> Item {
        Item { parent: Some(parent), ..self }
    }
    /// Its start and target, in days from today.
    const fn dated(self, start: i64, target: i64) -> Item {
        Item { start: Some(start), target: Some(target), ..self }
    }
}

/// The work items of each project, in the order `seed.py` makes them: the number of an item is its place in this list.
pub const ALPHA_ITEMS: [Item; 9] = [
    Item::new("i1", "Oracle item one", "Backlog", Priority::None),
    Item::new("i2", "Oracle item two", "Todo", Priority::Medium).assigned(&["member"]).labelled(&["bug"]).dated(0, 7),
    Item::new("i3", "Oracle item three", "In Progress", Priority::High)
        .assigned(&["owner"])
        .labelled(&["feature", "docs"])
        .dated(-2, 5),
    Item::new("i4", "Oracle item four", "Done", Priority::Low).assigned(&["owner", "member"]).dated(-9, -1),
    Item::new("i5", "Oracle item five", "Cancelled", Priority::Urgent).labelled(&["bug"]),
    Item::new("i6", "Oracle sub-item of three", "Todo", Priority::None).assigned(&["member"]).under("i3").dated(1, 3),
    Item::new("i7", "Oracle item seven", "Backlog", Priority::Low).labelled(&["docs"]),
    Item::new("i8", "Oracle item eight", "Todo", Priority::Medium)
        .assigned(&["owner"])
        .labelled(&["feature"])
        .dated(3, 10),
    Item::new("i9", "Oracle item nine", "In Progress", Priority::None),
];
pub const BETA_ITEMS: [Item; 2] = [
    Item::new("b1", "Oracle beta item one", "Backlog", Priority::None),
    Item::new("b2", "Oracle beta item two", "Todo", Priority::High).assigned(&["owner"]),
];

/// Key, token, the key of its owner, whether it has expired, whether it is inactive. The tokens are fake and say so; the
/// `ratelimit` one is a second token of the owner's, which the oracle uses only where a low limit is set, to record a 429. The
/// `expired` and `inactive` ones are what the 403 answers are recorded with.
pub const TOKENS: [(&str, &str, &str, bool, bool); 7] = [
    ("owner", "plane_api_oracle_owner_00000000000000000001", "owner", false, false),
    ("member", "plane_api_oracle_member_0000000000000000002", "member", false, false),
    ("guest", "plane_api_oracle_guest_00000000000000000003", "guest", false, false),
    ("outsider", "plane_api_oracle_outsider_000000000000004", "outsider", false, false),
    ("ratelimit", "plane_api_oracle_ratelimit_000000000000005", "owner", false, false),
    ("expired", "plane_api_oracle_expired_00000000000000000006", "owner", true, false),
    ("inactive", "plane_api_oracle_inactive_0000000000000000007", "owner", false, true),
];

/// What [`oracle_seed`] makes: the fixture, and the manifest of what is in it, in the shape of the oracle's own
/// `seed-manifest.json` (restricted to the keys that are reproduced, see the module's note).
pub struct OracleSeed {
    pub fixture: SeedFixture,
    pub manifest: Value,
}

/// The oracle's seed, as it is when it is loaded at `now`.
pub fn oracle_seed(now: Timestamp) -> OracleSeed {
    let today = now.date_naive();
    // everything is made a minute ago and one millisecond after the row before it: a list in creation order has no ties, and
    // nothing is made in the future of the request that loads it
    let base = now - Duration::minutes(1);
    let mut tick = 0_i64;
    let mut made_at = || {
        let at = base + Duration::milliseconds(tick);
        tick += 1;
        at
    };

    let workspace_id = WorkspaceId(uid("workspace:oracle"));
    let user = |key: &str| UserId(uid(&format!("user:{key}")));
    let mut f = SeedFixture {
        workspaces: vec![SeedWorkspace { id: workspace_id, slug: WorkspaceSlug::new(WORKSPACE_SLUG) }],
        ..SeedFixture::default()
    };
    let mut users = Map::new();
    let mut projects = Map::new();
    let mut states = Map::new();
    let mut labels = Map::new();
    let mut issues = Map::new();
    let mut tokens = Map::new();

    for person in &USERS {
        let (key, email) = (person.key, person.email);
        let user_id = user(key);
        f.users.push(user_id);
        // no password: nobody signs in as an oracle user, and hashing one costs seconds
        f.logins.push(SeedLogin { user_id, email: email.to_string(), password: None, is_active: true });
        f.profiles.push(SeedProfile {
            user_id,
            first_name: person.first_name.to_string(),
            last_name: person.last_name.to_string(),
            display_name: format!("oracle_{key}"),
        });
        if let Some(role) = person.workspace_role {
            f.workspace_roles.push(SeedWorkspaceRole { workspace_id, user_id, role });
        }
        users.insert(key.to_string(), json!({ "id": user_id.0.to_string(), "email": email }));
    }

    // the states a work item names, by project and name, and the labels by project and name
    let mut state_ids: HashMap<(&str, &str), Uuid> = HashMap::new();
    let mut label_ids: HashMap<(&str, &str), Uuid> = HashMap::new();
    let mut item_ids: HashMap<&str, Uuid> = HashMap::new();

    for (pkey, name, identifier, network, member_keys) in PROJECTS {
        let project_id = ProjectId(uid(&format!("project:{pkey}")));
        f.projects.push(SeedProject { id: project_id, workspace_id, name: name.to_string() });
        f.project_details.push(SeedProjectDetails {
            project_id,
            identifier: Some(identifier.to_string()),
            network,
            archived: false,
            project_lead: Some(user("owner")),
            created_by: None,
            created_at: Some(made_at()),
            description: format!("Synthetic project {name} for the v1 oracle."),
            // `seed.py` sets `project.default_state` to Backlog after it has made the project's states
            default_state: Some(uid(&format!("state:{pkey}:Backlog"))),
        });
        for member in member_keys {
            let role = match USERS.iter().find(|u| u.key == *member) {
                Some(u) if pkey == "alpha" => u.alpha_role,
                _ => Some(Role::Admin),
            };
            if let Some(role) = role {
                f.memberships.push(SeedMembership { project_id, workspace_id, user_id: user(member), role });
            }
        }
        projects.insert(pkey.to_string(), json!({ "id": project_id.0.to_string(), "identifier": identifier }));

        for (sname, color, sequence, group, is_default) in STATES {
            let id = uid(&format!("state:{pkey}:{sname}"));
            f.states.push(SeedState {
                id,
                project_id,
                workspace_id,
                name: sname.to_string(),
                color: color.to_string(),
                sequence,
                group,
                is_default,
            });
            state_ids.insert((pkey, sname), id);
            states.insert(format!("{pkey}:{sname}"), Value::String(id.to_string()));
        }
        for (n, (lname, color)) in LABELS.into_iter().enumerate() {
            let id = uid(&format!("label:{pkey}:{lname}"));
            f.labels.push(SeedLabel {
                id,
                project_id,
                workspace_id,
                name: lname.to_string(),
                color: color.to_string(),
                parent_id: None,
                sort_order: 65_535.0 + 10_000.0 * n as f64,
            });
            label_ids.insert((pkey, lname), id);
            labels.insert(format!("{pkey}:{lname}"), Value::String(id.to_string()));
        }

        let list: &[Item] = if pkey == "alpha" { &ALPHA_ITEMS } else { &BETA_ITEMS };
        let mut largest: HashMap<&str, i64> = HashMap::new();
        for (n, it) in list.iter().enumerate() {
            let id = uid(&format!("issue:{}", it.key));
            let sequence_id = n as i64 + 1;
            let sort_order = largest.get(it.state).map_or(65_535, |m| m + 10_000);
            largest.insert(it.state, sort_order);
            let created_at = made_at();
            let group = STATES.iter().find(|s| s.0 == it.state).map(|s| s.3);
            f.work_items.push(SeedIssue {
                id: WorkItemId(id),
                workspace_id,
                project_id,
                sequence_id,
                sort_order,
                name: it.name.to_string(),
                description: Some(format!("Synthetic description for {}.", it.name)),
                description_html: format!("<p>Synthetic description for {}.</p>", it.name),
                state_id: state_ids.get(&(pkey, it.state)).copied(),
                priority: it.priority,
                parent_id: it.parent.and_then(|p| item_ids.get(p)).copied(),
                start_date: it.start.map(|d| today + Duration::days(d)),
                target_date: it.target.map(|d| today + Duration::days(d)),
                completed_at: (group == Some(StateGroup::Completed)).then_some(created_at),
                assignee_ids: it.assignees.iter().map(|a| user(a)).collect(),
                label_ids: it.labels.iter().filter_map(|l| label_ids.get(&(pkey, *l))).copied().collect(),
                created_by: None,
                created_at,
            });
            item_ids.insert(it.key, id);
            issues.insert(it.key.to_string(), json!({ "id": id.to_string(), "sequence_id": sequence_id }));
        }
    }

    for (key, token, owner, expired, inactive) in TOKENS {
        f.api_keys.push(SeedApiKey {
            token: token.to_string(),
            user_id: user(owner),
            label: format!("oracle-{key}"),
            expires_at: expired.then(|| now - Duration::days(1)),
            revoked: inactive,
        });
        tokens.insert(key.to_string(), Value::String(token.to_string()));
    }

    let manifest = json!({
        "workspace": { "id": workspace_id.0.to_string(), "slug": WORKSPACE_SLUG },
        "users": users,
        "projects": projects,
        "states": states,
        "labels": labels,
        "issues": issues,
        "tokens": tokens,
    });
    OracleSeed { fixture: f, manifest }
}
