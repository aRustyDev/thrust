//! Fetching a target and handing its answer back (ADR-0111, research/0016).
//!
//! The upstream URL is the target's base, which came from its flag and was checked by [`crate::route::parse_target`], followed
//! by the request's own path and query: the host is never the client's. The upstream request says where it came from
//! (`X-Forwarded-Host`, `X-Forwarded-Proto`), and drops the client's `Host`, which `fetch` sets from the URL.
//!
//! On the way back, the response loses every `Set-Cookie` for `CF_Authorization`: Access sets it, with no `Domain`, on the
//! Preview's answer to the edge, and passed on it would be stored for the shared hostname. A `Location` that names the Preview
//! host is rewritten to the shared origin (the edge fetches with `redirect: manual`, so the browser sees the redirect).
//! `x-thrust-route` says which way the request went, replacing any the upstream sent.

/// The response header that says where a request went: `prod`, or the target's name.
pub const ROUTE_HEADER: &str = "x-thrust-route";

/// `base` (`https://<host>`, nothing after it) followed by `path_and_query`, which must start with `/`.
pub fn upstream_url(base: &str, path_and_query: &str) -> Option<String> {
    if !base.starts_with("https://") || base.ends_with('/') || !path_and_query.starts_with('/') {
        return None;
    }
    Some(format!("{base}{path_and_query}"))
}

/// The headers to send a target: the scrubbed request headers (see [`crate::route::scrub`]) without `Host`, with the shared
/// host and scheme in their place.
pub fn upstream_headers(scrubbed: &[(String, String)], shared_host: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = scrubbed
        .iter()
        .filter(|(n, _)| {
            let n = n.to_ascii_lowercase();
            n != "host" && n != "x-forwarded-host" && n != "x-forwarded-proto"
        })
        .cloned()
        .collect();
    out.push(("x-forwarded-host".to_string(), shared_host.to_string()));
    out.push(("x-forwarded-proto".to_string(), "https".to_string()));
    out
}

/// Is this `Set-Cookie` value for Access's cookie?
pub fn is_access_cookie(set_cookie: &str) -> bool {
    let name = set_cookie.split_once('=').map_or(set_cookie, |(n, _)| n).trim();
    name.eq_ignore_ascii_case("CF_Authorization")
}

/// `location` with an absolute or scheme-relative reference to `preview_host` replaced by `shared_origin`; anything else as it was.
pub fn rewrite_location(location: &str, preview_host: &str, shared_origin: &str) -> String {
    for scheme in ["https://", "http://", "//"] {
        let Some(rest) = strip_prefix_ignore_case(location, scheme) else { continue };
        let Some(after) = strip_prefix_ignore_case(rest, preview_host) else { continue };
        if after.is_empty() || after.starts_with(['/', '?', '#']) {
            return format!("{shared_origin}{after}");
        }
    }
    location.to_string()
}

fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        s.get(prefix.len()..)
    } else {
        None
    }
}

/// The headers of the response the client gets: see the module documentation. `preview_host` is `None` for production.
pub fn response_headers(
    headers: &[(String, String)],
    route: &str,
    preview_host: Option<&str>,
    shared_origin: &str,
) -> Vec<(String, String)> {
    let mut out = Vec::with_capacity(headers.len() + 1);
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if lower == ROUTE_HEADER || (lower == "set-cookie" && is_access_cookie(value)) {
            continue;
        }
        match (lower.as_str(), preview_host) {
            ("location", Some(host)) => out.push((name.clone(), rewrite_location(value, host, shared_origin))),
            _ => out.push((name.clone(), value.clone())),
        }
    }
    out.push((ROUTE_HEADER.to_string(), route.to_string()));
    out
}
