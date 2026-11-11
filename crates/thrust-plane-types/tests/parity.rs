//! `parity_plane_500_bodies` (#51): each `Plane500` kind answers the status and the body bytes the pinned Plane gives.
//!
//! The fixtures are in `tests/parity/fixtures/`. The base text is the oracle's own recording (`workspace-asset-upload` in
//! `tools/oracle-v1/corpus/assets.json`), and a test below ties the fixture to that file, so the fixture cannot drift from it.
//! The other two texts were never reached by the harness, so their fixtures cite the pinned source (AGENTS.md testing rule 9b).
//!
//! The corpus keeps a body as a parsed JSON value, not as the bytes the pin sent. The bytes compared here are therefore the
//! compact serialisation of the recorded value, which is what DRF's JSON renderer writes by default (`COMPACT_JSON`); that the
//! pin leaves the default alone is read from its output shape and not recorded.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

use std::path::PathBuf;

use serde_json::{json, Value};
use thrust_plane_types::{plane_500, Plane500};

const ALL: [Plane500; 3] = [Plane500::SomethingWentWrong, Plane500::UnexpectedError, Plane500::InternalServerError];

/// A new kind is a compile error here until it has a fixture file; `parity_the_fixtures_are_one_file_per_kind` holds the
/// directory to this list.
fn fixture_file(kind: Plane500) -> &'static str {
    match kind {
        Plane500::SomethingWentWrong => "something-went-wrong.json",
        Plane500::UnexpectedError => "unexpected-error.json",
        Plane500::InternalServerError => "internal-server-error.json",
    }
}

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures_dir() -> PathBuf {
    manifest().join("tests/parity/fixtures")
}

fn corpus_dir() -> PathBuf {
    manifest().join("../../tools/oracle-v1/corpus")
}

