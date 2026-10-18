// Probe: fails on the first attempt, passes on any later attempt (marker file in ZZ_MARKER_DIR).
use std::path::PathBuf;

fn first_attempt(name: &str) -> bool {
    let dir = PathBuf::from(std::env::var("ZZ_MARKER_DIR").expect("ZZ_MARKER_DIR must be set"));
    let marker = dir.join(name);
    if marker.exists() {
        false
    } else {
        std::fs::write(&marker, b"x").unwrap();
        true
    }
}

#[test]
fn prop_flaky_probe() {
    assert!(!first_attempt("prop"), "first attempt fails on purpose");
}

#[test]
fn plain_flaky_probe() {
    assert!(!first_attempt("plain"), "first attempt fails on purpose");
}
