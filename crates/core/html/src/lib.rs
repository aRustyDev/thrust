//! HTML sanitisation: Plane's allow-list (`plane_builder`), the hardened variant (`hardened_builder`, D-016) and the one Thrust
//! uses (`thrust_builder`, the default: D-016 to D-019).
//!
//! Source of truth for Plane's: `apps/api/plane/utils/content_validator.py` (`validate_html_content`):
//!   `nh3.clean(html, tags=nh3.ALLOWED_TAGS | CUSTOM_TAGS, attributes=ATTRIBUTES, url_schemes={http,https,mailto,tel})`
//! `nh3` 0.2.18 is a binding to the Rust crate `ammonia` 4.0.0, so the same engine (and, for the same version, the
//! same output) is available natively and on wasm32. `ATTRIBUTES` REPLACES ammonia's per-tag defaults (nh3 calls
//! `tag_attributes(...)`), it does not extend them.

use std::collections::{HashMap, HashSet};

/// Plane's `MAX_SIZE` (10 MB) for HTML and binary payloads.
pub const MAX_SIZE: usize = 10 * 1024 * 1024;

const CUSTOM_TAGS: &[&str] = &["mention-component", "label", "input", "image-component"];

const GENERIC: &[&str] = &[
    "class",
    "id",
    "title",
    "role",
    "aria-label",
    "aria-hidden",
    "style",
    "start",
    "type",
    "xmlns",
    "data-tight",
    "data-node-type",
    "data-type",
    "data-checked",
    "data-background-color",
    "data-text-color",
    "data-name",
    "data-id",
    "data-icon-name",
    "data-icon-color",
    "data-background",
    "data-emoji-unicode",
    "data-emoji-url",
    "data-logo-in-use",
    "data-block-type",
];

const PER_TAG: &[(&str, &[&str])] = &[
    ("a", &["href", "target"]),
    ("image-component", &["id", "width", "height", "aspectRatio", "aspectratio", "src", "alignment", "status"]),
    ("img", &["width", "height", "aspectRatio", "aspectratio", "alignment", "src", "alt", "title"]),
    ("mention-component", &["id", "entity_identifier", "entity_name"]),
    ("th", &["colspan", "rowspan", "colwidth", "background", "style"]),
    ("td", &["colspan", "rowspan", "colwidth", "background", "textColor", "textcolor", "style"]),
    ("tr", &["background", "textColor", "textcolor", "style"]),
    ("pre", &["language"]),
    ("code", &["language", "spellcheck"]),
    ("input", &["type", "checked"]),
];

const SAFE_PROTOCOLS: &[&str] = &["http", "https", "mailto", "tel"];

/// An ammonia builder configured exactly like `validate_html_content`.
pub fn plane_builder() -> ammonia::Builder<'static> {
    let mut b = ammonia::Builder::default();
    let mut tags: HashSet<&'static str> = b.clone_tags();
    tags.extend(CUSTOM_TAGS.iter().copied());
    b.tags(tags);
    b.generic_attributes(GENERIC.iter().copied().collect());
    let map: HashMap<&'static str, HashSet<&'static str>> =
        PER_TAG.iter().map(|(t, a)| (*t, a.iter().copied().collect())).collect();
    b.tag_attributes(map);
    b.url_schemes(SAFE_PROTOCOLS.iter().copied().collect());
    // nh3 leaves strip_comments=true and link_rel="noopener noreferrer" at their defaults, which are ammonia's.
    b
}

fn css_is_dangerous(v: &str) -> bool {
    // strip comments, drop backslashes and whitespace, lower-case: `exp/**/ression(`, `java\script:`, `JAVA SCRIPT:` and friends.
    // A CSS HEX escape (`j\61vascript:`, `\6a avascript:`) is NOT decoded, so it passes; measured on 2026-11-18, not fixed here (D-016's
    // claim is the corpus's 125 of 125, and browsers do not run `javascript:` from a CSS `url()` [inferred], the class of the nine).
    let mut s = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(i) = rest.find("/*") {
        s.push_str(&rest[..i]);
        match rest[i + 2..].find("*/") {
            Some(j) => rest = &rest[i + 2 + j + 2..],
            None => {
                rest = "";
                break;
            }
        }
    }
    s.push_str(rest);
    let s: String = s.chars().filter(|c| !c.is_whitespace() && *c != '\\').flat_map(|c| c.to_lowercase()).collect();
    [
        "expression(",
        "javascript:",
        "vbscript:",
        "behavior:",
        "-moz-binding",
        "@import",
        "url(javascript:",
        "url(data:",
        "url('javascript:",
        "url(\"javascript:",
        "url('data:",
        "url(\"data:",
    ]
    .iter()
    .any(|p| s.contains(p))
}

fn url_ok(v: &str) -> bool {
    let n: String =
        v.chars().filter(|c| !c.is_control() && !c.is_whitespace()).flat_map(|c| c.to_lowercase()).collect();
    let Some(i) = n.find([':', '/', '?', '#']) else { return true };
    match (n.get(..i), n.get(i..)) {
        (Some(scheme), Some(rest)) if rest.starts_with(':') => matches!(scheme, "http" | "https"),
        _ => true, // relative reference
    }
}

