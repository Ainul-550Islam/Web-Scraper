//! SSRF protection for every user-supplied URL.
//!
//! A scrape API is a remote-code-execution-shaped surface: the caller names a
//! host and the server dials it. Every URL that enters the system — from a
//! `POST /v1/scrape` body or a redirect the fetcher follows — passes through
//! [`Guard::check_url`] first.
//!
//! The check is deliberately boring and complete:
//!
//! * only `http` and `https` are accepted, and only with an explicit host;
//! * the host is resolved once, and **every** returned address must be public
//!   before the request is allowed;
//! * the literal host name is also screened against a deny list, so a
//!   `.localhost`-style name cannot slip through a resolver that answers
//!   differently than expected;
//! * numeric IPv4 spellings (decimal, hex, octal, dotted-quad) are folded to
//!   their address before the range test, so `http://127.1/` and
//!   `http://0x7f000001/` cannot bypass it;
//! * IPv4-mapped IPv6 (`::ffff:127.0.0.1`) and 6to4 (`2002::/16`) addresses
//!   are unwrapped and re-checked in the IPv4 space they encode;
//! * **every redirect hop** re-enters the same check, and the hop count is
//!   capped, so a public URL cannot bounce the engine onto an internal one.
//!
//! See `docs/modules/ssrf.md`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::str::FromStr;

use url::Url;

/// How a URL failed the SSRF policy. The HTTP layer maps each variant to a
/// stable machine code; nothing here leaks internal hostnames or addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsrfError {
    /// The URL could not be parsed at all.
    Malformed,
    /// Empty host (`http:///path`, `http://`).
    EmptyHost,
    /// Scheme outside the allow-list.
    UnsupportedScheme(String),
    /// A fragment was supplied. Fragments never reach the server and are a
    /// common way to make two URLs look different while pointing at the same
    /// resource.
    FragmentNotAllowed,
    /// The literal host name is on the deny list.
    HostDenied(String),
    /// DNS lookup failed. Fail closed: an unresolvable host is refused.
    DnsFailure(String),
    /// The host resolved, but at least one address is not public.
    BlockedAddress,
    /// The redirect target failed the same checks.
    RedirectBlocked(u32),
    /// Too many redirects.
    RedirectCap,
    /// The redirect target was not an `http(s)` URL.
    RedirectScheme(String),
}

impl SsrfError {
    pub fn stable_code(&self) -> &'static str {
        match self {
            SsrfError::Malformed => "malformed_url",
            SsrfError::EmptyHost => "empty_host",
            SsrfError::UnsupportedScheme(_) => "unsupported_scheme",
            SsrfError::FragmentNotAllowed => "fragment_not_allowed",
            SsrfError::HostDenied(_) => "host_denied",
            SsrfError::DnsFailure(_) => "dns_failure",
            SsrfError::BlockedAddress => "blocked_address",
            SsrfError::RedirectBlocked(_) => "redirect_blocked",
            SsrfError::RedirectCap => "redirect_cap",
            SsrfError::RedirectScheme(_) => "redirect_scheme",
        }
    }

    pub fn message(&self) -> String {
        match self {
            SsrfError::Malformed => "the URL could not be parsed".to_string(),
            SsrfError::EmptyHost => "the URL has no host".to_string(),
            SsrfError::UnsupportedScheme(s) => format!("scheme {s:?} is not allowed"),
            SsrfError::FragmentNotAllowed => {
                "fragments are not allowed in request URLs".to_string()
            }
            SsrfError::HostDenied(h) => format!("host {h:?} is not an allowed public destination"),
            SsrfError::DnsFailure(h) => format!("the host {h:?} did not resolve"),
            SsrfError::BlockedAddress => "the host resolved to a non-public address".to_string(),
            SsrfError::RedirectBlocked(hop) => {
                format!("redirect hop {hop} pointed at a non-public address")
            }
            SsrfError::RedirectCap => "too many redirects".to_string(),
            SsrfError::RedirectScheme(s) => {
                format!("redirect used scheme {s:?}, which is not allowed")
            }
        }
    }

    /// `true` when the failure is the caller's fault (bad URL, blocked
    /// target) rather than an infrastructure problem.
    pub fn is_client_error(&self) -> bool {
        !matches!(self, SsrfError::DnsFailure(_))
    }
}

/// Where a URL points, after the policy has accepted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardedTarget {
    pub scheme: String,
    /// Lower-cased host name, as the caller spelled it.
    pub host: String,
    pub port: u16,
    /// The addresses the host resolved to. Callers that can pin a connection
    /// should use these, which removes the check-then-connect gap.
    pub addresses: Vec<IpAddr>,
}

