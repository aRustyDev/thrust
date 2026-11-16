//! The Access token on an opt-in path (ADR-0111): Cloudflare Access puts `Cf-Access-Jwt-Assertion` on a request it admitted. The
//! edge does not trust that a request came through Access because of a header; it verifies the token.
//!
//! The order is: size, shape, `alg` (RS256 and nothing else, so `none` and an HMAC token signed with a public key are refused
//! before any key is touched), the key named by `kid` from the team's JWKS, the signature over `header.payload`, and only then the
//! claims: `iss` (the team), `aud` (the AUD of the Access app of the path's audience), `exp`, `nbf`. The principal is the `email`
//! claim (a person) or, without one, `common_name` (a service token, so CI). Fetching and caching the JWKS is the runtime's job;
//! this module is pure, and time is passed in.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rsa::traits::PublicKeyParts as _;
use rsa::{BigUint, Pkcs1v15Sign, RsaPublicKey};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::pin::{valid_principal, Kind};

/// The longest token the edge reads.
pub const MAX_TOKEN_LEN: usize = 8192;
/// The longest JWKS document the edge parses.
pub const MAX_JWKS_LEN: usize = 65536;
/// RSA modulus sizes accepted, in bits.
pub const MIN_RSA_BITS: usize = 2048;
pub const MAX_RSA_BITS: usize = 4096;
/// Allowance for clock skew on `nbf`, in seconds. `exp` gets none.
pub const NBF_LEEWAY: u64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessError {
    TooLong,
    Malformed,
    /// `alg` is not RS256, or the header carries `crit`.
    Algorithm,
    /// No key in the JWKS has the token's `kid` (or the token has none).
    UnknownKey,
    BadSignature,
    Issuer,
    Audience,
    Expired,
    NotYetValid,
    /// Neither a usable `email` nor a usable `common_name`.
    NoPrincipal,
    /// The JWKS document has no usable RSA signing key.
    NoKeys,
}

/// The team's signing keys, by `kid`.
#[derive(Clone, Debug)]
pub struct Jwks {
    keys: Vec<(String, RsaPublicKey)>,
}

#[allow(
    clippy::len_without_is_empty,
    reason = "a Jwks is never empty: parse_jwks refuses a document without a usable key"
)]
impl Jwks {
    /// How many usable keys the document had (at least one).
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    fn get(&self, kid: &str) -> Option<&RsaPublicKey> {
        self.keys.iter().find(|(k, _)| k == kid).map(|(_, key)| key)
    }
}

#[derive(Deserialize)]
struct JwksDoc {
    keys: Vec<JwkDoc>,
}

#[derive(Deserialize)]
struct JwkDoc {
    kid: Option<String>,
    kty: Option<String>,
    alg: Option<String>,
    #[serde(rename = "use")]
    use_: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

/// The usable RSA signing keys of a JWKS document (`https://<team>/cdn-cgi/access/certs`). Other members of the document are
/// ignored; a key that is not RSA, is for another algorithm or use, lacks `kid`, `n` or `e`, or has a modulus outside
/// [`MIN_RSA_BITS`]..=[`MAX_RSA_BITS`] is skipped. No usable key at all is an error.
pub fn parse_jwks(doc: &[u8]) -> Result<Jwks, AccessError> {
    if doc.len() > MAX_JWKS_LEN {
        return Err(AccessError::TooLong);
    }
    let doc: JwksDoc = serde_json::from_slice(doc).map_err(|_| AccessError::Malformed)?;
    let mut keys = Vec::new();
    for k in doc.keys {
        if k.kty.as_deref() != Some("RSA")
            || k.alg.as_deref().is_some_and(|a| a != "RS256")
            || k.use_.as_deref().is_some_and(|u| u != "sig")
        {
            continue;
        }
        let (Some(kid), Some(n), Some(e)) = (k.kid, k.n, k.e) else { continue };
        let (Ok(n), Ok(e)) = (URL_SAFE_NO_PAD.decode(n), URL_SAFE_NO_PAD.decode(e)) else { continue };
        let Ok(key) = RsaPublicKey::new(BigUint::from_bytes_be(&n), BigUint::from_bytes_be(&e)) else { continue };
        let bits = key.size() * 8;
        if kid.is_empty() || !(MIN_RSA_BITS..=MAX_RSA_BITS).contains(&bits) {
            continue;
        }
        keys.push((kid, key));
    }
    if keys.is_empty() {
        return Err(AccessError::NoKeys);
    }
    Ok(Jwks { keys })
}

/// What a token must say to be accepted.
#[derive(Clone, Copy, Debug)]
pub struct Expect<'a> {
    /// `https://<team>.cloudflareaccess.com`.
    pub issuer: &'a str,
    /// The AUD tag of the Access application of the audience's path.
    pub audience: &'a str,
    /// Seconds since the Unix epoch.
    pub now: u64,
}