/// The tokens of a `rel` that the editor writes (`noopener noreferrer nofollow`). A `rel` with any other token is dropped, so a
/// stored `rel="opener"` cannot give a `target="_blank"` link back the `window.opener` that browsers take from it by default.
const REL_TOKENS: &[&str] = &["noopener", "noreferrer", "nofollow"];

fn rel_ok(v: &str) -> bool {
    v.split_ascii_whitespace().all(|t| REL_TOKENS.iter().any(|ok| t.eq_ignore_ascii_case(ok)))
}

/// Whether an attribute VALUE may stay: the one rule set that `hardened_builder` and `thrust_builder` share (ammonia keeps one
/// attribute filter, so the second builder cannot stack its own on the first's). `rel` is only ever asked about by
/// `thrust_builder`: Plane's allow-list has no `rel` for the filter to see.
fn value_ok(attr: &str, value: &str) -> bool {
    match attr {
        "style" => !css_is_dangerous(value),
        "background" | "data-emoji-url" => url_ok(value),
        "rel" => rel_ok(value),
        _ => true,
    }
}

/// `plane_builder()` plus value filters for the places Plane's allow-list lets attacker-controlled URLs and CSS through
/// (`style`, `td/tr background`, `data-emoji-url`). A deliberate hardening, NOT parity with nh3 (D-016).
pub fn hardened_builder() -> ammonia::Builder<'static> {
    let mut b = plane_builder();
    b.attribute_filter(|_el, attr, value| value_ok(attr, value).then(|| value.into()));
    b
}

/// The work-item embed the editor writes (`issue-embed-component`, D-017). Its `id` is already a generic attribute.
const EMBED_TAG: &str = "issue-embed-component";
const EMBED_ATTRS: &[&str] = &["entity_identifier", "project_identifier", "workspace_identifier", "entity_name"];

/// What Thrust keeps of a description: `hardened_builder()` plus what Plane's allow-list would strip from the editor's own output,
/// so that none of the editor-generated documents changes (195 of 195, `crates/editor/doc/tests/sanitize.rs`):
/// - `issue-embed-component` and its attributes (D-017);
/// - `a rel` as the editor writes it (D-018), which means turning off ammonia's own `link_rel` (it refuses to run with both).
///
/// This is the default of `Sanitizer` and `validate_html_content`.
pub fn thrust_builder() -> ammonia::Builder<'static> {
    let mut b = hardened_builder();
    b.add_tags([EMBED_TAG]);
    b.add_tag_attributes(EMBED_TAG, EMBED_ATTRS.iter().copied());
    b.add_tag_attributes("a", ["rel"]);
    b.link_rel(None);
    b
}

/// The version of what `description_html` was cleaned with: this allow-list and the editor schema it must not damage (the pin
/// `oracle/v1-pin-20261018`). A change to either bumps it, so a stored description says which policy it was made under (ADR-0012).
pub const SCHEMA_VERSION: i16 = 1;

#[derive(Debug, PartialEq, Eq)]
pub enum SanitizeError {
    TooLarge,
}

/// The default policy, built once: building a builder is the expensive part, and this runs on every description write.
static DEFAULT: std::sync::LazyLock<ammonia::Builder<'static>> = std::sync::LazyLock::new(thrust_builder);

/// Plane's size check, which `validate_html_content` starts with: `Err` over 10 MB. The request layers use it to refuse an oversize
/// description in the pin's words, while they read the rest of the body, before the use-case cleans it.
pub fn check_size(html: &str) -> Result<(), SanitizeError> {
    if html.len() > MAX_SIZE {
        return Err(SanitizeError::TooLarge);
    }
    Ok(())
}

/// Plane's `validate_html_content` with Thrust's allow-list (`thrust_builder`, not `plane_builder`): `Ok(None)` for empty input (the
/// Python returns `clean_html=None`), `Err` over 10 MB.
pub fn validate_html_content(html: &str) -> Result<Option<String>, SanitizeError> {
    if html.is_empty() {
        return Ok(None);
    }
    check_size(html)?;
    Ok(Some(DEFAULT.clean(html).to_string()))
}

/// What a work item keeps of a description it was sent (issue #48, D-020): the one form that P0 stores, and what is derived from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Description {
    /// The cleaned HTML; the empty text stays empty, as the pin keeps it.
    pub html: String,
    /// `strip_tags` of `html`, the text a search or a preview reads (`description_stripped`).
    pub stripped: String,
    /// [`SCHEMA_VERSION`], the policy `html` was cleaned under.
    pub schema: i16,
}

/// Clean a description as it was sent, and derive what is stored with it. The one place that does: the use-cases call it, so the
/// compat plane and `/api/v1` share it. `Err` over 10 MB (the request layers have checked that already, with [`check_size`]).
pub fn describe(sent: &str) -> Result<Description, SanitizeError> {
    // `Ok(None)` is only an empty `sent`, which is kept empty
    let html = validate_html_content(sent)?.unwrap_or_default();
    let stripped = strip_tags(&html);
    Ok(Description { html, stripped, schema: SCHEMA_VERSION })
}

