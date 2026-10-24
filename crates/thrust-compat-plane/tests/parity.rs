//! One `parity_` test per compat row: the route answers what the pinned Plane (`oracle/v1-pin-20261018`, `ba85d4d3d3`)
//! recorded, after both are normalised by the same rules (tests/parity/normalise.json). The fixtures are output recorded
//! from that independent oracle by tools/compat-record/record.py; they are never edited by hand (AGENTS.md testing rule
//! 9c). A row's fixture holds every exchange the issue's acceptance criteria name for it, the 401/403/404 ones included.

mod common;

use common::*;
use serde_json::{json, Value};

/// `parity_<row>` per row, and `ROWS`, the list the guard below holds the fixtures directory to.
macro_rules! rows {
    ($($test:ident => $row:literal),+ $(,)?) => {
        const ROWS: &[&str] = &[$($row),+];
        $(
            #[test]
            fn $test() {
                let d = divergences(&app(), &load_fixture($row));
                assert!(d.is_empty(), "row {} diverges from the pinned Plane:\n{}", $row, d.join("\n"));
            }
        )+
    };
}

rows! {
    parity_s01_instances => "S01",
    parity_s02_users_me_signed_out => "S02",
}

/// A fixture without a test would pass unseen, and a test without a fixture is `load_fixture` panicking: hold the two
/// lists equal, so adding a row means adding both.
#[test]
fn parity_every_fixture_has_a_test() {
    let rows: Vec<String> = ROWS.iter().map(|r| r.to_string()).collect();
    assert_eq!(fixture_rows(), rows, "tests/parity/fixtures/*.json and the rows! list must name the same rows");
}

// ---------------------------------------------------------------------------------------------------------------------
// Controls (AGENTS.md testing rule 8): the harness must be able to fail. Each mutates a recording in memory and
// requires the replay to report a divergence; a pass here with the mutation unseen would void every row above.
// ---------------------------------------------------------------------------------------------------------------------

fn mutated(row: &str, edit: impl FnOnce(&mut Value)) -> Value {
    let mut f = load_fixture(row);
    edit(&mut f);
    f
}

#[test]
fn parity_control_unmutated_fixture_has_no_divergence() {
    // the baseline the controls below are measured against
    assert!(divergences(&app(), &load_fixture("S01")).is_empty());
}

#[test]
fn parity_control_renamed_field_is_a_divergence() {
    let f = mutated("S01", |f| {
        let instance = &mut f["exchanges"][0]["response"]["body"]["instance"];
        let v = instance.as_object_mut().unwrap().remove("is_setup_done").unwrap();
        instance["is_setup_complete"] = v;
    });
    assert!(!divergences(&app(), &f).is_empty());
}

#[test]
fn parity_control_changed_status_is_a_divergence() {
    let f = mutated("S02", |f| f["exchanges"][0]["response"]["status"] = json!(403));
    assert!(!divergences(&app(), &f).is_empty());
}

#[test]
fn parity_control_changed_value_is_a_divergence() {
    let f = mutated("S01", |f| {
        f["exchanges"][0]["response"]["body"]["config"]["enable_signup"] = json!(false);
    });
    assert!(!divergences(&app(), &f).is_empty());
}

#[test]
fn parity_control_changed_type_is_a_divergence() {
    // Plane sends file_size_limit as a float; 5242880 and 5242880.0 are different JSON numbers to serde_json
    let f = mutated("S01", |f| {
        f["exchanges"][0]["response"]["body"]["config"]["file_size_limit"] = json!(5_242_880);
    });
    assert!(!divergences(&app(), &f).is_empty());
}

/// The normaliser could erase a real difference (the failure mode its rules exist to avoid): two answers that differ
/// in an id's *shape of use* must still differ. Here the same id appearing twice versus two different ids.
#[test]
fn parity_control_normaliser_keeps_id_identity() {
    let rules = normalise::Rules::load();
    let ex = |a: &str, b: &str| {
        json!({"name": "n", "request": {"method": "GET", "path": "/"},
               "response": {"status": 200, "headers": {}, "body": {"x": a, "y": b}}})
    };
    let (u1, u2) = ("11111111-1111-1111-1111-111111111111", "22222222-2222-2222-2222-222222222222");
    let same = normalise::exchange(&rules, &mut normalise::Ids::new(&rules), &ex(u1, u1));
    let different = normalise::exchange(&rules, &mut normalise::Ids::new(&rules), &ex(u1, u2));
    assert_ne!(same, different);
}

/// The cases the Python normaliser (which records the fixtures) passes in `record.py --self-test`.
#[test]
fn parity_normaliser_cases() {
    let cases: Value = serde_json::from_str(include_str!("parity/normalise-cases.json")).unwrap();
    let rules = normalise::Rules::load();
    let cases = cases["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let mut ids = normalise::Ids::new(&rules);
        let got: Vec<Value> = case["input"].as_array().unwrap().iter().map(|ex| normalise::exchange(&rules, &mut ids, ex)).collect();
        assert_eq!(Value::Array(got), case["expected"], "case: {}", case["name"]);
    }
}
