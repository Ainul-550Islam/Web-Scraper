//! Tests for the SSRF guard. Resolution-dependent cases use literal IP
//! literals or the RFC 2606 `.invalid` TLD, so no test needs the network.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::*;

fn guard() -> Guard {
    Guard::new(SsrfPolicy::https_only())
}

fn v4(s: &str) -> IpAddr {
    IpAddr::V4(Ipv4Addr::from_str(s).unwrap())
}

fn v6(s: &str) -> IpAddr {
    IpAddr::V6(Ipv6Addr::from_str(s).unwrap())
}

/// Every host that must be refused, in every spelling a URL can carry.
const BLOCKED_V4: &[&str] = &[
    "127.0.0.1",       // loopback
    "127.1",           // omitted components
    "127.0.0.0",       // loopback network
    "127.255.255.255", // loopback broadcast
    "2130706433",      // decimal loopback
    "0x7f000001",      // hex loopback
    "0177.0.0.1",      // leading-zero octal
    "0.0.0.0",         // unspecified
    "0.1.2.3",         // "this network"
    "10.0.0.1",        // private
    "10.255.255.255",
    "172.16.0.1", // private
    "172.31.255.254",
    "192.168.0.1", // private
    "192.168.255.255",
    "169.254.169.254", // link-local + cloud metadata
    "169.254.0.1",
    "100.64.0.1", // carrier-grade NAT
    "100.127.255.255",
    "192.0.0.1",  // IETF protocol assignments
    "198.18.0.1", // benchmarking
    "198.19.255.255",
    "198.51.100.1", // TEST-NET-2
    "203.0.113.1",  // TEST-NET-3
    "224.0.0.1",    // multicast
    "239.255.255.255",
    "255.255.255.255", // broadcast
];

const BLOCKED_V6: &[&str] = &[
    "::1",                    // loopback
    "::",                     // unspecified
    "fe80::1",                // link-local
    "fe80::ffff:0:0",         // link-local, compressed tail
    "ff02::1",                // multicast
    "ff02::fb",               // multicast all-nodes
    "fc00::1",                // unique local
    "fd12:3456::1",           // unique local
    "fec0::1",                // site local
    "2001:db8::1",            // documentation
    "2001:0000::1",           // IETF protocol assignments
    "2001:0db8::1",           // documentation
    "2001:2::1",              // benchmarking
    "::ffff:127.0.0.1",       // IPv4-mapped loopback
    "::ffff:10.0.0.1",        // IPv4-mapped private
    "::ffff:169.254.169.254", // IPv4-mapped metadata
    "0:0:0:0:0:ffff:7f00:1",  // IPv4-mapped loopback, expanded
    "64:ff9b::7f00:1",        // NAT64 loopback
    "2002:7f00:1::",          // 6to4 loopback
    "2002:7f00:0001::",       // 6to4 loopback, explicit
];

const PUBLIC_V4: &[&str] = &[
    "8.8.8.8",
    "1.1.1.1",
    "93.184.216.34", // example.com
    "142.250.72.14", // example.com
    "223.5.5.5",
];

const PUBLIC_V6: &[&str] = &[
    "2606:2800:220:1:248:1893:25c8:1946", // example.com
    "2001:4860:4860::8888",               // Google DNS
    "2606:4700:4700::1111",               // Cloudflare DNS
];

// ------------------------------------------------------------- parsing

#[test]
fn https_public_host_with_explicit_port_is_accepted() {
    let t = guard().check_url("https://93.184.216.34/").unwrap();
    assert_eq!(t.scheme, "https");
    assert_eq!(t.host, "93.184.216.34");
    assert_eq!(t.port, 443);
    assert_eq!(t.addresses, vec![v4("93.184.216.34")]);
    assert!(t.is_https());
    assert_eq!(t.host_port(), "93.184.216.34");
    assert_eq!(t.url("/a/b"), "https://93.184.216.34/a/b");
}

#[test]
fn non_default_port_is_preserved() {
    let t = guard()
        .check_url("https://93.184.216.34:8443/path")
        .unwrap();
    assert_eq!(t.port, 8443);
    assert_eq!(t.host_port(), "93.184.216.34:8443");
}

