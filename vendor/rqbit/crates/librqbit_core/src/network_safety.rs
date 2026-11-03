//! Conservative destination restrictions for untrusted public torrent endpoints.
use std::net::IpAddr;

pub fn is_public_destination(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && !ip.is_multicast()
                && a != 0
                && a < 240
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (b == 18 || b == 19))
                && !(a == 192 && b == 0)
        }
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                return is_public_destination(IpAddr::V4(ip));
            }
            let s = ip.segments();
            // Exclude local, transition, documentation and non-global scopes.
            (s[0] & 0xe000) == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
        }
    }
}

pub fn validate_url(url: &url::Url) -> anyhow::Result<()> {
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https"),
        "unsupported network URL scheme"
    );
    if let Some(host) = url.host() {
        let ip = match host {
            url::Host::Ipv4(ip) => Some(IpAddr::V4(ip)),
            url::Host::Ipv6(ip) => Some(IpAddr::V6(ip)),
            _ => None,
        };
        anyhow::ensure!(
            ip.is_none_or(is_public_destination),
            "local network destination requires explicit permission"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_local_and_encoded_local_destinations() {
        for ip in [
            "127.0.0.1",
            "0.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "198.18.0.1",
            "224.0.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "2002:7f00:1::",
            "64:ff9b::7f00:1",
        ] {
            assert!(!is_public_destination(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(is_public_destination(ip.parse().unwrap()));
        }
        for url in [
            "http://127.1/announce",
            "http://2130706433/announce",
            "http://[::ffff:127.0.0.1]/announce",
        ] {
            assert!(validate_url(&url::Url::parse(url).unwrap()).is_err());
        }
    }
}