/// Who the token is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub principal: String,
    pub kind: Kind,
}

#[derive(Deserialize)]
struct Header {
    alg: String,
    kid: Option<String>,
    crit: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Aud {
    One(String),
    Many(Vec<String>),
}

impl Aud {
    fn contains(&self, want: &str) -> bool {
        match self {
            Aud::One(a) => a == want,
            Aud::Many(all) => all.iter().any(|a| a == want),
        }
    }
}

#[derive(Deserialize)]
struct Claims {
    aud: Aud,
    iss: String,
    exp: u64,
    nbf: Option<u64>,
    email: Option<String>,
    common_name: Option<String>,
}

/// Verifies `token` against `jwks` and `expect`, and says who it is for.
pub fn verify(token: &str, jwks: &Jwks, expect: &Expect<'_>) -> Result<Identity, AccessError> {
    if token.len() > MAX_TOKEN_LEN {
        return Err(AccessError::TooLong);
    }
    let mut parts = token.split('.');
    let (Some(h64), Some(p64), Some(s64), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(AccessError::Malformed);
    };
    let header = URL_SAFE_NO_PAD.decode(h64).map_err(|_| AccessError::Malformed)?;
    let header: Header = serde_json::from_slice(&header).map_err(|_| AccessError::Malformed)?;
    if header.alg != "RS256" || header.crit.is_some() {
        return Err(AccessError::Algorithm);
    }
    let key = header.kid.as_deref().and_then(|kid| jwks.get(kid)).ok_or(AccessError::UnknownKey)?;
    let signature = URL_SAFE_NO_PAD.decode(s64).map_err(|_| AccessError::Malformed)?;
    let signed_len = h64.len() + 1 + p64.len();
    let signed = token.get(..signed_len).ok_or(AccessError::Malformed)?;
    let hashed = Sha256::digest(signed.as_bytes());
    key.verify(Pkcs1v15Sign::new::<Sha256>(), &hashed, &signature).map_err(|_| AccessError::BadSignature)?;

    let claims = URL_SAFE_NO_PAD.decode(p64).map_err(|_| AccessError::Malformed)?;
    let claims: Claims = serde_json::from_slice(&claims).map_err(|_| AccessError::Malformed)?;
    if claims.iss != expect.issuer {
        return Err(AccessError::Issuer);
    }
    if !claims.aud.contains(expect.audience) {
        return Err(AccessError::Audience);
    }
    if claims.exp <= expect.now {
        return Err(AccessError::Expired);
    }
    if claims.nbf.is_some_and(|nbf| nbf > expect.now.saturating_add(NBF_LEEWAY)) {
        return Err(AccessError::NotYetValid);
    }
    match (claims.email.filter(|e| !e.is_empty()), claims.common_name.filter(|c| !c.is_empty())) {
        (Some(email), _) if valid_principal(&email) => Ok(Identity { principal: email, kind: Kind::Human }),
        (None, Some(cn)) if valid_principal(&cn) => Ok(Identity { principal: cn, kind: Kind::Ci }),
        _ => Err(AccessError::NoPrincipal),
    }
}