fn read_json(path: &std::path::Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn fixture(kind: Plane500) -> Value {
    read_json(&fixtures_dir().join(fixture_file(kind)))
}

/// What `plane_500(kind, ..)` answers: status, content type and the body's bytes.
fn answer(kind: Plane500) -> (u16, Option<String>, Vec<u8>) {
    let resp = plane_500(kind, "a cause that must not reach the body");
    let status = resp.status().as_u16();
    let content_type = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let bytes = futures_executor::block_on(axum::body::to_bytes(resp.into_body(), usize::MAX)).unwrap();
    (status, content_type, bytes.to_vec())
}

/// Where the answer for `kind` differs from what `recorded` (a fixture, possibly mutated by a control) says it should be.
fn divergences(kind: Plane500, recorded: &Value) -> Vec<String> {
    let (status, content_type, bytes) = answer(kind);
    let mut out = Vec::new();
    if recorded["kind"] != json!(format!("{kind:?}")) {
        out.push(format!("fixture is for {}, not {kind:?}", recorded["kind"]));
    }
    if recorded["status"] != json!(status) {
        out.push(format!("status {status}, recorded {}", recorded["status"]));
    }
    let expected = serde_json::to_vec(&recorded["body"]).unwrap();
    if bytes != expected {
        out.push(format!(
            "body {:?}, recorded {:?}",
            String::from_utf8_lossy(&bytes),
            String::from_utf8_lossy(&expected)
        ));
    }
    if content_type.as_deref() != Some("application/json") {
        out.push(format!("content-type {content_type:?}, DRF's JSON renderer sends application/json"));
    }
    out
}

#[test]
fn parity_plane_500_bodies() {
    for kind in ALL {
        let d = divergences(kind, &fixture(kind));
        assert!(d.is_empty(), "{kind:?} diverges from its fixture:\n{}", d.join("\n"));
    }
}

#[test]
fn parity_the_fixtures_are_one_file_per_kind() {
    let mut on_disk: Vec<String> = std::fs::read_dir(fixtures_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let mut named: Vec<String> = ALL.iter().map(|k| fixture_file(*k).to_owned()).collect();
    named.sort();
    assert_eq!(on_disk, named, "tests/parity/fixtures/ and the kinds must name the same files");
}

/// The base text's fixture is the oracle's recording, not a second opinion of it.
#[test]
fn parity_the_base_text_is_the_oracles_recording() {
    let corpus = read_json(&corpus_dir().join("assets.json"));
    let exchange = corpus["exchanges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "workspace-asset-upload")
        .expect("assets.json has the exchange the fixture cites");
    let fx = fixture(Plane500::SomethingWentWrong);
    assert_eq!(exchange["response"]["status"], fx["status"]);
    assert_eq!(exchange["response"]["body"], fx["body"]);
}

fn is_one_of_the_three(body: &Value) -> bool {
    ALL.iter().any(|k| *body == json!({ "error": k.message() }))
}

/// Every 500 the oracle recorded on the pin, in every corpus file, is one of the three texts. A fourth text, or a 500 in the
/// Thrust shape, would be a reproduced 500 that `Plane500` cannot say.
#[test]
fn parity_every_500_the_oracle_recorded_is_one_of_the_three_texts() {
    let mut files: Vec<PathBuf> = std::fs::read_dir(corpus_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let mut seen = Vec::new();
    for file in &files {
        let corpus = read_json(file);
        let Some(exchanges) = corpus.get("exchanges").and_then(Value::as_array) else { continue };
        for e in exchanges.iter().filter(|e| e["response"]["status"] == 500) {
            let name = format!("{}#{}", file.file_name().unwrap().to_string_lossy(), e["name"].as_str().unwrap_or("?"));
            assert!(is_one_of_the_three(&e["response"]["body"]), "{name}: {}", e["response"]["body"]);
            seen.push(name);
        }
    }
    // not vacuous: the corpus held four on 2026-11-11 (assets x2, pagination x2), and a scan that finds none has stopped looking
    for needed in ["assets.json#workspace-asset-upload", "pagination.json#per-page-zero"] {
        assert!(seen.iter().any(|s| s == needed), "the scan did not reach {needed}: {seen:?}");
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Controls (AGENTS.md testing rule 8): each makes the comparison fail on purpose. A pass above with one of these unable to
// fail would prove nothing.
// ---------------------------------------------------------------------------------------------------------------------

/// Changing one byte of a fixture makes the comparison fail, for each kind.
#[test]
fn parity_control_one_changed_byte_in_a_fixture_is_a_divergence() {
    for kind in ALL {
        let mut fx = fixture(kind);
        let text = fx["body"]["error"].as_str().unwrap().to_owned();
        let mut bytes = text.into_bytes();
        let last = bytes.len() - 1;
        bytes[last] = if bytes[last] == b'x' { b'y' } else { b'x' };
        fx["body"]["error"] = json!(String::from_utf8(bytes).unwrap());
        assert!(!divergences(kind, &fx).is_empty(), "{kind:?}: a changed byte went unseen");
    }
}

#[test]
fn parity_control_a_changed_status_is_a_divergence() {
    for kind in ALL {
        let mut fx = fixture(kind);
        fx["status"] = json!(502);
        assert!(!divergences(kind, &fx).is_empty(), "{kind:?}: a changed status went unseen");
    }
}

/// The kinds are distinct: one kind's answer is not accepted against another kind's fixture.
#[test]
fn parity_control_a_kind_does_not_match_another_kinds_fixture() {
    for kind in ALL {
        for other in ALL.iter().filter(|o| **o != kind) {
            assert!(
                !divergences(kind, &fixture(*other)).is_empty(),
                "{kind:?} was accepted against {other:?}'s fixture"
            );
        }
    }
}

/// The corpus scan can fail: a 500 in the Thrust shape, or with a text that is not one of the three, is refused.
#[test]
fn parity_control_a_500_that_is_not_one_of_the_three_is_refused() {
    assert!(is_one_of_the_three(&json!({ "error": "Something went wrong please try again later" })));
    assert!(!is_one_of_the_three(&json!({ "error": "Something went wrong" })));
    assert!(!is_one_of_the_three(&json!({ "error": { "code": "internal", "message": "internal error" } })));
    assert!(!is_one_of_the_three(&json!({ "detail": "Something went wrong please try again later" })));
}
