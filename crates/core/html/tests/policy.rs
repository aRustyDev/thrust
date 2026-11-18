//! What the default policy (`thrust_builder`) keeps and drops, against Plane's own (`plane_builder`) as the control (issue #48).
// clippy.toml exempts a `#[test]` function from the four panic lints, not the helpers of a test file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "test code")]
use proptest::prelude::*;
use serde_json::Value;
use thrust_html::{hardened_builder, plane_builder, thrust_builder, validate_html_content, Sanitizer};

/// The nine payloads of the spike-0006 XSS corpus that survive Plane's allow-list (`research/0008` §7: six `style` values, `td background`,
/// `td style` with `url(javascript:)`, the callout's `data-emoji-url`). They are read from the committed corpus by id, so the test
/// has the corpus's own markup, and a corpus that lost one of them fails here instead of passing with eight.
const SURVIVORS: [&str; 9] = [
    "owasp-css-110",
    "owasp-css-111",
    "owasp-css-112",
    "owasp-css-113",
    "owasp-css-114",
    "owasp-css-115",
    "owasp-tag-090",
    "plane-table-147",
    "plane-callout-144",
];

/// What makes one of the nine executable. A case-insensitive substring is enough for these nine: each carries its token in a
/// `style`, `background` or `data-emoji-url` value, and the sanitiser drops the whole attribute or keeps all of it.
const EXECUTABLE: [&str; 4] = ["javascript:", "expression(", "behavior:", "-moz-binding"];

fn survivors() -> Vec<(String, String)> {
    let path = concat!(env!("THRUST_ROOT"), "/crates/editor/doc/tests/corpus/xss-corpus.json");
    let corpus: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let found: Vec<(String, String)> = SURVIVORS
        .iter()
        .map(|id| {
            let entry = corpus
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == *id)
                .unwrap_or_else(|| panic!("{path} has no entry {id}"));
            assert_eq!(entry["malicious"], true, "{id} is no longer marked executable");
            (id.to_string(), entry["html"].as_str().unwrap().to_string())
        })
        .collect();
    assert_eq!(found.len(), 9);
    found
}

fn neutralised(out: &str) -> bool {
    let out = out.to_lowercase();
    EXECUTABLE.iter().all(|t| !out.contains(t)) && out.contains(">x<")
}

/// Rule 9 (b): the nine are named by `research/0008` §7. The default neutralises every one, and Plane's list is the control: it
/// leaves every one executable, so a check that stopped looking, or a default that went back to `plane_builder()`, fails here.
#[test]
fn the_nine_payloads_plane_lets_through_are_neutralised_by_the_default_and_not_by_plane() {
    let default = Sanitizer::default();
    let plane = Sanitizer::new(plane_builder());
    for (id, html) in survivors() {
        let theirs = plane.clean(&html);
        assert!(!neutralised(&theirs), "{id}: the control is not executable under Plane's list: {theirs}");
        let ours = default.clean(&html);
        assert!(neutralised(&ours), "{id}: still executable: {ours}");
        let kept = validate_html_content(&html).unwrap().unwrap();
        assert_eq!(kept, ours, "{id}: validate_html_content is not the default policy");
    }
}

/// The hardened builder is the part that closes the nine; the Thrust one only adds to it.
#[test]
fn the_hardened_builder_closes_the_nine_too() {
    for (id, html) in survivors() {
        let out = hardened_builder().clean(&html).to_string();
        assert!(neutralised(&out), "{id}: {out}");
    }
}

// ---- generated: the value rules --------------------------------------------------------------------------------------------

const CSS_TOKENS: [&str; 8] =
    ["expression(", "javascript:", "vbscript:", "behavior:", "-moz-binding", "@import", "url(javascript:", "url(data:"];
const SCHEMES: [&str; 3] = ["javascript:", "vbscript:", "data:"];

/// `token` with some letters upper-cased and one separator put inside it: the forms `css_is_dangerous` and `url_ok` promise to see
/// through (case, a CSS comment, whitespace).
fn obfuscated(tokens: &'static [&'static str], seps: &'static [&'static str]) -> impl Strategy<Value = String> {
    (prop::sample::select(tokens), prop::collection::vec(any::<bool>(), 24), 0usize..24, prop::sample::select(seps))
        .prop_map(|(token, upper, at, sep)| {
            let chars: Vec<char> = token
                .chars()
                .zip(upper.iter().cycle())
                .map(|(c, up)| if *up { c.to_ascii_uppercase() } else { c })
                .collect();
            let at = at % (chars.len() + 1);
            let (head, tail) = chars.split_at(at);
            format!("{}{}{}", head.iter().collect::<String>(), sep, tail.iter().collect::<String>())
        })
}

fn clean(html: &str) -> String {
    validate_html_content(html).unwrap().unwrap()
}

