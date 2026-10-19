//! HTML sanitisation with the allow-list Plane applies today.
//!
//! Source of truth: `apps/api/plane/utils/content_validator.py` (`validate_html_content`):
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
    (
        "image-component",
        &[
            "id",
            "width",
            "height",
            "aspectRatio",
            "aspectratio",
            "src",
            "alignment",
            "status",
        ],
    ),
    (
        "img",
        &[
            "width",
            "height",
            "aspectRatio",
            "aspectratio",
            "alignment",
            "src",
            "alt",
            "title",
        ],
    ),
    (
        "mention-component",
        &["id", "entity_identifier", "entity_name"],
    ),
    (
        "th",
        &["colspan", "rowspan", "colwidth", "background", "style"],
    ),
    (
        "td",
        &[
            "colspan",
            "rowspan",
            "colwidth",
            "background",
            "textColor",
            "textcolor",
            "style",
        ],
    ),
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
    let map: HashMap<&'static str, HashSet<&'static str>> = PER_TAG
        .iter()
        .map(|(t, a)| (*t, a.iter().copied().collect()))
        .collect();
    b.tag_attributes(map);
    b.url_schemes(SAFE_PROTOCOLS.iter().copied().collect());
    // nh3 leaves strip_comments=true and link_rel="noopener noreferrer" at their defaults, which are ammonia's.
    b
}

fn css_is_dangerous(v: &str) -> bool {
    // strip comments and escapes, drop whitespace, lower-case: `exp/**/ression(`, `j\61vascript:` and friends
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
    let s: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\\')
        .flat_map(|c| c.to_lowercase())
        .collect();
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
    let n: String = v
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect();
    match n.find([':', '/', '?', '#']) {
        Some(i) if n.as_bytes()[i] == b':' => matches!(&n[..i], "http" | "https"),
        _ => true, // relative reference
    }
}

/// `plane_builder()` plus value filters for the places Plane's allow-list lets attacker-controlled URLs and CSS through
/// (`style`, `td/tr background`, `data-emoji-url`). A deliberate hardening, NOT parity with nh3.
pub fn hardened_builder() -> ammonia::Builder<'static> {
    let mut b = plane_builder();
    b.attribute_filter(|_el, attr, value| match attr {
        "style" if css_is_dangerous(value) => None,
        "background" | "data-emoji-url" if !url_ok(value) => None,
        _ => Some(value.into()),
    });
    b
}

#[derive(Debug, PartialEq, Eq)]
pub enum SanitizeError {
    TooLarge,
}

/// `validate_html_content`: `Ok(None)` for empty input (the Python returns `clean_html=None`), `Err` over 10 MB.
pub fn validate_html_content(html: &str) -> Result<Option<String>, SanitizeError> {
    if html.is_empty() {
        return Ok(None);
    }
    if html.len() > MAX_SIZE {
        return Err(SanitizeError::TooLarge);
    }
    Ok(Some(plane_builder().clean(html).to_string()))
}

/// Reusable form (the builder is the expensive part).
pub struct Sanitizer(ammonia::Builder<'static>);
impl Default for Sanitizer {
    fn default() -> Self {
        Sanitizer(plane_builder())
    }
}
impl Sanitizer {
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
    let head = String::from_utf8_lossy(&data[..data.len().min(200)]).to_lowercase();
    for p in [
        "<html",
        "<!doctype",
        "<script",
        "javascript:",
        "data:",
        "<iframe",
    ] {
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
    while i < b.len() {
        match b[i] {
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
                } else if rest.starts_with("<![")
                    || rest.starts_with("<!")
                    || rest.starts_with("<?")
                {
                    match rest.find('>') {
                        Some(e) => i += e + 1,
                        None => {
                            out.push_str(rest);
                            i = b.len();
                        }
                    }
                } else if rest.len() > 1
                    && (rest.as_bytes()[1].is_ascii_alphabetic() || rest.as_bytes()[1] == b'/')
                {
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
                        .take_while(|c| {
                            if hex {
                                c.is_ascii_hexdigit()
                            } else {
                                c.is_ascii_digit()
                            }
                        })
                        .count();
                    if n > 0
                        && body
                            .as_bytes()
                            .get(n)
                            .map(|c| !c.is_ascii_hexdigit())
                            .unwrap_or(false)
                    {
                        out.push_str(&s[i..i + 2 + usize::from(hex) + n]);
                        out.push(';');
                        i += 2 + usize::from(hex) + n;
                        if b.get(i) == Some(&b';') {
                            i += 1;
                        }
                        continue;
                    }
                } else {
                    let n = rest
                        .bytes()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == b'-' || *c == b'.')
                        .count();
                    if n > 0
                        && rest.as_bytes()[0].is_ascii_alphabetic()
                        && rest
                            .as_bytes()
                            .get(n)
                            .map(|c| !c.is_ascii_alphanumeric())
                            .unwrap_or(false)
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
                let ch = s[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

/// index of the `>` that closes the tag starting at `s[0] == '<'`, honouring quoted attribute values
fn tag_end(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 1;
    let mut quote: Option<u8> = None;
    while i < b.len() {
        match (quote, b[i]) {
            (None, b'"') | (None, b'\'') => quote = Some(b[i]),
            (Some(q), c) if c == q => quote = None,
            (None, b'>') => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}