/// Reusable form (the builder is the expensive part). `default()` is Thrust's allow-list; a test that compares with nh3 asks for
/// `Sanitizer::new(plane_builder())`.
pub struct Sanitizer(ammonia::Builder<'static>);
impl Default for Sanitizer {
    fn default() -> Self {
        Sanitizer(thrust_builder())
    }
}
impl Sanitizer {
    pub fn new(builder: ammonia::Builder<'static>) -> Self {
        Sanitizer(builder)
    }

    pub fn clean(&self, html: &str) -> String {
        self.0.clean(html).to_string()
    }
}

/// Plane's `validate_binary_data` (size, minimum length, "suspicious" text patterns in the first 200 bytes).
pub fn validate_binary_data(data: &[u8]) -> Result<(), &'static str> {
    if data.is_empty() {
        return Ok(());
    }
    if data.len() > MAX_SIZE {
        return Err("Binary data exceeds maximum size limit (10MB)");
    }
    if data.len() < 4 {
        return Err("Binary data too short to be valid document format");
    }
    // the first 200 bytes, or all of it when it is shorter
    let head = String::from_utf8_lossy(data.get(..200).unwrap_or(data)).to_lowercase();
    for p in ["<html", "<!doctype", "<script", "javascript:", "data:", "<iframe"] {
        if head.contains(p) {
            return Err("Binary data contains suspicious content patterns");
        }
    }
    Ok(())
}

/// `django.utils.html.strip_tags` (used for `description_stripped`): a close port of the `HTMLParser`-based
/// `MLStripper` loop. Entity and character references are kept as text, `&name` is completed to `&name;`.
pub fn strip_tags(value: &str) -> String {
    let mut value = value.to_string();
    while value.contains('<') && value.contains('>') {
        let new_value = strip_once(&value);
        if value.matches('<').count() == new_value.matches('<').count() {
            break;
        }
        value = new_value;
    }
    value
}

fn strip_once(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(&byte) = b.get(i) {
        match byte {
            b'<' => {
                let rest = &s[i..];
                if rest.starts_with("<!--") {
                    match rest.find("-->") {
                        Some(e) => i += e + 3,
                        None => {
                            // unterminated comment: HTMLParser.close() flushes the rest as data
                            out.push_str(rest);
                            i = b.len();
                        }
                    }
                } else if rest.starts_with("<![") || rest.starts_with("<!") || rest.starts_with("<?") {
                    match rest.find('>') {
                        Some(e) => i += e + 1,
                        None => {
                            out.push_str(rest);
                            i = b.len();
                        }
                    }
                } else if rest.as_bytes().get(1).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'/') {
                    match tag_end(rest) {
                        Some(e) => i += e + 1,
                        None => {
                            out.push_str(rest);
                            i = b.len();
                        }
                    }
                } else {
                    out.push('<');
                    i += 1;
                }
            }
            b'&' => {
                let rest = &s[i + 1..];
                if let Some(r) = rest.strip_prefix('#') {
                    let (hex, body) = match r.strip_prefix(['x', 'X']) {
                        Some(h) => (true, h),
                        None => (false, r),
                    };
                    let n = body
                        .bytes()
                        .take_while(|c| if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() })
                        .count();
                    if n > 0 && body.as_bytes().get(n).map(|c| !c.is_ascii_hexdigit()).unwrap_or(false) {
                        out.push_str(&s[i..i + 2 + usize::from(hex) + n]);
                        out.push(';');
                        i += 2 + usize::from(hex) + n;
                        if b.get(i) == Some(&b';') {
                            i += 1;
                        }
                        continue;
                    }
                } else {
                    let n = rest.bytes().take_while(|c| c.is_ascii_alphanumeric() || *c == b'-' || *c == b'.').count();
                    // (a first byte that is alphabetic is counted by `take_while`, so `n >= 1` here: no `n > 0` of its own to keep)
                    if rest.as_bytes().first().is_some_and(|c| c.is_ascii_alphabetic())
                        && rest.as_bytes().get(n).map(|c| !c.is_ascii_alphanumeric()).unwrap_or(false)
                    {
                        out.push('&');
                        out.push_str(&rest[..n]);
                        out.push(';');
                        i += 1 + n;
                        if b.get(i) == Some(&b';') {
                            i += 1;
                        }
                        continue;
                    }
                }
                out.push('&');
                i += 1;
            }
            _ => {
                // `i` is on a character boundary and inside `s`, so there is always a character here
                let Some(ch) = s.get(i..).and_then(|t| t.chars().next()) else { break };
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

/// index of the `>` that closes the tag starting at `s[0] == '<'`, honouring quoted attribute values
fn tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (i, byte) in s.bytes().enumerate().skip(1) {
        match (quote, byte) {
            (None, b'"') | (None, b'\'') => quote = Some(byte),
            (Some(q), c) if c == q => quote = None,
            (None, b'>') => return Some(i),
            _ => {}
        }
    }
    None
}
