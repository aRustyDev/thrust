//! The states of a project as Plane's v1 API has them (issue #58): listed, opened, made, changed and deleted.
//!
//! Who may write is the caller's to decide when it makes the [`Scope`]: [`crate::Permission::WriteState`] is for a project admin only,
//! and a read is for any member. What the pin does is reproduced as it is, including its order of checks:
//!
//! * a read shows the live states that are not the intake one, and none of an archived project; a change and a deletion do not look at
//!   whether the project is archived (the pin's views differ there);
//! * a creation refuses the group `triage`, then an integration's key that another state has, then writes; a change finds its state
//!   first (a state that is not there is not found before anything else is read), then refuses the group `triage`, then the key;
//! * a new state takes Plane's place (the database does that), and a state that is the default takes the default from the others;
//! * a name that a state has is refused by the database's unique index; the pin answers a clash with a state it does not show (the
//!   intake one) by failing, which is an `Internal` here.

use thiserror::Error;
use thrust_contracts::*;
use thrust_ports::*;

use crate::{App, CoreError};

/// The place Plane's model gives a state that nothing else places (`State.sequence`'s default): the first state of a project has it.
pub const FIRST_STATE_SEQUENCE: f64 = 65535.0;

/// Why a state's use-case refused.
#[derive(Debug, Error)]
pub enum StateError {
    /// No such state in the project, as the pin's queryset has it.
    #[error("no such state")]
    NotFound,
    /// Deleting the project's default state.
    #[error("the default state cannot be deleted")]
    DefaultState,
    /// Deleting a state that has work items in it.
    #[error("the state is not empty")]
    NotEmpty,
    /// A state of the project already has this name. `id` is the one the caller can see, if it is one (a clash with the intake state
    /// is not one the pin shows).
    #[error("a state of this name exists")]
    NameTaken { id: Option<Uuid> },
    /// Another state of the project has this integration key; `id` is the state the pin names in its answer.
    #[error("a state with this external key exists")]
    ExternalTaken { id: Uuid },
    /// The group `triage` is the intake's: a state is not made or changed to it.
    #[error("a state cannot be made a triage state")]
    TriageGroup,
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl From<DbError> for StateError {
    fn from(e: DbError) -> Self {
        StateError::Core(e.into())
    }
}

/// What a request may set of a state, each only when it names it. A field that holds an `Option` says whether the request names it
/// (the outer one) and what it sets it to (the inner, `None` for null).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatePatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub sequence: Option<f64>,
    pub group: Option<StateGroup>,
    pub is_default: Option<bool>,
    pub external_source: Option<Option<String>>,
    pub external_id: Option<Option<String>>,
}

/// The other state of the project that has this key, if there is one.
fn with_key<'a>(states: &'a [StateRow], source: Option<&str>, key: &str) -> Option<&'a StateRow> {
    states.iter().find(|o| o.external_id.as_deref() == Some(key) && o.external_source.as_deref() == source)
}

/// What a read of the project's states shows: the live ones that are not the intake state, by `sequence`; none at all in a project
/// that is archived.
pub async fn visible_states(app: &App, scope: &Scope) -> Result<Vec<StateRow>, StateError> {
    if app.ports.db.project_is_archived(scope).await? {
        return Ok(Vec::new());
    }
    Ok(app.ports.db.project_states(scope).await?)
}

/// One state as a read finds it.
pub async fn visible_state(app: &App, scope: &Scope, id: Uuid) -> Result<StateRow, StateError> {
    visible_states(app, scope).await?.into_iter().find(|s| s.id == id).ok_or(StateError::NotFound)
}

/// A state as the pin's change and deletion find it: live and not the intake one, whatever the project's archive says (a read looks
/// at that; these do not). `NotFound` for any other, so a request about one is answered before its body is looked at.
pub async fn find_state(app: &App, scope: &Scope, id: Uuid) -> Result<StateRow, StateError> {
    app.ports.db.project_states(scope).await?.into_iter().find(|s| s.id == id).ok_or(StateError::NotFound)
}

