//! Source URL filtering to prevent open proxy misuse.
//!
//! Supports pattern types:
//! - Exact domain: `example.com`
//! - Wildcard subdomain: `*.example.com`
//! - Path prefix: `cdn.example.com/bucket/*`

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::Url;

/// Policy for one source check. Private and reserved addresses are rejected
/// even when `allow_all` is set, unless `allow_private_networks` is set.
#[derive(Debug, Clone, Copy)]
pub struct SourcePolicy {
    pub allow_all: bool,
    pub allow_private_networks: bool,
}

/// Check if a source URL is allowed by the pattern list and policy.
///
/// Checks run in this order: parse, http(s) scheme, no userinfo, private or
/// reserved host (unless `allow_private_networks`), `allow_all`, then patterns.
pub fn is_source_allowed(source_url: &str, patterns: &[String], policy: &SourcePolicy) -> bool {
    let url = match Url::parse(source_url) {
        Ok(u) => u,
        Err(_) => return false,
    };

    if url.scheme() != "http" && url.scheme() != "https" {
        return false;
    }

    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }

    if !policy.allow_private_networks && is_private_network(&url) {
        return false;
    }

    if policy.allow_all {
        return true;
    }

    if patterns.is_empty() {
        return false;
    }

    let host = match url.host_str() {
        Some(h) => h.to_lowercase(),
        None => return false,
    };
    let path = url.path();

    patterns
        .iter()
        .any(|pattern| matches_pattern(&host, path, pattern))
}

/// True for private, loopback, link-local, documentation, multicast, and other reserved addresses.
pub fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_forbidden_v4(v4),
        IpAddr::V6(v6) => is_forbidden_v6(v6),
    }
}

fn is_forbidden_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    // 0.0.0.0/8, 10.0.0.0/8, 127.0.0.0/8
    if a == 0 || a == 10 || a == 127 {
        return true;
    }
    // 100.64.0.0/10 CGNAT
    if a == 100 && (64..=127).contains(&b) {
        return true;
    }
    // 169.254.0.0/16 link-local
    if a == 169 && b == 254 {
        return true;
    }
    // 172.16.0.0/12
    if a == 172 && (16..=31).contains(&b) {
        return true;
    }
    // 192.0.0.0/24, 192.0.2.0/24, 192.168.0.0/16
    if a == 192 && b == 0 && c == 0 {
        return true;
    }
    if a == 192 && b == 0 && c == 2 {
        return true;
    }
    if a == 192 && b == 168 {
        return true;
    }
    // 198.18.0.0/15 benchmarking
    if a == 198 && (b == 18 || b == 19) {
        return true;
    }
    // 198.51.100.0/24, 203.0.113.0/24 documentation
    if a == 198 && b == 51 && c == 100 {
        return true;
    }
    if a == 203 && b == 0 && c == 113 {
        return true;
    }
    // 224.0.0.0/4 multicast and 240.0.0.0/4 reserved (includes broadcast)
    a >= 224
}

fn is_forbidden_v6(ip: Ipv6Addr) -> bool {
    if ip.is_unspecified() || ip.is_loopback() {
        return true;
    }

    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_forbidden_v4(v4);
    }

    let s = ip.segments();

    // Deprecated IPv4-compatible ::/96. Check the embedded v4.
    if s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0 {
        let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        return is_forbidden_v4(v4);
    }

    // NAT64 64:ff9b::/96
    if s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0 {
        let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        return is_forbidden_v4(v4);
    }

    // 6to4 2002::/16. The next 32 bits are the IPv4 address.
    if s[0] == 0x2002 {
        let v4 = Ipv4Addr::new((s[1] >> 8) as u8, s[1] as u8, (s[2] >> 8) as u8, s[2] as u8);
        return is_forbidden_v4(v4);
    }

    // 100::/64 discard-only
    if s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0 {
        return true;
    }
    // 2001:db8::/32 documentation
    if s[0] == 0x2001 && s[1] == 0x0db8 {
        return true;
    }
    // fc00::/7 unique local
    if s[0] & 0xfe00 == 0xfc00 {
        return true;
    }
    // fe80::/10 link-local
    if s[0] & 0xffc0 == 0xfe80 {
        return true;
    }
    // ff00::/8 multicast
    s[0] & 0xff00 == 0xff00
}

/// Check if a URL points to a private or reserved address, or a localhost name.
fn is_private_network(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => is_forbidden_ip(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => is_forbidden_ip(IpAddr::V6(ip)),
        Some(url::Host::Domain(host)) => {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            host == "localhost" || host.ends_with(".localhost")
        }
        None => false,
    }
}

fn path_has_dot_segment(path: &str) -> bool {
    let decoded = percent_encoding::percent_decode_str(path).decode_utf8_lossy();
    decoded
        .split('/')
        .any(|segment| segment == "." || segment == "..")
}

