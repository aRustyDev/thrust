//! Where a request goes (ADR-0111), and what of it may go there.
//!
//! [`decide`] is the whole routing rule. No valid pin means production, without asking Flagship, so an ordinary request never
//! depends on it. With a pin, three answers from Flagship are needed and any error is production: the global switch `routing`
//! must be on; `alias-<requested>`, when it names something, replaces the requested name (an alias naming `prod` is production);
//! `target-<name>`, evaluated with the principal's kind and audience, must return a target whose URL is a Preview of the Worker.
//!
//! [`scrub`] removes from a request what a client must never be able to hand to production or to a target: Access's headers and
//! cookie (the edge vouches for nobody by forwarding them), Cloudflare's version-override headers (they would pick a Worker version
//! past the rules), forwarding headers the edge sets itself, and the pin cookie, which only the edge reads.

use serde::Serialize;

use crate::pin::{valid_name, Pin, COOKIE_NAME};

/// The global switch for every non-production target.
pub const ROUTING_FLAG: &str = "routing";
/// The name that means production when an alias resolves to it.
pub const PRODUCTION: &str = "prod";

/// Flagship keys allow letters, digits, `-` and `_`, not `.`; names allow `.` and never `_`, so the mapping is one to one.
fn key(prefix: &str, name: &str) -> String {
    format!("{prefix}-{}", name.replace('.', "_"))
}

/// The key of the alias flag for `name` (`alias-staging`).
pub fn alias_key(name: &str) -> String {
    key("alias", name)
}

/// The key of the target flag for `name` (`target-v0_3_0`).
pub fn target_key(name: &str) -> String {
    key("target", name)
}

/// The context a target flag is evaluated with: its rules name kinds and audiences, never people.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EvalContext {
    pub kind: &'static str,
    pub audience: String,
}

impl EvalContext {
    pub fn of(pin: &Pin) -> Self {
        EvalContext { kind: pin.kind.as_str(), audience: pin.audience.clone() }
    }
}

/// An answer Flagship could not give. Every caller treats it as production.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlagError;

/// The three Flagship evaluations the rule needs. The runtime implements it on the Flagship binding; tests substitute answers.
#[allow(async_fn_in_trait, reason = "the edge is single-threaded wasm: no caller needs the futures to be Send")]
pub trait Flags {
    /// `routing`, default off.
    async fn routing(&self) -> Result<bool, FlagError>;
    /// The string an alias flag holds, or empty when there is no such flag.
    async fn alias(&self, key: &str) -> Result<String, FlagError>;
    /// The JSON a target flag returns for `ctx`; its default (denied, or no such flag) is an empty object.
    async fn target(&self, key: &str, ctx: &EvalContext) -> Result<serde_json::Value, FlagError>;
}

/// A target the request is allowed to reach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    /// `https://<preview host>`, nothing after the host.
    pub url: String,
    /// The Preview deployment id the target's flag names (what the target smoke compares, research/0016).
    pub deployment: String,
}

impl Target {
    /// The Preview's host: the URL without `https://`.
    pub fn host(&self) -> &str {
        self.url.strip_prefix("https://").unwrap_or(&self.url)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Production,
    Target(Target),
}

/// The routing rule; see the module documentation.
pub async fn decide<F: Flags>(pin: Option<&Pin>, flags: &F, preview_suffix: &str) -> Decision {
    let Some(pin) = pin else { return Decision::Production };
    if flags.routing().await != Ok(true) {
        return Decision::Production;
    }
    let name = match flags.alias(&alias_key(&pin.requested)).await {
        Ok(alias) if alias.is_empty() => pin.requested.clone(),
        Ok(alias) if valid_name(&alias) => alias,
        _ => return Decision::Production,
    };
    if name == PRODUCTION {
        return Decision::Production;
    }
    match flags.target(&target_key(&name), &EvalContext::of(pin)).await {
        Ok(value) => parse_target(&name, &value, preview_suffix).map_or(Decision::Production, Decision::Target),
        Err(FlagError) => Decision::Production,
    }
}

/// A DNS label: 1 to 63 lowercase letters, digits or `-`, not starting or ending with `-`.
fn dns_label(s: &str) -> bool {
    (1..=63).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// The target a flag's JSON describes, if it is one: `url` must be `https://<label><preview_suffix>` and nothing more, so a flag
/// can only send traffic to a Preview of the Worker, never to another host; `deployment` must be a plain id. `name` is the
/// target's name as [`decide`] resolved it, already in the grammar of [`valid_name`]; it is carried, not checked, here.
pub fn parse_target(name: &str, value: &serde_json::Value, preview_suffix: &str) -> Option<Target> {
    let url = value.get("url")?.as_str()?;
    let deployment = value.get("deployment")?.as_str()?;
    let host = url.strip_prefix("https://")?;
    let label = host.strip_suffix(preview_suffix)?;
    if preview_suffix.is_empty() || !preview_suffix.starts_with('-') || !dns_label(label) {
        return None;
    }
    let suffix_ok =
        preview_suffix.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.');
    let deployment_ok = (1..=64).contains(&deployment.len())
        && deployment.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !suffix_ok || !deployment_ok {
        return None;
    }
    Some(Target { name: name.to_string(), url: url.to_string(), deployment: deployment.to_string() })
}

/// Is this request header one a client may pass on? (Names compare without regard to case.)
pub fn keep_header(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    !(n.starts_with("cf-access-")
        || n.starts_with("cloudflare-workers-version-")
        || n == "x-forwarded-host"
        || n == "x-forwarded-proto")
}

/// A `Cookie` header's value without Access's cookie (`CF_Authorization`, any case) and without the pin; `None` when nothing is left.
pub fn strip_cookies(value: &str) -> Option<String> {
    let kept: Vec<&str> = value
        .split(';')
        .map(str::trim)
        .filter(|pair| {
            let name = pair.split_once('=').map_or(*pair, |(n, _)| n).trim();
            !pair.is_empty() && !name.eq_ignore_ascii_case("CF_Authorization") && name != COOKIE_NAME
        })
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(kept.join("; "))
    }
}

/// The request's headers as they may go on: see [`keep_header`] and [`strip_cookies`].
pub fn scrub(headers: &[(String, String)]) -> Vec<(String, String)> {
    let mut out = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        if !keep_header(name) {
            continue;
        }
        if name.eq_ignore_ascii_case("cookie") {
            if let Some(v) = strip_cookies(value) {
                out.push((name.clone(), v));
            }
            continue;
        }
        out.push((name.clone(), value.clone()));
    }
    out
}