#[test]
fn scheme_allow_list_is_https_only_by_default() {
    for raw in [
        "file:///etc/passwd",
        "gopher://93.184.216.34:6379/_",
        "ftp://93.184.216.34/",
        "javascript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "ws://93.184.216.34/",
        "wss://93.184.216.34/",
        "dict://93.184.216.34:11211/",
        "htTp://93.184.216.34/",
    ] {
        let err = guard().check_url(raw).unwrap_err();
        let parsed = url::Url::parse(raw).expect("fixture is a well-formed URL");
        assert_eq!(
            err,
            SsrfError::UnsupportedScheme(parsed.scheme().to_string()),
            "raw={raw}"
        );
    }
}

#[test]
fn scheme_is_case_insensitive_and_http_is_opt_in() {
    // A mixed-case https scheme is still https.
    assert!(guard().check_url("HTTPS://93.184.216.34/").is_ok());
    // http is refused by the https-only policy.
    assert!(guard().check_url("http://93.184.216.34/").is_err());
    let permissive = Guard::new(SsrfPolicy {
        allow_http: true,
        ..SsrfPolicy::default()
    });
    assert!(permissive.check_url("http://93.184.216.34/").is_ok());
    assert!(permissive.policy().allows_scheme("http"));
    assert!(!permissive.policy().allows_scheme("gopher"));
    assert!(permissive.policy().allows_scheme("https"));
}

#[test]
fn fragment_and_empty_host_are_refused() {
    assert_eq!(
        guard()
            .check_url("https://93.184.216.34/#frag")
            .unwrap_err(),
        SsrfError::FragmentNotAllowed
    );
    // `https:///path` is not "no host": the URL spec's special-authority
    // ignore-slashes state skips every leading slash, so the host is `path`.
    // It still has to fail closed.
    assert!(guard().check_url("https:///path").is_err());
    assert_eq!(
        guard().check_url("https://").unwrap_err(),
        SsrfError::Malformed
    );
    assert_eq!(guard().check_url("").unwrap_err(), SsrfError::Malformed);
    assert_eq!(
        guard().check_url("not a url").unwrap_err(),
        SsrfError::Malformed
    );
}

#[test]
fn oversized_url_is_refused() {
    let long = format!("https://93.184.216.34/{}", "a".repeat(5_000));
    assert_eq!(guard().check_url(&long).unwrap_err(), SsrfError::Malformed);
}

#[test]
fn trailing_dot_on_the_host_is_normalised() {
    let t = guard().check_url("https://93.184.216.34./").unwrap();
    assert_eq!(t.host, "93.184.216.34");
}

// ------------------------------------------------------------- blocked space

#[test]
fn every_blocked_v4_spelling_is_refused() {
    for host in BLOCKED_V4 {
        let raw = format!("https://{host}/");
        let err = guard().check_url(&raw).unwrap_err();
        assert!(
            matches!(err, SsrfError::BlockedAddress),
            "{host} should be refused, got {err:?}"
        );
    }
}

#[test]
fn an_octal_spelling_the_url_parser_refuses_still_fails_closed() {
    // The URL parser rejects `0o...` outright, so the string never becomes a
    // request. The important property is that it is refused, not parsed.
    assert_eq!(
        guard().check_url("https://0o177.0.0.1/").unwrap_err(),
        SsrfError::Malformed
    );
    // ...and `fold_numeric_host`, which the resolver path also uses, folds it.
    assert_eq!(
        fold_numeric_host("0o177.0.0.1"),
        Some("127.0.0.1".parse().unwrap())
    );
}

#[test]
fn every_blocked_v6_spelling_is_refused() {
    for host in BLOCKED_V6 {
        let raw = format!("https://[{host}]/");
        let err = guard().check_url(&raw).unwrap_err();
        assert!(
            matches!(err, SsrfError::BlockedAddress),
            "{host} should be refused, got {err:?}"
        );
    }
}

#[test]
fn public_addresses_are_not_refused() {
    for host in PUBLIC_V4 {
        assert!(
            guard().check_url(&format!("https://{host}/")).is_ok(),
            "{host}"
        );
    }
    for host in PUBLIC_V6 {
        assert!(
            guard().check_url(&format!("https://[{host}]/")).is_ok(),
            "{host}"
        );
    }
}

