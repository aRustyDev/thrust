use serde_json::Value;
use thrust_editor_doc::vdom::unescape_html;

/// Differential fuzz of entity decoding against zeed-dom's `unescapeHTML` (target/spike-0006/entities-fuzz.json).
#[test]
fn entities_match_the_oracle() {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/spike-0006/entities-fuzz.json"
    );
    let Ok(s) = std::fs::read_to_string(p) else {
        eprintln!("no entity fuzz corpus (run tools/spike-0006/gen-all.sh)");
        return;
    };
    let c: Value = serde_json::from_str(&s).unwrap();
    let (mut ok, mut n) = (0, 0);
    let mut fails = vec![];
    for r in c["rows"].as_array().unwrap() {
        let input = r["input"].as_str().unwrap();
        let want = r["expected"].as_str().unwrap();
        let got = unescape_html(input);
        n += 1;
        if got == want {
            ok += 1;
        } else if fails.len() < 15 {
            fails.push(format!("{input:?} -> got {got:?} want {want:?}"));
        }
    }
    eprintln!("entity decoding vs entities 5.0.0: {ok}/{n}");
    for f in &fails {
        eprintln!("  FAIL {f}");
    }
    assert_eq!(ok, n);
}
