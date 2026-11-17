//! Validation for the internal preview provisioner. Access is enforced at the Cloudflare edge.
use serde::Deserialize;

#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    Inventory,
    Provision(i32),
    Deprovision(i32),
    Acknowledge(i32),
}

pub fn route(method: &str, path: &str) -> Option<Route> {
    if method == "GET" && path == "/v1/previews" {
        return Some(Route::Inventory);
    }
    let number = path.strip_prefix("/v1/previews/")?;
    if number.is_empty() || number.len() > 9 || number.starts_with('0') || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number = number.parse().ok()?;
    match method {
        "PUT" => Some(Route::Provision(number)),
        "DELETE" => Some(Route::Deprovision(number)),
        "POST" => Some(Route::Acknowledge(number)),
        _ => None,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    password: String,
}

pub fn password(bytes: &[u8]) -> Option<String> {
    if bytes.len() > 256 {
        return None;
    }
    let body: Body = serde_json::from_slice(bytes).ok()?;
    if !(24..=128).contains(&body.password.len())
        || !body.password.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    Some(body.password)
}

#[cfg(target_arch = "wasm32")]
mod runtime;

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    proptest! {
        #[test]
        fn prop_valid_numbers_route(n in 1i32..=999_999_999) {
            prop_assert_eq!(route("PUT", &format!("/v1/previews/{n}")), Some(Route::Provision(n)));
            prop_assert_eq!(route("DELETE", &format!("/v1/previews/{n}")), Some(Route::Deprovision(n)));
        }
        #[test]
        fn prop_untrusted_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..1024)) {
            let _ = password(&bytes);
        }
        #[test]
        fn prop_valid_password_round_trip(p in "[A-Za-z0-9_-]{24,128}") {
            let body = serde_json::json!({"password": p});
            prop_assert_eq!(password(body.to_string().as_bytes()), body["password"].as_str().map(str::to_owned));
        }
    }
    #[test]
    fn fuzz_replay_request_seeds() {
        for bytes in [
            include_bytes!("../fuzz/seeds/request/valid-json").as_slice(),
            include_bytes!("../fuzz/seeds/request/valid-path").as_slice(),
            b"".as_slice(),
        ] {
            let _ = password(bytes);
            if let Ok(path) = std::str::from_utf8(bytes) {
                let _ = route("PUT", path);
            }
        }
    }
}