#[test]
fn every_resolved_address_must_be_public() {
    // One good and one bad answer is still refused: a DNS answer set cannot
    // smuggle in a private address behind a public one.
    assert!(!all_public(&[v4("8.8.8.8"), v4("10.0.0.1")]));
    assert!(all_public(&[v4("8.8.8.8"), v4("1.1.1.1")]));
    let empty: Vec<IpAddr> = Vec::new();
    assert!(!all_public(&empty));
    assert!(!all_public(&[v6("2001:4860:4860::8888"), v6("fe80::1")]));
    assert!(all_public(&[v6("2001:4860:4860::8888")]));
}

#[test]
fn ipv4_mapped_and_tunnelled_v6_are_unwrapped_and_rechecked() {
    assert!(is_blocked_address(v6("::ffff:127.0.0.1")));
    assert!(is_blocked_address(v6("::ffff:10.1.2.3")));
    assert!(is_blocked_address(v6("::ffff:192.168.0.1")));
    assert!(is_blocked_address(v6("64:ff9b::c0a8:1"))); // NAT64 192.168.0.1
    assert!(is_blocked_address(v6("2002:c0a8:1::"))); // 6to4 192.168.0.1
                                                      // ...but a public embedded address is fine.
    assert!(!is_blocked_address(v6("::ffff:8.8.8.8")));
    assert!(!is_blocked_address(v6("2002:0808:0808::")));
    let mapped: Ipv6Addr = "::ffff:127.0.0.1".parse().unwrap();
    assert_eq!(
        embedded_ipv4(&mapped),
        Some("127.0.0.1".parse::<Ipv4Addr>().unwrap())
    );
}

#[test]
fn ipv4_range_boundaries_are_correct() {
    let blocked = |s: &str| is_blocked_address(v4(s));
    assert!(blocked("172.16.0.1"));
    assert!(blocked("172.31.255.255"));
    assert!(!blocked("172.15.255.255"));
    assert!(!blocked("172.32.0.1"));
    assert!(blocked("100.64.0.0"));
    assert!(blocked("100.127.255.255"));
    assert!(!blocked("100.128.0.1"));
    assert!(!blocked("100.63.255.255"));
    assert!(blocked("169.254.0.0"));
    assert!(!blocked("169.253.255.255"));
    assert!(!blocked("169.255.0.1"));
    assert!(blocked("192.0.0.9"));
    assert!(!blocked("192.0.1.1"));
    assert!(blocked("198.18.0.0"));
    assert!(blocked("198.19.255.255"));
    assert!(!blocked("198.20.0.1"));
    // 223.0.0.0/4 is ordinary public space, not multicast.
    assert!(!blocked("223.255.255.255"));
    assert!(blocked("224.0.0.0"));
    assert!(blocked("239.0.0.1"));
    assert!(blocked("240.0.0.1"));
    assert!(!blocked("210.0.0.1"));
    assert!(!blocked("11.0.0.1"));
    assert!(!blocked("173.0.0.1"));
    assert!(!blocked("199.0.0.1"));
}

#[test]
fn ipv6_range_boundaries_are_correct() {
    assert!(is_blocked_address(v6("fc00::1")));
    assert!(is_blocked_address(v6("fdff:ffff::")));
    assert!(!is_blocked_address(v6("fe00::1")));
    assert!(is_blocked_address(v6("fe80::1")));
    assert!(is_blocked_address(v6("febf::1")));
    assert!(is_blocked_address(v6("ff00::")));
    assert!(!is_blocked_address(v6("2001:4860:4860::8888")));
    assert!(!is_blocked_address(v6(
        "2606:2800:220:1:248:1893:25c8:1946"
    )));
}

// ------------------------------------------------------------- host deny list

#[test]
fn deny_listed_host_names_are_refused_before_dns() {
    for host in [
        "localhost",
        "localhost.",
        "LOCALHOST",
        "foo.localhost",
        "foo.local",
        "metadata",
        "metadata.google.internal",
        "instance-data",
        "kubernetes.default",
        "kubernetes.default.svc",
        "host.docker.internal",
        "gateway.docker.internal",
        "bridge.docker.internal",
        "local",
    ] {
        let err = guard().check_url(&format!("https://{host}/")).unwrap_err();
        assert!(
            matches!(err, SsrfError::HostDenied(_)),
            "{host} should be on the deny list, got {err:?}"
        );
    }
}

