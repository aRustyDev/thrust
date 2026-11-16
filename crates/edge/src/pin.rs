//! The pin cookie (ADR-0111): who opted in, as which kind of principal, through which audience, to which target, until when.
//!
//! The value is `v1.<payload>.<mac>`: `payload` is the base64url (no padding) of a JSON object with exactly the five fields of
//! [`Pin`], and `mac` the base64url HMAC-SHA256, under the edge's secret `EDGE_PIN_KEY`, of the bytes `v1.<payload>`. The MAC is
//! checked before the payload is parsed, so nothing an unauthenticated client wrote is ever parsed as JSON, and the comparison is
//! the constant-time one of `hmac`. A malformed, oversized, forged or expired value is refused; time is passed in.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// The cookie's name. `__Host-` makes the browser refuse it unless it is `Secure`, has `Path=/` and no `Domain`, so a sibling host
/// cannot plant one for the shared hostname.
pub const COOKIE_NAME: &str = "__Host-thrust_edge";
/// The longest cookie value the edge reads. Real ones are about 250 bytes.
pub const MAX_VALUE_LEN: usize = 1024;
/// The shortest acceptable key, in bytes (the HMAC-SHA256 block is 64; anything under 32 is a mistake).
pub const MIN_KEY_LEN: usize = 32;
const VERSION: &str = "v1";
const MAX_PRINCIPAL_LEN: usize = 254;

/// The kind of principal that opted in: a person (Access gave an email) or CI (a service token, its `common_name`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Human,
    Ci,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Human => "human",
            Kind::Ci => "ci",
        }
    }
}

/// What a valid pin says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    /// The email (a person) or the service token's `common_name` (CI).
    #[serde(rename = "p")]
    pub principal: String,
    #[serde(rename = "k")]
    pub kind: Kind,
    /// The Access path that admitted the principal (`/_edge/opt-in/<audience>`).
    #[serde(rename = "a")]
    pub audience: String,
    /// The target or alias asked for.
    #[serde(rename = "r")]
    pub requested: String,
    /// Expiry, seconds since the Unix epoch.
    #[serde(rename = "e")]
    pub exp: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinError {
    /// The key is shorter than [`MIN_KEY_LEN`].
    KeyTooShort,
    /// Longer than [`MAX_VALUE_LEN`].
    TooLong,
    /// Not `v1.<payload>.<mac>` with base64url parts.
    Malformed,
    /// The MAC does not match.
    BadSignature,
    /// Signed, but not the five fields of a pin, or a field outside its grammar.
    Invalid,
    /// `exp` is not after now.
    Expired,
}

type HmacSha256 = Hmac<Sha256>;

fn mac(key: &[u8]) -> Result<HmacSha256, PinError> {
    if key.len() < MIN_KEY_LEN {
        return Err(PinError::KeyTooShort);
    }
    HmacSha256::new_from_slice(key).map_err(|_| PinError::KeyTooShort)
}

/// An audience: the last segment of `/_edge/opt-in/<audience>`. Lowercase letters only, 1 to 16 of them.
pub fn valid_audience(s: &str) -> bool {
    (1..=16).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase())
}

/// A target or alias name: a lowercase letter or digit, then up to 56 lowercase letters, digits, `.` or `-` (so `v0.3.0`,
/// `staging`, `dev-login-page`). 57 characters at most, so `target-<name>` fits Flagship's 64-character key.
pub fn valid_name(s: &str) -> bool {
    let b = s.as_bytes();
    match b.split_first() {
        Some((first, rest)) => {
            b.len() <= 57
                && (first.is_ascii_lowercase() || first.is_ascii_digit())
                && rest.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'.' || *c == b'-')
        }
        None => false,
    }
}

/// A principal: 1 to 254 visible ASCII characters (an email or a token's `common_name`), no space and no control character.
pub fn valid_principal(s: &str) -> bool {
    (1..=MAX_PRINCIPAL_LEN).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_graphic())
}

fn valid(pin: &Pin) -> bool {
    valid_principal(&pin.principal) && valid_audience(&pin.audience) && valid_name(&pin.requested)
}

/// The cookie value for `pin`, signed with `key`. Refuses a pin whose fields are outside their grammar.
pub fn seal(pin: &Pin, key: &[u8]) -> Result<String, PinError> {
    if !valid(pin) {
        return Err(PinError::Invalid);
    }
    let json = serde_json::to_vec(pin).map_err(|_| PinError::Invalid)?;
    let signed = format!("{VERSION}.{}", URL_SAFE_NO_PAD.encode(json));
    let mut m = mac(key)?;
    m.update(signed.as_bytes());
    let tag = URL_SAFE_NO_PAD.encode(m.finalize().into_bytes());
    Ok(format!("{signed}.{tag}"))
}

/// The pin in `value` if it is whole, signed with `key` and not expired at `now`.
pub fn open(value: &str, key: &[u8], now: u64) -> Result<Pin, PinError> {
    let mut m = mac(key)?;
    if value.len() > MAX_VALUE_LEN {
        return Err(PinError::TooLong);
    }
    let (signed, tag) = value.rsplit_once('.').ok_or(PinError::Malformed)?;
    let (version, payload) = signed.split_once('.').ok_or(PinError::Malformed)?;
    if version != VERSION || payload.is_empty() || tag.is_empty() {
        return Err(PinError::Malformed);
    }
    let tag = URL_SAFE_NO_PAD.decode(tag).map_err(|_| PinError::Malformed)?;
    let payload = URL_SAFE_NO_PAD.decode(payload).map_err(|_| PinError::Malformed)?;
    m.update(signed.as_bytes());
    m.verify_slice(&tag).map_err(|_| PinError::BadSignature)?;
    let pin: Pin = serde_json::from_slice(&payload).map_err(|_| PinError::Invalid)?;
    if !valid(&pin) {
        return Err(PinError::Invalid);
    }
    if pin.exp <= now {
        return Err(PinError::Expired);
    }
    Ok(pin)
}

/// The value of the pin cookie in a `Cookie` header, if there is exactly one. Two or more is refused (`None`): a browser sends one
/// `__Host-` cookie per name for the host, so a second is somebody else's.
pub fn from_cookie_header(header: &str) -> Option<&str> {
    let mut found = None;
    for pair in header.split(';') {
        let pair = pair.trim();
        if let Some((name, value)) = pair.split_once('=') {
            if name.trim() == COOKIE_NAME {
                if found.is_some() {
                    return None;
                }
                found = Some(value.trim());
            }
        }
    }
    found
}

/// The `Set-Cookie` that stores `value` for `max_age` seconds.
pub fn set_cookie(value: &str, max_age: u64) -> String {
    format!("{COOKIE_NAME}={value}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={max_age}")
}

/// The `Set-Cookie` that removes the pin.
pub fn clear_cookie() -> String {
    format!("{COOKIE_NAME}=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0")
}
