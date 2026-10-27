//! The server's own credentials and one-time values: random, shown once, kept only as hashes.
//!
//! Each is 256 random bits, so a plain SHA-256 is what it is stored as: there is nothing to
//! guess, and a slow hash would only slow down every request. Whoever reads the records gets
//! hashes that open nothing.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use remote_proto::account::Secret;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Random bytes in every credential and one-time value.
const ENTROPY: usize = 32;

/// What a credential is for. It is written into the credential, so one kind cannot be presented
/// as another, and a leaked one says what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `Authorization: Bearer` on each request.
    Access,
    /// Traded, once, for new credentials.
    Refresh,
    /// The one-time code a sign-in sends to the device's redirect.
    Handoff,
    /// The secret in a host's pairing QR code.
    Offer,
}

impl Kind {
    fn prefix(self) -> &'static str {
        match self {
            Self::Access => "ys1.at.",
            Self::Refresh => "ys1.rt.",
            Self::Handoff => "ys1.hc.",
            Self::Offer => "ys1.po.",
        }
    }
}

/// A credential's SHA-256: what the records keep, and what a presented credential is looked up by.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TokenHash(pub [u8; 32]);

impl std::fmt::Debug for TokenHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Enough to tell two apart in a test failure.
        write!(f, "TokenHash({:02x}{:02x}…)", self.0[0], self.0[1])
    }
}

/// A new credential: the text, handed out once, and the hash that is kept.
pub struct Minted {
    pub secret: Secret,
    pub hash: TokenHash,
}

/// A new credential of `kind`.
pub fn mint(kind: Kind) -> Minted {
    let text = format!("{}{}", kind.prefix(), random());
    Minted {
        hash: hash(&text),
        secret: Secret::new(text),
    }
}

/// The hash a presented credential of `kind` is looked up by, or `None` when it is not of that
/// kind at all.
pub fn presented(kind: Kind, text: &str) -> Option<TokenHash> {
    text.starts_with(kind.prefix()).then(|| hash(text))
}

/// 256 random bits, base64url without padding: 43 characters, all of them allowed in a PKCE
/// verifier, a URL query and an OAuth `state`.
///
/// Panics when the operating system has no randomness to give. A server in that state must not
/// issue anything.
pub fn random() -> String {
    let mut bytes = [0u8; ENTROPY];
    getrandom::fill(&mut bytes).expect("the operating system's random source");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// A pairing's short code: five random bytes as eight characters, and the hash of it that is kept.
/// It is no credential: it finds an offer within one account, a few tries at most.
pub fn short_code() -> Minted {
    let mut bytes = [0u8; 5];
    getrandom::fill(&mut bytes).expect("the operating system's random source");
    let code = remote_proto::pairing::format_code(bytes);
    Minted {
        hash: code_hash(&code),
        secret: Secret::new(remote_proto::pairing::display_code(&code)),
    }
}

/// The hash a short code is kept and looked up by, `code` being normalized already. Its own
/// prefix keeps it apart from every credential's hash.
pub fn code_hash(code: &str) -> TokenHash {
    hash(&format!("ys1.pc.{code}"))
}

pub fn hash(text: &str) -> TokenHash {
    TokenHash(Sha256::digest(text.as_bytes()).into())
}

/// The PKCE S256 challenge for `verifier`.
pub fn s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Whether `verifier` is the one `challenge` was made from. Compared in constant time.
pub fn verifies(challenge: &str, verifier: &str) -> bool {
    s256(verifier).as_bytes().ct_eq(challenge.as_bytes()).into()
}

/// Whether two hashes are the same. Compared in constant time.
pub fn same(a: &TokenHash, b: &TokenHash) -> bool {
    a.0.ct_eq(&b.0).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rfc_7636_example_verifies() {
        // Appendix B of RFC 7636.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(s256(verifier), challenge);
        assert!(verifies(challenge, verifier));
        assert!(!verifies(challenge, &verifier.replace('d', "e")));
    }

    #[test]
    fn credentials_differ_say_their_kind_and_hash_to_what_is_kept() {
        let a = mint(Kind::Refresh);
        let b = mint(Kind::Refresh);
        assert_ne!(a.secret, b.secret);
        assert!(a.secret.expose().starts_with("ys1.rt."));
        assert_eq!(presented(Kind::Refresh, a.secret.expose()), Some(a.hash));
        assert_eq!(presented(Kind::Access, a.secret.expose()), None);
        assert_eq!(random().len(), 43);
    }
}