impl GuardedTarget {
    pub fn url(&self, path: &str) -> String {
        format!("{}://{}{}", self.scheme, self.host_port(), path)
    }

    pub fn host_port(&self) -> String {
        if self.port == default_port(&self.scheme) {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    pub fn is_https(&self) -> bool {
        self.scheme == "https"
    }
}

fn default_port(scheme: &str) -> u16 {
    if scheme == "https" {
        443
    } else {
        80
    }
}

/// Policy knobs. All values are bounded; [`Default`] is the production set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsrfPolicy {
    /// Maximum number of redirects the fetcher will follow.
    pub max_redirects: u32,
    /// `http` is refused unless explicitly enabled.
    pub allow_http: bool,
    /// Scheme allow-list; `https` is always required to be present.
    pub schemes: Vec<String>,
    /// Operator opt-in for the documented demo/test mode. Loopback and
    /// RFC1918 targets become reachable, but unspecified, multicast,
    /// link-local and metadata addresses stay refused. Never enable this in
    /// production: the prompt's SSRF rule is unconditional.
    pub allow_private_targets: bool,
}

impl Default for SsrfPolicy {
    fn default() -> Self {
        Self {
            max_redirects: 5,
            allow_http: false,
            schemes: vec!["https".to_string(), "http".to_string()],
            allow_private_targets: false,
        }
    }
}

impl SsrfPolicy {
    pub fn https_only() -> Self {
        Self {
            allow_http: false,
            ..Self::default()
        }
    }

    /// Production policy: https only, five hops.
    pub fn production() -> Self {
        Self {
            max_redirects: 5,
            allow_http: false,
            schemes: vec!["https".to_string()],
            allow_private_targets: false,
        }
    }

    /// `true` when `ip` must be refused under this policy. The demo/test
    /// opt-in relaxes loopback and RFC1918 space; nothing else changes.
    pub fn blocks(&self, ip: IpAddr) -> bool {
        if self.allow_private_targets && is_demo_reachable(&ip) {
            return false;
        }
        is_blocked_address(ip)
    }

    pub fn allows_scheme(&self, scheme: &str) -> bool {
        if scheme == "https" {
            return true;
        }
        if scheme == "http" {
            return self.allow_http;
        }
        self.schemes.iter().any(|s| s == scheme)
    }
}

/// Host names that are always refused before DNS is consulted. Screening the
/// literal name costs nothing and removes a whole class of resolver surprises.
const DENY_HOSTS: &[&str] = &[
    "localhost",
    "local",
    "localhost.localdomain",
    "ip6-localhost",
    "ip6-loopback",
    "metadata",
    "metadata.google.internal",
    "instance-data",
    "kubernetes.default",
    "kubernetes.default.svc",
    "host.docker.internal",
    "gateway.docker.internal",
    "bridge.docker.internal",
    "ccm-internal",
    "proxied",
];

/// The guard. Cheap to share: it holds no per-request state.
#[derive(Debug, Clone)]
pub struct Guard {
    policy: SsrfPolicy,
}

impl Default for Guard {
    fn default() -> Self {
        Self::new(SsrfPolicy::default())
    }
}

impl Guard {
    pub fn new(policy: SsrfPolicy) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> &SsrfPolicy {
        &self.policy
    }

    /// Full check for a caller-supplied URL: parse, scheme, host deny list,
    /// numeric-form folding, DNS, and range test on every answer.
    pub fn check_url(&self, raw: &str) -> Result<GuardedTarget, SsrfError> {
        let parsed = self.parse(raw)?;
        self.resolve_and_check(&parsed.host, parsed.port, &parsed.scheme)
    }

