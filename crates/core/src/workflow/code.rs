//! The one-time code a terminal must present to decide an approval.
//!
//! The plain code is shown once, in the app and in the notification. Only a salted hash is
//! stored. Checking a guess is slow on purpose, and the compare does not stop at the first
//! byte that differs.

use hmac::Mac;
use sha2::Sha256;

/// How many wrong codes `ys` accepts before it stops until the app shows a new one.
pub const ATTEMPT_LIMIT: i64 = 5;

/// PBKDF2-HMAC-SHA256 rounds. A short code has to be slow to test from the stored hash.
const ROUNDS: u32 = 100_000;

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

/// Whether `code` is the one `salt` and `hash` were made from. Letter case does not matter.
pub fn matches(code: &str, salt: &[u8], hash: &[u8]) -> bool {
    let canonical = code.trim().to_ascii_uppercase();
    let got = derive(canonical.as_bytes(), salt);
    constant_eq(&got, hash)
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
    let mut out = [0u8; 32];
    pbkdf2(code, salt, &mut out);
    out
}

/// PBKDF2-HMAC-SHA256, one block. `out` is 32 bytes, the hash length.
fn pbkdf2(password: &[u8], salt: &[u8], out: &mut [u8; 32]) {
    type HmacSha256 = hmac::Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(password).expect("hmac accepts a key of any length");
    mac.update(salt);
    mac.update(&1u32.to_be_bytes());
    let mut u = mac.finalize().into_bytes();
    let mut mixed = u;
    for _ in 1..ROUNDS {
        let mut mac =
            HmacSha256::new_from_slice(password).expect("hmac accepts a key of any length");
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
        assert!(matches(&code, &salt, &hash));
        assert!(matches(&code.to_ascii_lowercase(), &salt, &hash));
        assert!(!matches("AAAAAAAA", &salt, &hash) || code == "AAAAAAAA");
        assert_ne!(hash, code.as_bytes());
        let (other, _, _) = issue();
        assert_ne!(code, other);
    }
}
