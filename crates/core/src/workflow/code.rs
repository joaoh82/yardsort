//! The one-time code a terminal must present to decide an approval.
//!
//! The plain code is shown once, in the app and in the notification. Only a salted hash is
//! stored. Checking a guess is slow on purpose, and the compare does not stop at the first
//! byte that differs.

use hmac::Mac;
use sha2::Sha256;

/// How many wrong codes `ys` accepts before it stops until the app shows a new one.
pub const ATTEMPT_LIMIT: i64 = 5;

/// PBKDF2-HMAC-SHA256 rounds for a code issued now. A short code has to be slow to test
/// from the stored hash.
pub const ROUNDS: u32 = 600_000;

/// Rounds used before this. A hash stored at this count still verifies. None of those
/// codes shipped in a release. The column default is this, so a row written before the
/// rounds were recorded verifies too.
pub const LEGACY_ROUNDS: u32 = 100_000;

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// A new code, and the salt and hash to store. The code is eight base32 characters.
pub fn issue() -> (String, Vec<u8>, Vec<u8>) {
    let id = uuid::Uuid::new_v4();
    let bytes = id.as_bytes();
    let code = encode(&bytes[..5]);
    let salt = bytes[5..].to_vec();
    let hash = derive(code.as_bytes(), &salt).to_vec();
    (code, salt, hash)
}

/// `ROUNDS` and [`LEGACY_ROUNDS`] are the only counts a check will run. Anything else,
/// including 0 or 1, is refused before any hashing, so a row cannot ask for a huge count.
pub fn known_rounds(rounds: u32) -> bool {
    rounds == ROUNDS || rounds == LEGACY_ROUNDS
}

/// Whether `code` is the one `salt` and `hash` were made from, at `rounds`. Letter case
/// does not matter. An unknown `rounds` does not match.
pub fn matches(code: &str, salt: &[u8], hash: &[u8], rounds: u32) -> bool {
    let canonical = code.trim().to_ascii_uppercase();
    let Some(got) = digest(canonical.as_bytes(), salt, rounds) else {
        return false;
    };
    constant_eq(&got, hash)
}

/// The hash to store for `code` at `rounds`. `None` when `rounds` is not a known count.
pub fn digest(code: &[u8], salt: &[u8], rounds: u32) -> Option<[u8; 32]> {
    if !known_rounds(rounds) {
        return None;
    }
    let mut out = [0u8; 32];
    pbkdf2(code, salt, rounds, &mut out);
    Some(out)
}

fn encode(bytes: &[u8]) -> String {
    let mut n = 0u64;
    for byte in bytes {
        n = (n << 8) | u64::from(*byte);
    }
    let mut out = [0u8; 8];
    for slot in out.iter_mut().rev() {
        *slot = ALPHABET[(n & 31) as usize];
        n >>= 5;
    }
    String::from_utf8(out.to_vec()).unwrap_or_else(|_| "AAAAAAAA".to_owned())
}

fn derive(code: &[u8], salt: &[u8]) -> [u8; 32] {
    digest(code, salt, ROUNDS).expect("the current round count is accepted")
}

/// PBKDF2-HMAC-SHA256, one block. `out` is 32 bytes, the hash length.
/// The key is set up once and cloned for each round.
fn pbkdf2(password: &[u8], salt: &[u8], rounds: u32, out: &mut [u8; 32]) {
    type HmacSha256 = hmac::Hmac<Sha256>;
    let keyed = HmacSha256::new_from_slice(password).expect("hmac accepts a key of any length");
    let mut mac = keyed.clone();
    mac.update(salt);
    mac.update(&1u32.to_be_bytes());
    let mut u = mac.finalize().into_bytes();
    let mut mixed = u;
    for _ in 1..rounds {
        let mut mac = keyed.clone();
        mac.update(&u);
        u = mac.finalize().into_bytes();
        for (left, right) in mixed.iter_mut().zip(u.iter()) {
            *left ^= *right;
        }
    }
    out.copy_from_slice(&mixed);
}

fn constant_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_code_matches_only_itself_and_its_hash_is_not_the_code() {
        let (code, salt, hash) = issue();
        assert_eq!(code.len(), 8);
        assert!(matches(&code, &salt, &hash, ROUNDS));
        assert!(matches(&code.to_ascii_lowercase(), &salt, &hash, ROUNDS));
        assert!(!matches("AAAAAAAA", &salt, &hash, ROUNDS) || code == "AAAAAAAA");
        assert!(!matches(&code, &salt, &hash, LEGACY_ROUNDS));
        assert_ne!(hash, code.as_bytes());
        let (other, _, _) = issue();
        assert_ne!(code, other);
    }

    #[test]
    fn a_hash_made_at_the_old_round_count_still_matches() {
        let salt = b"old-salt";
        let hash = digest(b"ABCD2345", salt, LEGACY_ROUNDS).expect("legacy rounds");
        assert!(digest(b"ABCD2345", salt, 0).is_none());
        assert!(digest(b"ABCD2345", salt, 1).is_none());
        assert!(matches("abcd2345", salt, &hash, LEGACY_ROUNDS));
        assert!(!matches("abcd2345", salt, &hash, ROUNDS));
        assert!(!matches("abcd2345", salt, &hash, 0));
        assert!(!matches("abcd2345", salt, &hash, 1));
    }
}
