use hmac::{Hmac, Mac};
use rand::RngCore;
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// Generate a fresh, cryptographically secure secret for use as hub.secret.
/// Returns a 32-byte hex string (64 characters).
pub fn generate_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Compute HMAC-SHA1 of `body` using `secret` and return as lowercase hex.
fn compute_sha1(secret: &str, body: &[u8]) -> String {
    let mut mac = HmacSha1::new_from_slice(secret.as_bytes())
        .expect("HMAC can take any key length");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Verify an `X-Hub-Signature` header against the request body.
/// The header value must be in the form `sha1=<hex>`.
/// Uses constant-time comparison to prevent timing attacks.
pub fn verify(header_value: &str, secret: &str, body: &[u8]) -> bool {
    let Some(received_hex) = header_value.strip_prefix("sha1=") else {
        return false;
    };

    let expected_hex = compute_sha1(secret, body);

    constant_time_eq(received_hex.as_bytes(), expected_hex.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    // HMAC-SHA1 Signature Verification Spec
    //
    // WebSub 4.1: hub.secret is a shared secret registered at subscribe time.
    // On each push, Hub computes HMAC-SHA1(secret, body) and sends it as
    // `X-Hub-Signature: sha1=<hex>`. Subscriber must verify before trusting body.
    // Constant-time comparison is required to prevent timing attacks.

    #[test]
    fn test_generate_secret_length_and_charset() {
        let secret = generate_secret();
        assert_eq!(secret.len(), 64, "32 bytes hex = 64 chars");
        assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_secret_uniqueness() {
        let s1 = generate_secret();
        let s2 = generate_secret();
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_verify_matching_signature() {
        let secret = "my_secret_key";
        let body = b"some atom feed body";
        let sig = compute_sha1(secret, body);
        let header = format!("sha1={}", sig);

        assert!(verify(&header, secret, body));
    }

    #[test]
    fn test_verify_tampered_body() {
        let secret = "my_secret_key";
        let original_body = b"original";
        let sig = compute_sha1(secret, original_body);
        let header = format!("sha1={}", sig);

        assert!(!verify(&header, secret, b"tampered"));
    }

    #[test]
    fn test_verify_wrong_secret() {
        let body = b"body";
        let sig = compute_sha1("secret_a", body);
        let header = format!("sha1={}", sig);

        assert!(!verify(&header, "secret_b", body));
    }

    #[test]
    fn test_verify_missing_sha1_prefix() {
        let secret = "s";
        let body = b"b";
        let sig = compute_sha1(secret, body);
        // Header without the "sha1=" prefix should be rejected.
        assert!(!verify(&sig, secret, body));
    }

    #[test]
    fn test_verify_known_hmac_sha1_vector() {
        // RFC 2202 test case 1: key = 20 bytes of 0x0b, data = "Hi There"
        // Expected HMAC-SHA1 = b617318655057264e28bc0b6fb378c8ef146be00
        let key = "\x0b".repeat(20);
        let body = b"Hi There";
        let expected = "b617318655057264e28bc0b6fb378c8ef146be00";

        assert_eq!(compute_sha1(&key, body), expected);
        assert!(verify(&format!("sha1={}", expected), &key, body));
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(constant_time_eq(b"", b""));
    }
}