    /// Parse + policy on a URL without resolving it. Used for redirect hops
    /// that the caller resolves itself.
    pub fn parse(&self, raw: &str) -> Result<ParsedUrl, SsrfError> {
        let raw = raw.trim();
        if raw.is_empty() || raw.len() > 4096 {
            return Err(SsrfError::Malformed);
        }
        let url = Url::parse(raw).map_err(|_| SsrfError::Malformed)?;
        if url.fragment().is_some() {
            return Err(SsrfError::FragmentNotAllowed);
        }
        if !self.policy.allows_scheme(url.scheme()) {
            return Err(SsrfError::UnsupportedScheme(url.scheme().to_string()));
        }
        let host = match url.host_str() {
            Some(h) if !h.is_empty() => normalize_host(h),
            _ => return Err(SsrfError::EmptyHost),
        };
        if host.is_empty() {
            return Err(SsrfError::EmptyHost);
        }
        if is_denied_host(&host) {
            return Err(SsrfError::HostDenied(host));
        }
        // Fold the numeric spellings a browser would also fold, before DNS.
        if let Some(ip) = fold_numeric_host(&host) {
            if self.policy.blocks(IpAddr::V4(ip)) {
                return Err(SsrfError::BlockedAddress);
            }
            return Ok(ParsedUrl {
                scheme: url.scheme().to_string(),
                host: ip.to_string(),
                port: url.port_or_known_default().unwrap_or(80),
            });
        }
        // An IPv6 literal is checked directly: no resolver needed, and the
        // IPv4-mapped forms must be unwrapped and re-checked.
        if let Ok(ipv6) = Ipv6Addr::from_str(&host) {
            if self.policy.blocks(IpAddr::V6(ipv6)) {
                return Err(SsrfError::BlockedAddress);
            }
            return Ok(ParsedUrl {
                scheme: url.scheme().to_string(),
                host: format!("[{host}]"),
                port: url.port_or_known_default().unwrap_or(80),
            });
        }
        Ok(ParsedUrl {
            scheme: url.scheme().to_string(),
            host,
            port: url.port_or_known_default().unwrap_or(80),
        })
    }

    /// Resolve `host` and require that **every** answer is public.
    pub fn resolve_and_check(
        &self,
        host: &str,
        port: u16,
        scheme: &str,
    ) -> Result<GuardedTarget, SsrfError> {
        let host = normalize_host(host);
        if host.is_empty() {
            return Err(SsrfError::EmptyHost);
        }
        if is_denied_host(&host) {
            return Err(SsrfError::HostDenied(host));
        }
        // An IPv6 literal: check it and skip the resolver entirely.
        if let Ok(ipv6) = Ipv6Addr::from_str(&host) {
            if self.policy.blocks(IpAddr::V6(ipv6)) {
                return Err(SsrfError::BlockedAddress);
            }
            return Ok(GuardedTarget {
                scheme: scheme.to_string(),
                host: format!("[{host}]"),
                port,
                addresses: vec![IpAddr::V6(ipv6)],
            });
        }
        if let Some(ip) = fold_numeric_host(&host) {
            if self.policy.blocks(IpAddr::V4(ip)) {
                return Err(SsrfError::BlockedAddress);
            }
            return Ok(GuardedTarget {
                scheme: scheme.to_string(),
                host: ip.to_string(),
                port,
                addresses: vec![IpAddr::V4(ip)],
            });
        }

        let addrs: Vec<IpAddr> = match (host.as_str(), port).to_socket_addrs() {
            Ok(iter) => iter.map(|s| s.ip()).collect(),
            Err(_) => return Err(SsrfError::DnsFailure(host)),
        };
        if addrs.is_empty() {
            return Err(SsrfError::DnsFailure(host));
        }
        for ip in &addrs {
            if self.policy.blocks(*ip) {
                return Err(SsrfError::BlockedAddress);
            }
        }
        Ok(GuardedTarget {
            scheme: scheme.to_string(),
            host,
            port,
            addresses: addrs,
        })
    }

    /// Check a redirect target against the current hop count.
    ///
    /// Returns `Err(RedirectCap)` once the hop budget is spent, so a caller
    /// loop cannot turn a cap into an infinite follow.
    pub fn check_redirect(&self, raw: &str, hop: u32) -> Result<ParsedUrl, SsrfError> {
        if hop >= self.policy.max_redirects {
            return Err(SsrfError::RedirectCap);
        }
        let parsed = self.parse(raw)?;
        // Re-enter the same resolution the original URL got.
        match self.resolve_and_check(&parsed.host, parsed.port, &parsed.scheme) {
            Ok(target) => Ok(ParsedUrl {
                scheme: target.scheme,
                host: target.host,
                port: target.port,
            }),
            Err(SsrfError::BlockedAddress) => Err(SsrfError::RedirectBlocked(hop + 1)),
            Err(e) => Err(e),
        }
    }

    /// Follow-list helper: given the next `Location` header value, validate it
    /// and return the parsed target. `hop` is the number of redirects already
    /// followed (0 for the first hop).
    pub fn follow(&self, location: &str, hop: u32) -> Result<ParsedUrl, SsrfError> {
        let location = location.trim();
        if location.is_empty() || location.len() > 2048 {
            return Err(SsrfError::Malformed);
        }
        // A relative Location is resolved against the current URL by the
        // client; the service layer always hands us absolute URLs.
        self.check_redirect(location, hop)
    }
}

/// A URL that passed parse + policy, ready for resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUrl {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

/// Normalise a host as the URL parser hands it to us: strip whitespace, a
/// trailing root dot, and the brackets the `url` crate keeps around IPv6
/// literals (`[::1]`).
fn normalize_host(raw: &str) -> String {
    let h = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    match h.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
        Some(inner) => inner.to_string(),
        None => h,
    }
}

