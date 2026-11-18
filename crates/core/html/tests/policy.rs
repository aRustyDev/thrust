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
    fn prop_a_dangerous_style_value_is_dropped(v in obfuscated(&CSS_TOKENS, &["", "/**/", "/*x*/", " ", "\t", "\u{1}"])) {
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

// ---- CSS escapes: `j\61vascript:` is `javascript:` to a stylesheet ---------------------------------------------------------

/// How one character of a token is written in a CSS value (CSS Syntax 3, §4.3.7, "consume an escaped code point"). The encoder is
/// written here from the spec, apart from the decoder under test, so a decoder that reads an escape differently fails.
#[derive(Clone, Copy, Debug)]
enum Spell {
    /// As it is.
    Plain,
    /// `\` and the character: the character itself, unless it is a hex digit (`\a` is U+000A), which then stays plain.
    Backslash,
    /// `\` and the code point in hex, as short as it goes. The whitespace `end` closes it; `bare` leaves it unclosed when the next
    /// character does not continue the hex number (`\61vascript` is `a` and `vascript`).
    Short { upper: bool, end: &'static str, bare: bool },
    /// `\` and exactly six hex digits, which nothing needs to close: a seventh digit is not part of it (`\00006aavascript`).
    Padded { upper: bool },
}

fn spell() -> impl Strategy<Value = Spell> {
    prop_oneof![
        3 => Just(Spell::Plain),
        2 => Just(Spell::Backslash),
        3 => (any::<bool>(), prop::sample::select(&[" ", "\t", "\n", "\r\n", "\u{c}"]), any::<bool>())
            .prop_map(|(upper, end, bare)| Spell::Short { upper, end, bare }),
        3 => any::<bool>().prop_map(|upper| Spell::Padded { upper }),
    ]
}

/// `token` with each character written as its `Spell` says, and a letter or two in upper case.
fn spelled(token: &str, spells: &[Spell], upper: &[bool]) -> String {
    let mut out = String::new();
    let mut open = false; // the last part is a `bare` short escape: a hex digit next would continue its number
    for (i, c) in token.chars().enumerate() {
        let c = if upper.get(i).copied().unwrap_or(false) { c.to_ascii_uppercase() } else { c };
        let code = c as u32;
        let (part, bare) = match spells.get(i).copied().unwrap_or(Spell::Plain) {
            Spell::Backslash if !c.is_ascii_hexdigit() => (format!("\\{c}"), false),
            Spell::Short { upper, end, bare } => {
                let hex = if upper { format!("{code:X}") } else { format!("{code:x}") };
                (if bare { format!("\\{hex}") } else { format!("\\{hex}{end}") }, bare)
            }
            Spell::Padded { upper } => (if upper { format!("\\{code:06X}") } else { format!("\\{code:06x}") }, false),
            Spell::Plain | Spell::Backslash => (c.to_string(), false),
        };
        if open && part.starts_with(|n: char| n.is_ascii_hexdigit()) {
            out.push(' '); // the one thing that ends it
        }
        out.push_str(&part);
        open = bare;
    }
    out
}

proptest! {
    /// A dangerous token, with each of its characters written plain, with a backslash, as a short or a padded hex escape, in either
    /// case, closed by any whitespace or by nothing, still takes the whole `style` away: what a stylesheet reads is what is judged.
    #[test]
    fn prop_a_dangerous_css_token_is_seen_through_any_escape_of_its_characters(
        token in prop::sample::select(&CSS_TOKENS),
        spells in prop::collection::vec(spell(), 16),
        upper in prop::collection::vec(any::<bool>(), 16),
    ) {
        let v = spelled(token, &spells, &upper);
        let out = clean(&format!("<p style=\"color:red;{v};margin:0\">x</p>"));
        prop_assert!(!out.contains("style"), "{:?} -> {}", v, out);
        prop_assert!(out.contains(">x<"), "{}", out);
    }

    /// A token inside a comment, spelled any way, is part of the comment: the value is kept. What a stylesheet does not read is not
    /// judged, which also shows that the property above is not passing by dropping every value that holds a token.
    #[test]
    fn prop_a_token_inside_a_comment_is_not_judged(
        token in prop::sample::select(&CSS_TOKENS),
        spells in prop::collection::vec(spell(), 16),
        upper in prop::collection::vec(any::<bool>(), 16),
    ) {
        let v = spelled(token, &spells, &upper);
        let out = clean(&format!("<p style=\"color:red;/* {v} */margin:0\">x</p>"));
        prop_assert!(out.contains("style=\"color:red;/* "), "{:?} -> {}", v, out);
    }

    /// And a comment that an escaped slash did not open hides nothing: the token after `\/*` is read, spelled any way.
    #[test]
    fn prop_an_escaped_slash_opens_no_comment(
        token in prop::sample::select(&CSS_TOKENS),
        spells in prop::collection::vec(spell(), 16),
        upper in prop::collection::vec(any::<bool>(), 16),
    ) {
        let v = spelled(token, &spells, &upper);
        let out = clean(&format!("<p style=\"color:red;\\/* {v} */margin:0\">x</p>"));
        prop_assert!(!out.contains("style"), "{:?} -> {}", v, out);
    }

    /// The control of the one above: escapes of characters that spell nothing dangerous are kept as written (`\5B57\4F53` is how a
    /// stylesheet writes 字体), so it does not pass by dropping every value that has a backslash in it.
    #[test]
    fn prop_an_escaped_harmless_value_is_kept(
        cps in prop::collection::vec(0x4e00u32..0x9fff, 1..6),
        upper in any::<bool>(),
    ) {
        let names: Vec<String> = cps.iter().map(|c| if upper { format!("\\{c:X}") } else { format!("\\{c:x}") }).collect();
        let style = format!("font-family:{}", names.join(" "));
        let out = clean(&format!("<p style=\"{style}\">x</p>"));
        prop_assert!(out.contains(&format!("style=\"{style}\"")), "{}", out);
    }
}

/// Pins the gap that PR #305's report named on 2026-11-18: `css_is_dangerous` took the backslashes out and did not read a hex
/// escape, so `j\61vascript:` and the like passed the value filter. Each value is `javascript:`, `expression(`, `url(data:`,
/// `vbscript:`, `behavior:`, `-moz-binding` or `@import` to a stylesheet, however its characters are written.
#[test]
fn regression_a_css_hex_escape_cannot_spell_a_dangerous_token() {
    for v in [
        r"background:url(j\61vascript:alert(1))", // `\61` ends at `v`, which is no hex digit
        r"background:url(\6a avascript:alert(1))", // the space ends the escape, and is not part of the value
        r"background:url(\6A\61 \76 ascript:alert(1))",
        r"background:url(\00006a avascript:alert(1))", // six digits, leading zeros
        r"background:url(\00006aavascript:alert(1))",  // six digits end it: the seventh is the `a` of `avascript`
        r"width:\65xpression(alert(1))",
        r"width:\65 xpression(alert(1))",
        r"background:\75rl(\64 ata:text/html,x)",
        r"x:\76 bscript:1", // a space must end `\76`: `\76b` would be U+076B
        r"x:-moz-\62 inding:url(x)",
        r"x:\62 ehavior:url(x)",
        r"\40import 'x'",
        r"\@import 'x'", // a backslash before a character that is no hex digit is that character
        r"x:j\000061 vascript:1",
        // `\/` is an escaped slash, so `\/*` opens no comment and what follows it is read (the comment was cut out before the escape was)
        r"x:\/* javascript:alert(1) */",
        r"x:\/* \6a avascript:1 */",
        r"x:/* a */javascript:1",   // a comment that is closed hides only what it holds
        r"x:/*/ a */ javascript:1", // `/*/` opens one and the `/` does not close it: `*/` after does
        r"x:java\\script:1",        // `\\` is a backslash, taken out as ever: it can only make a token easier to see
        // a code point that no `char` can be is dropped, so it cannot be the break that hides a token (old browsers ignored a NUL)
        r"x:java\0 script:1",
        r"x:java\d800 script:1",
        r"x:java\110000 script:1",
        "x:java\u{1}script:1", // a control character, not an escape: removed as `url_ok` does
    ] {
        let out = clean(&format!("<p style=\"{v}\">x</p>"));
        assert!(!out.contains("style"), "{v:?} was kept: {out}");
        assert!(out.contains(">x<"), "{v:?}: {out}");
    }
}

/// What the test above is not allowed to pass by: a hex number is read greedily to six digits, so these spell something that is not
/// a dangerous token, and a comment holds what it holds without it being read. All are kept as written.
#[test]
fn a_css_escape_that_spells_something_else_is_kept() {
    for v in [
        r"x:\76bscript:1",     // `\76b` is U+076B
        r"x:\6a1avascript:1",  // `\6a1a` is U+6A1A
        r"x:\6a 61vascript:1", // `j`, then `61vascript`
        r"content:'\201C'",
        r"font-family:\5B57\4F53",
        r"x:\@mport 1", // `\@` is `@`, and `@mport` is no `@import`
        // a comment is a comment: what it holds is not read, closed or not (CSS Syntax 3, §4.3.2)
        r"color:red /* javascript:alert(1) */",
        r"color:red /* expression(1)",
        r"color:red /*/ javascript:1",
    ] {
        let out = clean(&format!("<p style=\"{v}\">x</p>"));
        assert!(out.contains(&format!("style=\"{v}\"")), "{v:?} was dropped: {out}");
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