#[test]
fn deny_list_helper_is_pure() {
    assert!(is_denied_host(""));
    assert!(is_denied_host("  "));
    assert!(is_denied_host("localhost"));
    assert!(is_denied_host("sub.local"));
    assert!(!is_denied_host("93.184.216.34"));
    assert!(!is_denied_host("example.com"));
    assert!(!is_denied_host("localhost.example.com"));
}

#[test]
fn a_name_that_does_not_resolve_fails_closed() {
    // RFC 2606 reserves `.invalid` so this never reaches a real resolver.
    let err = guard()
        .check_url("https://ssrf-guard-test-nonexistent-host.invalid/")
        .unwrap_err();
    assert!(matches!(err, SsrfError::DnsFailure(_)), "got {err:?}");
    assert_eq!(err.stable_code(), "dns_failure");
    assert!(
        !err.is_client_error(),
        "dns failure is an infrastructure error"
    );
    assert!(err.message().contains("did not resolve"));
}

// ------------------------------------------------------------- redirects

#[test]
fn redirect_hops_are_capped() {
    let g = Guard::new(SsrfPolicy {
        max_redirects: 2,
        ..SsrfPolicy::https_only()
    });
    // Hop 0 and hop 1 are allowed; hop 2 hits the cap before it is followed.
    assert!(g.check_redirect("https://93.184.216.34/", 0).is_ok());
    assert!(g.check_redirect("https://93.184.216.34/", 1).is_ok());
    assert_eq!(
        g.check_redirect("https://93.184.216.34/", 2).unwrap_err(),
        SsrfError::RedirectCap
    );
    assert_eq!(
        g.check_redirect("https://93.184.216.34/", 99).unwrap_err(),
        SsrfError::RedirectCap
    );
}

#[test]
fn a_redirect_onto_an_internal_host_is_refused() {
    let g = guard();
    for target in [
        "https://127.0.0.1/",
        "https://10.0.0.5/",
        "https://169.254.169.254/latest/meta-data/",
        "https://[::1]/",
        "https://localhost/",
        "https://metadata/",
    ] {
        let err = g.check_redirect(target, 0).unwrap_err();
        assert!(
            matches!(
                err,
                SsrfError::RedirectBlocked(_)
                    | SsrfError::HostDenied(_)
                    | SsrfError::BlockedAddress
            ),
            "{target} should be refused, got {err:?}"
        );
    }
}

#[test]
fn a_redirect_to_a_disallowed_scheme_is_refused() {
    let g = guard();
    assert_eq!(
        g.check_redirect("file:///etc/passwd", 0).unwrap_err(),
        SsrfError::UnsupportedScheme("file".to_string())
    );
    assert_eq!(
        g.check_redirect("http://93.184.216.34/", 0).unwrap_err(),
        SsrfError::UnsupportedScheme("http".to_string())
    );
}

#[test]
fn follow_accepts_a_public_target_and_refuses_the_rest() {
    let g = Guard::new(SsrfPolicy {
        max_redirects: 2,
        ..SsrfPolicy::https_only()
    });
    assert!(g.follow("https://93.184.216.34/next", 0).is_ok());
    assert_eq!(g.follow("", 0).unwrap_err(), SsrfError::Malformed);
    assert_eq!(
        g.follow(&"x".repeat(2_049), 0).unwrap_err(),
        SsrfError::Malformed
    );
    assert!(g.follow("https://127.0.0.1/", 0).is_err());
    assert!(g.follow("https://93.184.216.34/", 2).is_err());
}

#[test]
fn a_redirect_chain_that_ends_on_the_metadata_host_is_refused() {
    let g = guard();
    // hop 0: public. hop 1: the metadata address, spelled as an IPv4-mapped
    // IPv6 so the check has to unwrap it.
    assert!(g.follow("https://93.184.216.34/a", 0).is_ok());
    let err = g
        .follow("https://[::ffff:169.254.169.254]/latest/meta-data/", 1)
        .unwrap_err();
    assert!(
        matches!(
            err,
            SsrfError::RedirectBlocked(_) | SsrfError::BlockedAddress
        ),
        "got {err:?}"
    );
}

