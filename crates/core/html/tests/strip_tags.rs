//! `strip_tags` against what Django's function answered (issue #48): `parity_`, output recorded from an independent oracle and never
//! edited by hand (AGENTS.md testing rule 9 (c)). The fixture is `tests/parity/strip_tags.json`, written by `tools/strip-tags/record.py`
//! from the oracle `django_strip_tags` of tools/spike-0006/sanitize/run.py, on the Python it names. A case is an input and the text the
//! function made of it. The inputs are what `strip_tags` is given in production, the default sanitiser's output for every entry of the
//! XSS corpus (`xss:<id>`) and every document of the editor corpus (`doc:<id>`), and 61 hand-written ones (`edge:<n>`).
//!
//! The port is the algorithm behind a work item's `description_stripped`, and mutation testing found it untested before this file: 77 of
//! the sanitiser crate's 127 mutants, all of them in `strip_tags`, `strip_once` and `tag_end`, survived the tests of the sanitiser.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "test code")]
use serde_json::Value;
use thrust_html::strip_tags;

/// The cases on which the port does not answer what Python 3.12's parser does, and why. Each is hand-written input that the sanitiser
/// never lets through (it removes a `<script>` with its text, it quotes every attribute, and it escapes every `&` it does not mean as a
/// reference), so no description reaches `strip_tags` in any of these forms; they are here so that the limits are known and so that a
/// change to one, a fix or a regression, is seen. The test asserts that exactly these differ.
const KNOWN_GAPS: [(&str, &str); 3] = [
    ("edge:26", "an attribute value whose quote is never closed: Python reads to the end of the text, the port finds a tag"),
    ("edge:51", "the text of a `<script>`: Python's parser reads it as data up to `</script>`, the port reads a tag in it"),
    ("edge:66", "a malformed character reference (`&#xZZ;`, `&#;`): Python's parser stops looking for tags after it and returns the rest as text, so a later `</p>` stays; the port keeps stripping"),
];

fn cases() -> Vec<Value> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/parity/strip_tags.json");
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    fixture["cases"].as_array().unwrap().clone()
}

/// The ids of the cases on which `strip` does not answer what the oracle did.
fn divergences(strip: impl Fn(&str) -> String, cases: &[Value]) -> Vec<String> {
    cases
        .iter()
        .filter(|c| strip(c["in"].as_str().unwrap()) != c["out"].as_str().unwrap())
        .map(|c| c["from"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn parity_strip_tags_matches_the_recorded_django_function() {
    let cases = cases();
    // a fixture that lost its cases must not pass by comparing less
    let kind = |k: &str| cases.iter().filter(|c| c["from"].as_str().unwrap().starts_with(&format!("{k}:"))).count();
    assert!(
        kind("xss") >= 160 && kind("doc") >= 190 && kind("edge") >= 50,
        "{} xss, {} doc, {} edge",
        kind("xss"),
        kind("doc"),
        kind("edge")
    );
    assert!(
        cases.iter().filter(|c| !c["out"].as_str().unwrap().is_empty()).count() > 300,
        "most cases have text to compare"
    );

    let differ = divergences(strip_tags, &cases);
    let known: Vec<&str> = KNOWN_GAPS.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        differ.iter().map(String::as_str).collect::<Vec<_>>(),
        known,
        "of {} cases; the known gaps are {KNOWN_GAPS:?}",
        cases.len()
    );
}

/// Control: a recording that was changed is a divergence, and names the case.
#[test]
fn parity_control_a_changed_expectation_is_a_divergence() {
    let mut cases = cases();
    let victim = cases
        .iter()
        .position(|c| c["from"].as_str().unwrap().starts_with("doc:") && !c["out"].as_str().unwrap().is_empty())
        .unwrap();
    let id = cases[victim]["from"].as_str().unwrap().to_string();
    cases[victim]["out"] = Value::String("not what Django answered".into());
    let differ = divergences(strip_tags, &cases);
    assert!(differ.contains(&id), "{differ:?}");
    assert_eq!(differ.len(), KNOWN_GAPS.len() + 1, "{differ:?}");
}

/// Control: ports that are not `strip_tags` are seen. One that strips nothing, one that loses the `;` an entity is completed with, and
/// one that strips with a counter and no parser each differ from the recording.
#[test]
fn parity_control_a_port_that_is_not_strip_tags_is_a_divergence() {
    let cases = cases();
    let nothing = divergences(|s| s.to_string(), &cases);
    assert!(nothing.len() > 100, "a port that strips nothing differs on only {} cases", nothing.len());

    let no_semicolon = divergences(|s| strip_tags(s).replace(';', ""), &cases);
    assert!(no_semicolon.len() > KNOWN_GAPS.len(), "a port that loses the `;` of an entity is not seen");

    let by_counter = divergences(
        |s| {
            let mut out = String::new();
            let mut depth = 0u32;
            for c in s.chars() {
                match c {
                    '<' => depth += 1,
                    '>' if depth > 0 => depth -= 1,
                    c if depth == 0 => out.push(c),
                    _ => {}
                }
            }
            out
        },
        &cases,
    );
    assert!(by_counter.len() > KNOWN_GAPS.len(), "a port that strips with a counter and no parser is not seen");
}