/// Check if a host/path matches a single pattern.
///
/// Pattern formats:
/// - `*` - match all domains
/// - `example.com` - exact domain match
/// - `*.example.com` - wildcard subdomain (also matches example.com itself)
/// - `example.com/path/*` - domain + path prefix
fn matches_pattern(host: &str, path: &str, pattern: &str) -> bool {
    let pattern = pattern.trim().to_lowercase();

    if pattern.is_empty() {
        return false;
    }

    if pattern == "*" {
        return true;
    }

    let (domain_pattern, path_pattern) = if let Some((domain, path_pat)) = pattern.split_once('/') {
        (domain, Some(path_pat))
    } else {
        (pattern.as_str(), None)
    };

    let domain_matches = if let Some(bare) = domain_pattern.strip_prefix("*.") {
        host == bare || host.ends_with(&format!(".{bare}"))
    } else {
        host == domain_pattern
    };

    if !domain_matches {
        return false;
    }

    let Some(path_pat) = path_pattern else {
        return true;
    };

    // Dot segments are rejected after full percent-decoding. The prefix itself
    // is compared to the raw path so a %2F inside a key still matches.
    if path_has_dot_segment(path) {
        return false;
    }

    let wanted = format!("/{path_pat}");
    if let Some(prefix) = wanted.strip_suffix('*') {
        path.starts_with(prefix)
    } else {
        path == wanted
    }
}