/// Make a state. It needs a name and a colour; the group is `backlog` unless it says; its place is the database's; it is the default
/// only if it says so, and then the others are not.
pub async fn create_state(app: &App, scope: &Scope, patch: StatePatch) -> Result<StateRow, StateError> {
    let StatePatch { name, description, color, sequence, group, is_default, external_source, external_id } = patch;
    let (Some(name), Some(color)) = (name, color) else {
        return Err(CoreError::Invalid("a state needs a name and a colour".into()).into());
    };
    let group = group.unwrap_or(StateGroup::Backlog);
    if group == StateGroup::Triage {
        return Err(StateError::TriageGroup);
    }
    let (external_source, external_id) = (external_source.flatten(), external_id.flatten());
    let others = app.ports.db.project_states(scope).await?;
    // the pin refuses a key only when the request names both halves of it
    if let (Some(source), Some(key)) =
        (external_source.as_deref().filter(|s| !s.is_empty()), external_id.as_deref().filter(|k| !k.is_empty()))
    {
        if let Some(taken) = with_key(&others, Some(source), key) {
            return Err(StateError::ExternalTaken { id: taken.id });
        }
    }
    let now = app.ports.clock.now();
    let row = StateRow {
        id: app.ports.ids.new_uuid(),
        workspace_id: scope.workspace_id(),
        project_id: scope.project_id(),
        name,
        description: description.unwrap_or_default(),
        color,
        sequence: sequence.unwrap_or(FIRST_STATE_SEQUENCE),
        group,
        is_default: is_default.unwrap_or(false),
        created_by: Some(scope.user_id()),
        created_at: now,
        updated_at: now,
        external_source,
        external_id,
        updated_by: None,
    };
    match app.ports.db.insert_state(scope, &row).await {
        Ok(stored) => Ok(stored),
        Err(DbError::Conflict(_)) => match others.iter().find(|o| o.name == row.name) {
            Some(o) => Err(StateError::NameTaken { id: Some(o.id) }),
            // the clash is with a state the pin does not show: its lookup finds none, and the view fails
            None => Err(CoreError::Internal("a new state's name is the intake state's".into()).into()),
        },
        Err(e) => Err(e.into()),
    }
}

/// Change a state: the fields the patch names are set, whatever the project's archive says; who changed it and when are saved with it.
pub async fn change_state(app: &App, scope: &Scope, id: Uuid, patch: StatePatch) -> Result<StateRow, StateError> {
    let states = app.ports.db.project_states(scope).await?;
    let Some(current) = states.iter().find(|s| s.id == id) else {
        return Err(StateError::NotFound);
    };
    if patch.group == Some(StateGroup::Triage) {
        return Err(StateError::TriageGroup);
    }
    // a key that is new, with the source the request names or else the state's own, may not be another state's; the pin names the
    // state that is being changed in its answer, not the one that has the key
    let new_key = patch.external_id.as_ref().and_then(|k| k.as_deref()).filter(|k| !k.is_empty());
    if let Some(key) = new_key.filter(|k| current.external_id.as_deref() != Some(*k)) {
        let source = match &patch.external_source {
            Some(named) => named.as_deref(),
            None => current.external_source.as_deref(),
        };
        if with_key(&states, source, key).is_some() {
            return Err(StateError::ExternalTaken { id: current.id });
        }
    }

    let mut row = current.clone();
    if let Some(v) = patch.name {
        row.name = v;
    }
    if let Some(v) = patch.description {
        row.description = v;
    }
    if let Some(v) = patch.color {
        row.color = v;
    }
    if let Some(v) = patch.sequence {
        row.sequence = v;
    }
    if let Some(v) = patch.group {
        row.group = v;
    }
    if let Some(v) = patch.is_default {
        row.is_default = v;
    }
    if let Some(v) = patch.external_source {
        row.external_source = v;
    }
    if let Some(v) = patch.external_id {
        row.external_id = v;
    }
    row.updated_by = Some(scope.user_id());
    row.updated_at = app.ports.clock.now();
    match app.ports.db.update_state(scope, &row).await {
        Ok(true) => {}
        Ok(false) => return Err(StateError::NotFound),
        Err(DbError::Conflict(_)) => {
            let other = states.iter().find(|o| o.id != id && o.name == row.name).map(|o| o.id);
            return Err(StateError::NameTaken { id: other });
        }
        Err(e) => return Err(e.into()),
    }
    app.ports.db.project_states(scope).await?.into_iter().find(|s| s.id == id).ok_or(StateError::NotFound)
}

/// Delete a state: not the default, and not one that has work items in it (in the pin's sense: not a draft, an archived or a deleted
/// one, and none in an archived project).
pub async fn delete_state(app: &App, scope: &Scope, id: Uuid) -> Result<(), StateError> {
    let states = app.ports.db.project_states(scope).await?;
    let Some(state) = states.iter().find(|s| s.id == id) else {
        return Err(StateError::NotFound);
    };
    if state.is_default {
        return Err(StateError::DefaultState);
    }
    if app.ports.db.state_has_work_items(scope, id).await? {
        return Err(StateError::NotEmpty);
    }
    if !app.ports.db.delete_state(scope, id, app.ports.clock.now()).await? {
        return Err(StateError::NotFound);
    }
    Ok(())
}