/// `true` when the literal host name is on the deny list.
pub fn is_denied_host(host: &str) -> bool {
    let h = normalize_host(host);
    if h.is_empty() {
        return true;
    }
    DENY_HOSTS.iter().any(|d| h == *d) || h.ends_with(".localhost") || h.ends_with(".local")
}

/// Fold the numeric IPv4 spellings a browser's URL parser would also fold.
/// Returns the address when `host` is one of:
///
/// * a dotted quad (`127.0.0.1`);
/// * a dotted form with omitted trailing components (`127.1`, `127.0.1`);
/// * a single integer in any base (`2130706433`, `0x7f000001`, `0o177.0.0.1`);
/// * dotted components with their own base (`0177.0.0.1`).
///
/// Returns `None` for anything that is genuinely a name. A value that is too
/// large to be an IPv4 address is also `None`: the caller then treats the
/// host as a name, and a failed lookup fails closed.
pub fn fold_numeric_host(host: &str) -> Option<Ipv4Addr> {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    if h.is_empty() || h.len() > 64 {
        return None;
    }
    if let Ok(ip) = Ipv4Addr::from_str(&h) {
        return Some(ip);
    }
    let parts: Vec<&str> = h.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let mut values = [0u32; 4];
    for (i, part) in parts.iter().enumerate() {
        values[i] = parse_integer_literal(part)?;
    }
    let n = parts.len();
    let [a, b, c, d] = values;
    // Every component except the last must fit in a byte; the last one may
    // fill the remaining bytes.
    for v in values.iter().take(n - 1) {
        if *v > 255 {
            return None;
        }
    }
    let octets: [u8; 4] = match n {
        // A single value is a 32-bit address in host byte order: `127` is
        // `0.0.0.127` and `2130706433` is `127.0.0.1`.
        1 => [(a >> 24) as u8, (a >> 16) as u8, (a >> 8) as u8, a as u8],
        // `inet_aton`: `127.1` is `127.0.0.1`, `127.0.1` is `127.0.0.1`.
        2 => [a as u8, b as u8, b as u8, b as u8],
        3 => [a as u8, b as u8, c as u8, c as u8],
        _ => [a as u8, b as u8, c as u8, d as u8],
    };
    Some(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]))
}

fn parse_integer_literal(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() || s.len() > 16 {
        return None;
    }
    let (digits, radix) = if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))
    {
        (rest, 16)
    } else if let Some(rest) = s.strip_prefix("0o").or_else(|| s.strip_prefix("0O")) {
        (rest, 8)
    } else if let Some(rest) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
        (rest, 2)
    } else if s.len() > 1 && s.starts_with('0') && s.bytes().all(|b| b.is_ascii_digit()) {
        // A leading zero is an octal spelling in the POSIX world.
        (&s[1..], 8)
    } else if s.bytes().all(|b| b.is_ascii_digit()) {
        (s, 10)
    } else {
        return None;
    };
    let v: u64 = u64::from_str_radix(digits, radix).ok()?;
    if v > u32::MAX as u64 {
        return None;
    }
    Some(v as u32)
}

/// `true` when `ip` is not a routable public address. This is the whole
/// policy: everything that is not globally reachable is refused.
pub fn is_blocked_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(embedded) = embedded_ipv4(&v6) {
                // IPv4-mapped, IPv4-compatible (`::a.b.c.d`), and NAT64
                // (`64:ff9b::/96`) all carry an IPv4 destination.
                return is_blocked_v4(embedded);
            }
            is_blocked_v6(&v6)
        }
    }
}

/// Addresses a demo/test run may reach when the operator opts in. Unspecified,
/// multicast, link-local and metadata space is deliberately absent: the opt-in
/// relaxes private addressing, not the fail-closed rules.
fn is_demo_reachable(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            let [a, b, c, _] = o;
            a == 0
                || a == 10
                || (a == 100 && (b & 0xc0) == 0x40)
                || a == 127
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 0 && c == 2)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113)
        }
        IpAddr::V6(v6) => match embedded_ipv4(v6) {
            Some(v4) => is_demo_reachable(&IpAddr::V4(v4)),
            None => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00,
        },
    }
}