/// Extract just the domain from a URL for error messages.
pub fn extract_domain(url: &str) -> Option<String> {
    Url::parse(url).ok()?.host_str().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(allow_all: bool, allow_private: bool) -> SourcePolicy {
        SourcePolicy {
            allow_all,
            allow_private_networks: allow_private,
        }
    }

    fn closed() -> SourcePolicy {
        policy(false, false)
    }

    fn allowed(url: &str, patterns: &[String]) -> bool {
        is_source_allowed(url, patterns, &closed())
    }

    #[test]
    fn test_exact_domain_match() {
        let patterns = vec!["example.com".to_string()];
        assert!(allowed("https://example.com/image.jpg", &patterns));
        assert!(allowed("https://example.com/path/to/image.jpg", &patterns));
        assert!(!allowed("https://other.com/image.jpg", &patterns));
        assert!(!allowed("https://sub.example.com/image.jpg", &patterns));
    }

    #[test]
    fn test_wildcard_subdomain() {
        let patterns = vec!["*.example.com".to_string()];
        assert!(allowed("https://cdn.example.com/image.jpg", &patterns));
        assert!(allowed("https://images.cdn.example.com/x.jpg", &patterns));
        assert!(allowed("https://example.com/image.jpg", &patterns));
        assert!(!allowed("https://example.org/image.jpg", &patterns));
        assert!(!allowed("https://notexample.com/image.jpg", &patterns));
    }

    #[test]
    fn test_path_prefix() {
        let patterns = vec!["storage.googleapis.com/my-bucket/*".to_string()];
        assert!(allowed(
            "https://storage.googleapis.com/my-bucket/image.jpg",
            &patterns
        ));
        assert!(allowed(
            "https://storage.googleapis.com/my-bucket/subdir/image.jpg",
            &patterns
        ));
        assert!(!allowed(
            "https://storage.googleapis.com/other-bucket/image.jpg",
            &patterns
        ));
    }

    #[test]
    fn test_exact_path() {
        let patterns = vec!["example.com/specific/path".to_string()];
        assert!(allowed("https://example.com/specific/path", &patterns));
        assert!(!allowed(
            "https://example.com/specific/path/more",
            &patterns
        ));
        assert!(!allowed("https://example.com/other", &patterns));
    }

    #[test]
    fn test_allow_all_bypasses() {
        let patterns = vec![];
        assert!(is_source_allowed(
            "https://anything.com/image.jpg",
            &patterns,
            &policy(true, false)
        ));
        assert!(!allowed("https://anything.com/image.jpg", &patterns));
    }

    #[test]
    fn test_empty_patterns_blocks_all() {
        let patterns: Vec<String> = vec![];
        assert!(!allowed("https://example.com/image.jpg", &patterns));
    }

    #[test]
    fn test_case_insensitive() {
        let patterns = vec!["Example.COM".to_string()];
        assert!(allowed("https://EXAMPLE.com/image.jpg", &patterns));
        assert!(allowed("https://example.COM/image.jpg", &patterns));
    }

    #[test]
    fn test_invalid_url() {
        let patterns = vec!["example.com".to_string()];
        assert!(!allowed("not-a-valid-url", &patterns));
        assert!(!allowed("", &patterns));
    }

    #[test]
    fn test_private_network_blocked() {
        let patterns = vec!["*".to_string()];
        assert!(!allowed("http://localhost/image.jpg", &patterns));
        assert!(!allowed("http://localhost:8080/image.jpg", &patterns));
        assert!(!allowed("http://127.0.0.1/image.jpg", &patterns));
        assert!(!allowed("http://127.1.2.3/image.jpg", &patterns));
        assert!(!allowed("http://10.0.0.1/image.jpg", &patterns));
        assert!(!allowed("http://10.255.255.255/image.jpg", &patterns));
        assert!(!allowed("http://192.168.1.1/image.jpg", &patterns));
        assert!(!allowed("http://192.168.0.100/image.jpg", &patterns));
        assert!(!allowed("http://172.16.0.1/image.jpg", &patterns));
        assert!(!allowed("http://172.31.255.255/image.jpg", &patterns));
        assert!(allowed("http://172.15.0.1/image.jpg", &patterns));
        assert!(allowed("http://172.32.0.1/image.jpg", &patterns));
        assert!(!allowed("http://0.0.0.0/image.jpg", &patterns));
        assert!(!allowed("http://[::1]/image.jpg", &patterns));
    }

    #[test]
    fn test_private_network_blocked_when_allow_all() {
        let patterns: Vec<String> = vec![];
        let open = policy(true, false);
        assert!(!is_source_allowed(
            "http://localhost/image.jpg",
            &patterns,
            &open
        ));
        assert!(!is_source_allowed(
            "http://192.168.1.1/image.jpg",
            &patterns,
            &open
        ));
    }

    #[test]
    fn test_multiple_patterns() {
        let patterns = vec![
            "cdn.example.com".to_string(),
            "*.storage.example.com".to_string(),
            "images.other.org/public/*".to_string(),
        ];
        assert!(allowed("https://cdn.example.com/img.jpg", &patterns));
        assert!(allowed("https://us.storage.example.com/img.jpg", &patterns));
        assert!(allowed(
            "https://images.other.org/public/photo.jpg",
            &patterns
        ));
        assert!(!allowed("https://random.site.com/img.jpg", &patterns));
        assert!(!allowed(
            "https://images.other.org/private/photo.jpg",
            &patterns
        ));
    }

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain("https://example.com/path"),
            Some("example.com".to_string())
        );
        assert_eq!(
            extract_domain("http://sub.example.com:8080/path"),
            Some("sub.example.com".to_string())
        );
        assert_eq!(extract_domain("invalid"), None);
    }

    #[test]
    fn test_whitespace_in_patterns() {
        let patterns = vec!["  example.com  ".to_string()];
        assert!(allowed("https://example.com/image.jpg", &patterns));
    }

    #[test]
    fn test_empty_pattern_ignored() {
        let patterns = vec!["".to_string(), "example.com".to_string()];
        assert!(allowed("https://example.com/image.jpg", &patterns));
    }

    #[test]
    fn denied_metadata_and_mapped_addresses() {
        let open = policy(true, false);
        let patterns = vec![];
        for url in [
            "http://169.254.169.254/",
            "http://[::ffff:169.254.169.254]/",
            "http://[::ffff:a9fe:a9fe]/",
            "http://100.64.0.1/",
            "http://[fd00::1]/",
            "http://[fe80::1]/",
        ] {
            assert!(!is_source_allowed(url, &patterns, &open), "{url}");
        }
    }

    #[test]
    fn denied_obfuscated_loopback_and_names() {
        let open = policy(true, false);
        let patterns = vec![];
        for url in [
            "http://2130706433/",
            "http://0x7f.1/",
            "http://127.1/",
            "http://localhost./",
            "http://a.localhost/",
            "http://[64:ff9b::7f00:1]/",
            "http://224.0.0.1/",
            "http://user@example.com/",
        ] {
            assert!(!is_source_allowed(url, &patterns, &open), "{url}");
        }
    }

    #[test]
    fn denied_non_http_schemes() {
        let open = policy(true, false);
        let patterns = vec!["*".to_string()];
        assert!(!is_source_allowed("file:///etc/passwd", &patterns, &open));
        assert!(!is_source_allowed("ftp://example.com/x", &patterns, &open));
        assert!(!is_source_allowed(
            "gopher://example.com/",
            &patterns,
            &open
        ));
    }

    #[test]
    fn allowed_public_addresses() {
        let open = policy(true, false);
        let patterns = vec![];
        assert!(is_source_allowed("http://93.184.216.34/", &patterns, &open));
        assert!(is_source_allowed(
            "http://[2606:4700::1111]/",
            &patterns,
            &open
        ));
    }

    #[test]
    fn allow_private_networks_allows_loopback() {
        let patterns = vec![];
        assert!(is_source_allowed(
            "http://127.0.0.1/",
            &patterns,
            &policy(true, true)
        ));
    }

    #[test]
    fn path_prefix_rejects_encoded_dot_segments() {
        let patterns = vec!["host/bucket/*".to_string()];
        assert!(!allowed("http://host/bucket/%2e%2e/x", &patterns));
        assert!(!allowed("http://host/bucket/..%2Fother/x", &patterns));
    }

    #[test]
    fn encoded_slash_still_matches_prefix() {
        let patterns = vec!["images.example.com/o/*".to_string()];
        assert!(allowed(
            "https://images.example.com/o/cimg%2Fa%2Fb.jpg",
            &patterns
        ));
    }
}
