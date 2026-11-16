//! URL signing module using HMAC-SHA256
//!
//! Provides cryptographic URL signing to prevent unauthorized use of the image proxy.
//! Each subdomain can have its own signing key.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Generate a signature for a canonical path.
///
/// The signature is a 16-character truncated base64url-encoded HMAC-SHA256.
/// 16 chars = 96 bits of entropy, sufficient for URL protection.
pub fn generate_signature(key: &str, canonical_path: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC can take key of any size");
    mac.update(canonical_path.as_bytes());
    let result = mac.finalize();
    let full_sig = URL_SAFE_NO_PAD.encode(result.into_bytes());
    // Truncate to 16 chars for shorter URLs
    full_sig[..16].to_string()
}

/// Verify a signature for a canonical path using constant-time comparison.
///
/// Returns true if the signature is valid.
pub fn verify_signature(key: &str, canonical_path: &str, signature: &str) -> bool {
    let expected = generate_signature(key, canonical_path);
    // Use constant-time comparison to prevent timing attacks
    expected.as_bytes().ct_eq(signature.as_bytes()).into()
}

/// Build a canonical signing path from resize parameters.
///
/// The canonical path is built by:
/// 1. Sorting all key/value pairs alphabetically by key (excluding 's')
/// 2. Appending the source URL
///
/// This ensures the same signature works regardless of param order in the URL.
///
/// # Arguments
/// * `params` - Iterator of (key, value) pairs (signature 's' should be excluded)
/// * `source_url` - The source image URL
///
/// # Returns
/// A canonical path string like "/fit/cover/h/200/w/300/https://example.com/img.jpg"
pub fn build_canonical_path<'a>(
    params: impl Iterator<Item = (&'a str, &'a str)>,
    source_url: &str,
) -> String {
    // Collect and sort params by key
    let mut sorted_params: Vec<(&str, &str)> = params.collect();
    sorted_params.sort_by(|a, b| a.0.cmp(b.0));

    // Build the canonical path
    let mut path = String::new();
    for (key, value) in sorted_params {
        path.push('/');
        path.push_str(key);
        path.push('/');
        path.push_str(value);
    }
    path.push('/');
    path.push_str(source_url);

    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signature_roundtrip() {
        let key = "test-secret-key-at-least-32-chars!";
        let path = "/h/200/w/300/https://example.com/image.jpg";
        let sig = generate_signature(key, path);
        assert!(verify_signature(key, path, &sig));
    }

    #[test]
    fn test_signature_fails_on_tamper() {
        let key = "test-secret-key-at-least-32-chars!";
        let path = "/h/200/w/300/https://example.com/image.jpg";
        let sig = generate_signature(key, path);
        let tampered_path = "/h/200/w/400/https://example.com/image.jpg";
        assert!(!verify_signature(key, tampered_path, &sig));
    }

    #[test]
    fn test_signature_fails_wrong_key() {
        let key1 = "test-secret-key-at-least-32-chars!";
        let key2 = "different-secret-key-also-32-chars";
        let path = "/h/200/w/300/https://example.com/image.jpg";
        let sig = generate_signature(key1, path);
        assert!(!verify_signature(key2, path, &sig));
    }

    #[test]
    fn test_signature_length() {
        let key = "test-key";
        let path = "/w/300/h/200/https://example.com/image.jpg";
        let sig = generate_signature(key, path);
        assert_eq!(sig.len(), 16);
    }

    #[test]
    fn test_canonical_path_sorting() {
        let params = vec![("w", "300"), ("h", "200"), ("fit", "cover")];
        let url = "https://example.com/image.jpg";
        let canonical = build_canonical_path(params.into_iter(), url);
        // Should be sorted alphabetically: fit, h, w
        assert_eq!(
            canonical,
            "/fit/cover/h/200/w/300/https://example.com/image.jpg"
        );
    }

    #[test]
    fn test_canonical_path_consistent() {
        let key = "test-secret-key-at-least-32-chars!";
        let url = "https://example.com/image.jpg";

        // Different input orders
        let params1 = vec![("w", "300"), ("h", "200")];
        let params2 = vec![("h", "200"), ("w", "300")];

        let canonical1 = build_canonical_path(params1.into_iter(), url);
        let canonical2 = build_canonical_path(params2.into_iter(), url);

        // Should produce the same canonical path
        assert_eq!(canonical1, canonical2);

        // And therefore the same signature
        let sig1 = generate_signature(key, &canonical1);
        let sig2 = generate_signature(key, &canonical2);
        assert_eq!(sig1, sig2);
    }
}
