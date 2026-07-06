//! Shared SSRF guard for webhook target URLs.
//!
//! One implementation, used by three call sites:
//! - create/update validation ([`validate_target_url`]),
//! - delivery-time re-validation ([`target_url_is_blocked`]).
//!
//! The guard rejects any host that is, or resolves to, a private, loopback,
//! link-local, unspecified, broadcast, multicast, or internal-mapped address.
//! Hostnames are resolved via DNS at call time so that a name pointing at an
//! internal address is caught at creation, and so that re-resolving at delivery
//! time defeats DNS-rebinding. Resolution failure or an empty result fails
//! closed: a host we cannot vet is treated as private and never contacted.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::error::ApiError;

/// Validates that `url` is an absolute http(s) URL whose host is routable and
/// not private.
///
/// When `allow_private_targets` is true (dev / test) the private-host check is
/// skipped entirely, but the scheme/host structural checks still apply.
pub async fn validate_target_url(url: &str, allow_private_targets: bool) -> Result<(), ApiError> {
    let url = url.trim();

    if url.is_empty() {
        return Err(ApiError::InvalidInput {
            message: "target_url must not be empty".into(),
        });
    }

    let parsed = reqwest::Url::parse(url).map_err(|_| ApiError::InvalidInput {
        message: "target_url must be an absolute URL with http or https scheme".into(),
    })?;

    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(ApiError::InvalidInput {
            message: "target_url must be an absolute URL with http or https scheme".into(),
        });
    }

    let host = parsed.host_str().ok_or_else(|| ApiError::InvalidInput {
        message: "target_url must include a host".into(),
    })?;

    if !allow_private_targets && is_private_webhook_host(host).await {
        return Err(ApiError::InvalidInput {
            message: "target_url host must resolve to a public address (not localhost, private, \
                      loopback, link-local, unspecified, or unresolvable)"
                .into(),
        });
    }

    Ok(())
}

/// Delivery-time re-validation: returns true when `url` must NOT be contacted
/// under the current policy.
///
/// Fails closed: when `allow_private_targets` is false, an unparseable URL, a
/// missing host, or a host that (re-)resolves to a private address all return
/// true so the delivery is skipped. When `allow_private_targets` is true the
/// check is bypassed and this always returns false.
pub async fn target_url_is_blocked(url: &str, allow_private_targets: bool) -> bool {
    if allow_private_targets {
        return false;
    }

    let Ok(parsed) = reqwest::Url::parse(url.trim()) else {
        return true;
    };

    let Some(host) = parsed.host_str() else {
        return true;
    };

    is_private_webhook_host(host).await
}

/// Returns true if `host` is, or resolves to, a private/internal address.
///
/// `localhost` and the reserved `.localhost` TLD are rejected up front as
/// defense-in-depth (RFC 6761), independent of any resolver state. Literal IPs
/// are classified directly; other hostnames are resolved via DNS.
pub async fn is_private_webhook_host(host: &str) -> bool {
    let normalized = normalize_host(host);

    if normalized == "localhost" || normalized.ends_with(".localhost") {
        return true;
    }

    if let Ok(ip) = normalized.parse::<IpAddr>() {
        return is_private_webhook_ip(ip);
    }

    host_resolves_private(&normalized).await
}

/// Resolves `host` via DNS and reports whether any resolved address is private.
///
/// Fails closed: a resolution error or an empty address set is treated as
/// private so an unvettable host is never contacted.
async fn host_resolves_private(host: &str) -> bool {
    match tokio::net::lookup_host((host, 0u16)).await {
        Ok(addrs) => {
            let mut resolved_any = false;
            for addr in addrs {
                resolved_any = true;
                if is_private_webhook_ip(addr.ip()) {
                    return true;
                }
            }
            !resolved_any
        }
        Err(_) => true,
    }
}

/// Lowercases the host and strips a trailing FQDN dot and IPv6 brackets so that
/// bracketed literals such as `[::1]` (as returned by `Url::host_str`) classify
/// correctly.
fn normalize_host(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('.');

    let unbracketed = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);

    unbracketed.to_ascii_lowercase()
}

fn is_private_webhook_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_private_webhook_ipv4(ip),
        IpAddr::V6(ip) => is_private_webhook_ipv6(ip),
    }
}