proptest! {
    /// A `style` that carries a dangerous token, however it is cased or broken up, loses the whole attribute.
    #[test]
    fn prop_a_dangerous_style_value_is_dropped(v in obfuscated(&CSS_TOKENS, &["", "/**/", "/*x*/", " ", "\t"])) {
        let out = clean(&format!("<p style=\"color:red;{v}\">x</p>"));
        prop_assert!(!out.contains("style"), "{}", out);
        prop_assert!(out.contains(">x<"), "{}", out);
    }

    /// A `td background` or a callout's `data-emoji-url` whose scheme is not http or https loses the attribute, however the scheme
    /// is cased or broken up by whitespace.
    #[test]
    fn prop_a_url_with_a_dangerous_scheme_is_dropped(v in obfuscated(&SCHEMES, &["", " ", "\t", "\n"])) {
        let cell = clean(&format!("<table><tr><td background=\"{v}alert(1)\">x</td></tr></table>"));
        prop_assert!(!cell.contains("background"), "{}", cell);
        let callout = clean(&format!("<div data-block-type=\"callout-component\" data-emoji-url=\"{v}alert(1)\"><p>x</p></div>"));
        prop_assert!(!callout.contains("data-emoji-url"), "{}", callout);
        prop_assert!(callout.contains(">x<"), "{}", callout);
    }

    /// A URL is judged by how it reads with whitespace and control characters taken out, as a browser reads one (`url_ok`): an
    /// `https` with a space, a tab or a control character inside it is still `https`, and is kept. (Found by mutation testing: the
    /// dangerous schemes are dropped either way, so only a safe one shows whether the characters are removed.)
    #[test]
    fn prop_whitespace_or_a_control_character_inside_an_http_scheme_does_not_change_it(
        v in obfuscated(&["https:", "http:"], &[" ", "\t", "\n", "\u{1}"])
    ) {
        let cell = clean(&format!("<table><tr><td background=\"{v}//example.com/x\">x</td></tr></table>"));
        prop_assert!(cell.contains("background="), "{:?} -> {}", v, cell);
    }

    /// The control of the two above: a harmless value is kept as written, so they do not pass by dropping everything.
    #[test]
    fn prop_a_harmless_value_is_kept(width in 1u32..2000, path in "[a-z0-9/_-]{1,24}") {
        let style = clean(&format!("<p style=\"color:red;width:{width}px\">x</p>"));
        prop_assert!(style.contains(&format!("style=\"color:red;width:{width}px\"")), "{}", style);
        let url = format!("https://example.com/{path}");
        let cell = clean(&format!("<table><tr><td background=\"{url}\">x</td></tr></table>"));
        prop_assert!(cell.contains(&format!("background=\"{url}\"")), "{}", cell);
        let relative = clean(&format!("<table><tr><td background=\"/{path}\">x</td></tr></table>"));
        prop_assert!(relative.contains(&format!("background=\"/{path}\"")), "{}", relative);
    }

    /// `rel` stays when every token is one the editor writes, in any case and order, and goes when one is not (an `opener` would give
    /// a `target="_blank"` link back the `window.opener` that browsers take from it).
    #[test]
    fn prop_rel_stays_only_when_every_token_is_one_the_editor_writes(
        tokens in prop::collection::vec(prop::sample::select(&["noopener", "noreferrer", "nofollow", "NoFollow", "opener", "external", "me"]), 1..5)
    ) {
        let rel = tokens.join(" ");
        let kept = tokens.iter().all(|t| ["noopener", "noreferrer", "nofollow"].contains(&t.to_lowercase().as_str()));
        let out = clean(&format!("<a href=\"https://example.com\" target=\"_blank\" rel=\"{rel}\">x</a>"));
        prop_assert_eq!(out.contains("rel="), kept, "{}", out);
        if kept {
            prop_assert!(out.contains(&format!("rel=\"{rel}\"")), "{}", out);
        }
    }
}

// ---- the editor's own markup is kept (D-017, D-018) ------------------------------------------------------------------------

/// Rule 9 (b): `research/0008` §5 rows 7 and 8 name both. What the editor writes for a work-item embed and for a link, which Plane's
/// list rewrites or strips and the default must not.
#[test]
fn the_editors_embed_and_link_markup_survives_the_default_and_not_plane() {
    let embed = "<issue-embed-component entity_identifier=\"7\" project_identifier=\"PLATF\" workspace_identifier=\"acme\" id=\"e1\" entity_name=\"issue\"></issue-embed-component>";
    let link = "<a href=\"https://example.com\" target=\"_blank\" rel=\"noopener noreferrer nofollow\">l</a>";
    let ours = clean(&format!("{embed}{link}"));
    for attr in [
        "entity_identifier=\"7\"",
        "project_identifier=\"PLATF\"",
        "workspace_identifier=\"acme\"",
        "id=\"e1\"",
        "entity_name=\"issue\"",
        "rel=\"noopener noreferrer nofollow\"",
    ] {
        assert!(ours.contains(attr), "{attr} lost: {ours}");
    }
    assert!(ours.contains("<issue-embed-component"), "{ours}");

    let theirs = Sanitizer::new(plane_builder()).clean(&format!("{embed}{link}"));
    assert!(!theirs.contains("issue-embed-component"), "control: Plane's list strips the embed: {theirs}");
    assert!(!theirs.contains("nofollow"), "control: Plane's list rewrites rel to `noopener noreferrer`: {theirs}");
}

/// An attribute that is not the embed's own is still dropped from it.
#[test]
fn the_embed_takes_no_attribute_of_its_own_choosing() {
    let out = clean("<issue-embed-component onclick=\"alert(1)\" style=\"x:expression(1)\" entity_name=\"issue\"></issue-embed-component>");
    assert!(out.contains("entity_name=\"issue\""), "{out}");
    assert!(!out.contains("onclick") && !out.contains("style"), "{out}");
}

/// The builders can be made twice and clean the same: nothing in `thrust_builder` is built once and shared by accident.
#[test]
fn two_builders_clean_alike() {
    let html = "<p style=\"color:red\"><a href=\"https://example.com\" rel=\"nofollow\">x</a></p>";
    assert_eq!(thrust_builder().clean(html).to_string(), thrust_builder().clean(html).to_string());
}
