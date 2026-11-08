//! Classify a provider endpoint as Local, Lan or Cloud (SPEC §6.3).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

use crate::providers::Locality;

/// `confirmed_hosts` are hosts the user has said they control (case-insensitive).
pub fn classify(endpoint: &str, confirmed_hosts: &[String]) -> Locality {
    let Ok(url) = Url::parse(endpoint) else {
        // Unparseable endpoints are treated as the least trusted option.
        return Locality::Cloud;
    };
    let confirmed = |h: &str| confirmed_hosts.iter().any(|c| c.eq_ignore_ascii_case(h));
    match url.host() {
        Some(Host::Domain(d)) => {
            let d = d.to_ascii_lowercase();
            if d == "localhost" || d.ends_with(".localhost") {
                Locality::Local
            } else if confirmed(&d) {
                Locality::Lan { confirmed: true }
            } else if d.ends_with(".local") || d.ends_with(".lan") || d.ends_with(".home.arpa") {
                Locality::Lan { confirmed: false }
            } else {
                Locality::Cloud
            }
        }
        Some(Host::Ipv4(ip)) => classify_ip(IpAddr::V4(ip), confirmed(&ip.to_string())),
        Some(Host::Ipv6(ip)) => classify_ip(IpAddr::V6(ip), confirmed(&ip.to_string())),
        None => Locality::Cloud,
    }
}

fn classify_ip(ip: IpAddr, confirmed: bool) -> Locality {
    if ip.is_loopback() {
        Locality::Local
    } else if is_private(ip) {
        Locality::Lan { confirmed }
    } else if confirmed {
        // A user may confirm a public IP they control (e.g. a home server); still Lan, not Local.
        Locality::Lan { confirmed: true }
    } else {
        Locality::Cloud
    }
}

fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => is_private_v6(v6),
    }
}

fn is_private_v4(ip: Ipv4Addr) -> bool {
    // RFC1918 + link-local + CGNAT (Tailscale and similar overlays use 100.64/10).
    let o = ip.octets();
    ip.is_private() || ip.is_link_local() || (o[0] == 100 && (o[1] & 0xC0) == 64)
}

fn is_private_v6(ip: Ipv6Addr) -> bool {
    let seg = ip.segments()[0];
    // Unique local fc00::/7 and link-local fe80::/10.
    (seg & 0xfe00) == 0xfc00 || (seg & 0xffc0) == 0xfe80
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(url: &str) -> Locality {
        classify(url, &[])
    }

    #[test]
    fn loopback_is_local() {
        assert_eq!(c("http://localhost:11434"), Locality::Local);
        assert_eq!(c("http://127.0.0.1:11434"), Locality::Local);
        assert_eq!(c("http://[::1]:11434"), Locality::Local);
        assert_eq!(c("http://ollama.localhost"), Locality::Local);
    }

    #[test]
    fn private_ranges_are_unconfirmed_lan() {
        for u in [
            "http://192.168.1.20:11434",
            "http://10.0.0.5:11434",
            "http://172.20.1.1",
            "http://169.254.3.3",
            "http://100.100.1.1",
            "http://[fd00::1]:11434",
            "http://[fe80::1]",
            "http://gpu-box.local:11434",
            "http://gpu.lan",
        ] {
            assert_eq!(c(u), Locality::Lan { confirmed: false }, "{u}");
        }
    }

    #[test]
    fn confirmed_hosts_become_confirmed_lan() {
        let hosts = vec!["192.168.1.20".to_string(), "Ollama.Example.org".to_string()];
        assert_eq!(
            classify("http://192.168.1.20:11434", &hosts),
            Locality::Lan { confirmed: true }
        );
        assert_eq!(
            classify("https://ollama.example.org", &hosts),
            Locality::Lan { confirmed: true }
        );
        assert_eq!(
            classify("http://8.8.8.8", &["8.8.8.8".into()]),
            Locality::Lan { confirmed: true }
        );
    }

    #[test]
    fn everything_else_is_cloud() {
        assert_eq!(c("https://api.anthropic.com"), Locality::Cloud);
        assert_eq!(c("http://8.8.8.8"), Locality::Cloud);
        assert_eq!(c("http://172.32.0.1"), Locality::Cloud);
        assert_eq!(c("http://[2001:db8::1]"), Locality::Cloud);
        assert_eq!(c("not a url"), Locality::Cloud);
        assert_eq!(c("file:///tmp/x"), Locality::Cloud);
    }
}