fn is_private_webhook_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.octets()[0] == 0
        || ip.octets()[0] >= 224
}

fn is_private_webhook_ipv6(ip: Ipv6Addr) -> bool {
    // IPv4-mapped (`::ffff:a.b.c.d`) and IPv4-compatible (`::a.b.c.d`) addresses
    // route to the embedded IPv4 host, so vet that IPv4 directly. Without this,
    // `::ffff:169.254.169.254` bypasses the guard and reaches the cloud metadata
    // endpoint. `to_ipv4` returns the embedded address for both forms (and for
    // `::` / `::1`, which the IPv4 predicate then classifies as private).
    if let Some(v4) = ip.to_ipv4() {
        return is_private_webhook_ipv4(v4);
    }

    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ((ip.segments()[0] & 0xfe00) == 0xfc00)
        || ((ip.segments()[0] & 0xffc0) == 0xfe80)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn literal_private_ipv4_is_blocked() {
        for host in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.169.254",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
        ] {
            assert!(
                is_private_webhook_host(host).await,
                "{host} must be blocked"
            );
        }
    }

    #[tokio::test]
    async fn literal_public_ipv4_is_allowed() {
        for host in ["8.8.8.8", "1.1.1.1", "93.184.216.34"] {
            assert!(
                !is_private_webhook_host(host).await,
                "{host} must be allowed"
            );
        }
    }

    #[tokio::test]
    async fn ipv4_mapped_ipv6_metadata_is_blocked() {
        // The headline bypass: an IPv4-mapped IPv6 pointing at the cloud
        // metadata endpoint and at loopback must be rejected.
        assert!(is_private_webhook_host("::ffff:169.254.169.254").await);
        assert!(is_private_webhook_host("::ffff:127.0.0.1").await);
        assert!(is_private_webhook_host("::ffff:10.0.0.1").await);
        // Bracketed form as produced by URL parsing.
        assert!(is_private_webhook_host("[::ffff:169.254.169.254]").await);
    }

    #[tokio::test]
    async fn ipv4_mapped_public_ipv6_is_allowed() {
        assert!(!is_private_webhook_host("::ffff:8.8.8.8").await);
    }

    #[tokio::test]
    async fn literal_private_ipv6_is_blocked() {
        for host in ["::1", "::", "fc00::1", "fd12:3456::1", "fe80::1", "ff02::1"] {
            assert!(
                is_private_webhook_host(host).await,
                "{host} must be blocked"
            );
        }
    }

    #[tokio::test]
    async fn public_ipv6_is_allowed() {
        assert!(!is_private_webhook_host("2606:4700:4700::1111").await);
    }

    #[tokio::test]
    async fn localhost_names_are_blocked() {
        assert!(is_private_webhook_host("localhost").await);
        assert!(is_private_webhook_host("LOCALHOST").await);
        assert!(is_private_webhook_host("api.localhost").await);
        assert!(is_private_webhook_host("localhost.").await);
    }

    #[tokio::test]
    async fn hostname_resolving_to_loopback_is_blocked() {
        // `localhost` resolves to loopback via /etc/hosts (offline), so the DNS
        // resolution path classifies it as private. Exercised through the
        // resolver helper directly to bypass the string special-case above.
        assert!(host_resolves_private("localhost").await);
    }

    #[tokio::test]
    async fn unresolvable_host_fails_closed() {
        // RFC 6761 reserves `.invalid`; it must never resolve, so the guard
        // fails closed and blocks it.
        assert!(host_resolves_private("nonexistent-host.invalid").await);
    }

    #[tokio::test]
    async fn allow_private_bypasses_the_check() {
        assert!(!target_url_is_blocked("http://127.0.0.1/hook", true).await);
        assert!(!target_url_is_blocked("http://localhost:9000/hook", true).await);
    }

    #[tokio::test]
    async fn target_url_blocks_private_and_fails_closed() {
        assert!(target_url_is_blocked("http://127.0.0.1/hook", false).await);
        assert!(target_url_is_blocked("http://[::ffff:169.254.169.254]/x", false).await);
        assert!(target_url_is_blocked("http://localhost/hook", false).await);

        // Unparseable URL and missing host fail closed.
        assert!(target_url_is_blocked("not a url", false).await);

        // Public target is not blocked.
        assert!(!target_url_is_blocked("https://example.com/hook", false).await);
    }
}