/// Unwrap the IPv4 an IPv6 address encodes, when it does.
pub fn embedded_ipv4(v6: &Ipv6Addr) -> Option<Ipv4Addr> {
    let s = v6.segments();
    // IPv4-mapped: ::ffff:a.b.c.d
    if s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0xffff {
        return Some(Ipv4Addr::new(
            (s[6] >> 8) as u8,
            (s[6] & 0xff) as u8,
            (s[7] >> 8) as u8,
            (s[7] & 0xff) as u8,
        ));
    }
    // IPv4-compatible (RFC 4291, deprecated but still parsed): ::a.b.c.d
    if s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0 {
        return Some(Ipv4Addr::new(
            (s[6] >> 8) as u8,
            (s[6] & 0xff) as u8,
            (s[7] >> 8) as u8,
            (s[7] & 0xff) as u8,
        ));
    }
    // NAT64 well-known prefix 64:ff9b::/96.
    if s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 0 && s[3] == 0 {
        return Some(Ipv4Addr::new(
            (s[6] >> 8) as u8,
            (s[6] & 0xff) as u8,
            (s[7] >> 8) as u8,
            (s[7] & 0xff) as u8,
        ));
    }
    // 6to4: the IPv4 address is in bits 16..48.
    if s[0] == 0x2002 {
        let a = u32::from(s[2]) << 16 | u32::from(s[3]);
        return Some(Ipv4Addr::from(a));
    }
    None
}

fn is_blocked_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    let [a, b, _, _] = o;
    // 0.0.0.0/8 — "this network", unspecified, and the broadcast-ish space.
    if a == 0 {
        return true;
    }
    if a == 10 {
        return true;
    }
    // 100.64.0.0/10 — carrier-grade NAT.
    if a == 100 && (b & 0xc0) == 0x40 {
        return true;
    }
    if a == 127 {
        return true;
    }
    // 169.254.0.0/16 — link-local, and the cloud metadata address.
    if a == 169 && b == 254 {
        return true;
    }
    if a == 172 && (16..=31).contains(&b) {
        return true;
    }
    if a == 192 && b == 168 {
        return true;
    }
    // 192.0.0.0/24, 192.0.2.0/24, 198.18.0.0/15, 198.51.100.0/24,
    // 203.0.113.0/24 are reserved/benchmarking; 192.0.0.9/32 is anycast.
    if a == 192 && b == 0 && o[2] == 0 {
        return true;
    }
    if a == 198 && (b == 18 || b == 19) {
        return true;
    }
    if (a == 198 && b == 51 && o[2] == 100) || (a == 203 && b == 0 && o[2] == 113) {
        return true;
    }
    // 224.0.0.0/4 — multicast.
    if a >= 224 {
        return true;
    }
    false
}

fn is_blocked_v6(v6: &Ipv6Addr) -> bool {
    let s = v6.segments();
    // Unspecified and loopback.
    if v6.is_unspecified() || v6.is_loopback() {
        return true;
    }
    // IPv4-mapped / IPv4-compatible / NAT64 / 6to4 are handled by
    // `embedded_ipv4` before we get here, but double-check the mapped form.
    if s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0xffff {
        return true;
    }
    // Discard-only.
    if s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0 {
        return true;
    }
    // Documentation range.
    if s[0] == 0x2001 && s[1] == 0x0db8 {
        return true;
    }
    // Unique-local fc00::/7.
    if (s[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    // Link-local fe80::/10.
    if (s[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    // Site-local fec0::/10 (deprecated but present in old configs).
    if (s[0] & 0xffc0) == 0xfec0 {
        return true;
    }
    // Multicast ff00::/8.
    if (s[0] & 0xff00) == 0xff00 {
        return true;
    }
    // ORCHIDv2 2001:10::/28.
    if s[0] == 0x2001 && s[1] == 0x0010 {
        return true;
    }
    // Reserved-by-IANA 2001:0000::/32 .. 2001:00ff::/32 (IETF protocol
    // assignments, benchmarking, Teredo and friends).
    if s[0] == 0x2001 && s[1] <= 0x00ff {
        return true;
    }
    // IPv4-translated 64:ff9b::/96 and the 6to4 anycast prefix.
    if s[0] == 0x0064 && s[1] == 0xff9b {
        return true;
    }
    if s[0] == 0x2002 {
        return true;
    }
    false
}

/// `true` when every address in `addrs` is public.
pub fn all_public(addrs: &[IpAddr]) -> bool {
    !addrs.is_empty() && addrs.iter().all(|ip| !is_blocked_address(*ip))
}

/// Synchronous wrapper used by non-async call sites and tests.
pub fn check_url_with_policy(raw: &str, policy: &SsrfPolicy) -> Result<GuardedTarget, SsrfError> {
    Guard::new(policy.clone()).check_url(raw)
}

#[cfg(test)]
mod tests;
