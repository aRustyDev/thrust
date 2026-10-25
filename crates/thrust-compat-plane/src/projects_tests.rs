//! `saved` and `taken` of `projects.rs`: the answer to a creation or a change that the database refused for a name or a key that
//! another request took first. Through the app that path needs a race (the handler asks for clashes before it writes, and
//! answers those itself), so no recorded exchange reaches it; the pin's two codes are the ones its serializer gives for the same
//! clash (`PROJECT_NAME_ALREADY_EXIST`, `PROJECT_IDENTIFIER_ALREADY_EXIST`), and the unique index that refused (migration 0012)
//! picks the field. The cases are examples the pin names (rule 9 (b)); the race itself is held by `pg_adapter.rs`.

use rstest::rstest;
use serde_json::json;
use thrust_core::{CoreError, ProjectError};

use crate::error::PlaneError;
use crate::projects::saved;

fn fields(e: PlaneError) -> serde_json::Value {
    match e {
        PlaneError::Fields(map) => serde_json::Value::Object(map),
        _ => panic!("not a field error"),
    }
}

#[rstest]
#[case::the_name_index("projects_workspace_name_key", json!({"name": ["PROJECT_NAME_ALREADY_EXIST"]}))]
#[case::the_key_index("projects_workspace_identifier_key", json!({"identifier": ["PROJECT_IDENTIFIER_ALREADY_EXIST"]}))]
fn a_name_or_key_another_request_took_first_is_the_field_error_of_the_index_that_refused(#[case] constraint: &str, #[case] want: serde_json::Value) {
    let got = saved(ProjectError::Core(CoreError::Conflict(constraint.into())));
    assert_eq!(fields(got), want);
}

#[test]
fn any_other_refusal_is_answered_as_the_route_answers_it() {
    assert!(matches!(saved(ProjectError::Forbidden), PlaneError::Forbidden(_)));
    assert!(matches!(saved(ProjectError::NotFound), PlaneError::NotFound(_)));
    assert!(matches!(saved(ProjectError::Archived), PlaneError::BadRequest(_)));
}
