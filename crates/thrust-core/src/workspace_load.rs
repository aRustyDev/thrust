//! What the app reads when a member opens a workspace (issue #44): the projects they can see, and the rows that are theirs
//! alone, which they get on their first visit: the sidebar's entries, the home page's widgets and the settings of the
//! work-item lists. Each read takes a [`WorkspaceScope`], so only an active member of the workspace can ask.
//!
//! A member's rows are made the first time they ask, from the defaults below (the pin's), and are never changed by asking
//! again: the adapter inserts what is missing and returns what is there.

use serde_json::{json, Value};
use thrust_ports::*;

use crate::{App, CoreError};

/// The sidebar's entries in the order the pin makes them, and whether each starts pinned. A key's place in this list gives its
/// default `sort_order`: 65535 for the first, and 10000 more for each after it.
pub const SIDEBAR_KEYS: [(&str, bool); 7] = [
    ("views", false),
    ("active_cycles", false),
    ("analytics", false),
    ("drafts", true),
    ("your_work", true),
    ("archives", false),
    ("stickies", true),
];

/// The home page's widgets in the order the pin makes them (`quick_tutorial` and `new_at_plane` are in its enum but are never
/// made). A widget's place gives its default `sort_order`: 999 for the first, and one less for each after it.
pub const HOME_KEYS: [&str; 3] = ["quick_links", "recents", "my_stickies"];

/// The projects of the workspace the caller sees, newest first: all of them for an admin, the ones they are in and the public
/// ones for a member, the ones they are in for a guest.
pub async fn workspace_projects(app: &App, scope: &WorkspaceScope) -> Result<Vec<ProjectListing>, CoreError> {
    Ok(app.ports.db.workspace_projects(scope).await?)
}

/// The caller's sidebar for this workspace, by `sort_order`, made on their first visit.
pub async fn sidebar_preferences(app: &App, scope: &WorkspaceScope) -> Result<Vec<SidebarPreference>, CoreError> {
    let defaults: Vec<SidebarPreference> = SIDEBAR_KEYS
        .iter()
        .enumerate()
        .map(|(i, (key, pinned))| SidebarPreference { key: (*key).to_string(), is_pinned: *pinned, sort_order: 65535.0 + 10000.0 * i as f64 })
        .collect();
    Ok(app.ports.db.ensure_sidebar_preferences(scope, &defaults, app.ports.clock.now()).await?)
}

/// The caller's home page widgets for this workspace, made on their first visit, in the order the pin lists them: the one it
/// made last first (its rows are listed by the time they were made, newest first, and it makes them in `HOME_KEYS` order).
pub async fn home_preferences(app: &App, scope: &WorkspaceScope) -> Result<Vec<HomePreference>, CoreError> {
    let now = app.ports.clock.now();
    let defaults: Vec<HomePreference> = HOME_KEYS
        .iter()
        .enumerate()
        .map(|(i, key)| HomePreference {
            key: (*key).to_string(),
            is_enabled: true,
            config: json!({}),
            sort_order: 999.0 - i as f64,
            created_at: now,
        })
        .collect();
    let mut rows = app.ports.db.ensure_home_preferences(scope, &defaults).await?;
    // rows made together share a time, so the pin's order of making breaks the tie; a key that is not one of ours sorts first
    let made = |key: &str| HOME_KEYS.iter().position(|k| *k == key);
    rows.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| made(&b.key).cmp(&made(&a.key))));
    Ok(rows)
}

/// The caller's work-item list settings for this workspace: the pin's defaults on their first visit.
pub async fn user_properties(app: &App, scope: &WorkspaceScope) -> Result<UserProperties, CoreError> {
    let now = app.ports.clock.now();
    let defaults = UserProperties {
        id: app.ports.ids.new_uuid(),
        filters: json!({
            "priority": null, "state": null, "state_group": null, "assignees": null, "created_by": null,
            "labels": null, "start_date": null, "target_date": null, "subscriber": null,
        }),
        // the pin stores these two one level deeper than the other lists' (`{"display_filters": {...}}`), and the app reads
        // them so
        display_filters: json!({"display_filters": {
            "group_by": null, "order_by": "-created_at", "type": null, "sub_issue": true, "show_empty_groups": true,
            "layout": "list", "calendar_date_range": "",
        }}),
        display_properties: json!({"display_properties": {
            "assignee": true, "attachment_count": true, "created_on": true, "due_date": true, "estimate": true,
            "key": true, "labels": true, "link": true, "priority": true, "start_date": true, "state": true,
            "sub_issue_count": true, "updated_on": true,
        }}),
        rich_filters: Value::Object(Default::default()),
        navigation_project_limit: 10,
        navigation_control_preference: "ACCORDION".to_string(),
        created_at: now,
        updated_at: now,
    };
    Ok(app.ports.db.ensure_user_properties(scope, &defaults).await?)
}