// ------------------------------------------------------------- numeric folding

#[test]
fn numeric_spellings_fold_to_the_address_a_browser_would_use() {
    assert_eq!(
        fold_numeric_host("127.0.0.1"),
        Some("127.0.0.1".parse().unwrap())
    );
    assert_eq!(
        fold_numeric_host("2130706433"),
        Some("127.0.0.1".parse().unwrap())
    );
    assert_eq!(
        fold_numeric_host("0x7f000001"),
        Some("127.0.0.1".parse().unwrap())
    );
    assert_eq!(
        fold_numeric_host("0o177.0.0.1"),
        Some("127.0.0.1".parse().unwrap())
    );
    // `inet_aton` semantics: `127.1` and `127.0.1` both land inside 127/8.
    // The exact octets are an `inet_aton` detail; the security property is
    // that whatever they expand to is still loopback.
    assert!(is_blocked_address(
        fold_numeric_host("127.1").unwrap().into()
    ));
    assert!(is_blocked_address(
        fold_numeric_host("127.0.1").unwrap().into()
    ));
    assert_eq!(fold_numeric_host("127.1").unwrap().octets()[0], 127);
    assert_eq!(
        fold_numeric_host("0.0.0.0"),
        Some("0.0.0.0".parse().unwrap())
    );
    assert_eq!(fold_numeric_host("example.com"), None);
    assert_eq!(fold_numeric_host("127.0.0.1.nip.io"), None);
    assert_eq!(fold_numeric_host("0xZZZZZZZZ"), None);
    assert_eq!(fold_numeric_host("99999999999999999999"), None);
    assert_eq!(fold_numeric_host("1.2.3.4.5"), None);
    assert_eq!(fold_numeric_host(""), None);
}

// ------------------------------------------------------------- error surface

#[test]
fn error_codes_are_stable_and_do_not_leak_addresses() {
    let err = guard().check_url("https://10.0.0.1/").unwrap_err();
    assert_eq!(err.stable_code(), "blocked_address");
    assert_eq!(err.message(), "the host resolved to a non-public address");
    assert!(!err.message().contains("10.0.0.1"));
    assert!(err.is_client_error());

    let err = guard().check_url("https://localhost/").unwrap_err();
    assert_eq!(err.stable_code(), "host_denied");
    assert_eq!(
        err.message(),
        "host \"localhost\" is not an allowed public destination"
    );

    let err = guard().check_url("gopher://8.8.8.8/").unwrap_err();
    assert_eq!(err.stable_code(), "unsupported_scheme");
    assert_eq!(err, SsrfError::UnsupportedScheme("gopher".to_string()));
    assert_eq!(err.message(), "scheme \"gopher\" is not allowed");

    assert_eq!(SsrfError::RedirectCap.stable_code(), "redirect_cap");
    assert_eq!(
        SsrfError::FragmentNotAllowed.stable_code(),
        "fragment_not_allowed"
    );
    assert_eq!(SsrfError::EmptyHost.stable_code(), "empty_host");
    assert_eq!(SsrfError::Malformed.stable_code(), "malformed_url");
    assert_eq!(
        SsrfError::RedirectScheme("ftp".to_string()).stable_code(),
        "redirect_scheme"
    );
    assert_eq!(
        SsrfError::RedirectBlocked(3).stable_code(),
        "redirect_blocked"
    );
    assert!(SsrfError::RedirectCap.is_client_error());
    assert!(!SsrfError::DnsFailure("x".to_string()).is_client_error());
}

#[test]
fn policy_defaults_are_production_safe() {
    let p = SsrfPolicy::default();
    assert_eq!(p.max_redirects, 5);
    assert!(!p.allow_http);
    assert_eq!(SsrfPolicy::production().max_redirects, 5);
    assert!(!SsrfPolicy::production().allow_http);
    assert_eq!(Guard::default().policy().max_redirects, 5);
}

#[test]
fn check_url_with_policy_helper_works() {
    let t = check_url_with_policy("https://93.184.216.34/", &SsrfPolicy::https_only()).unwrap();
    assert_eq!(t.host, "93.184.216.34");
    assert!(check_url_with_policy("https://127.0.0.1/", &SsrfPolicy::https_only()).is_err());
}
