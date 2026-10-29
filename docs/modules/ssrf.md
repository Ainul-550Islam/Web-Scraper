# Module `ssrf`

Source: [`src/ssrf.rs`](../../src/ssrf.rs) · unit tests: [`src/ssrf/tests.rs`](../../src/ssrf/tests.rs)

The server-side request forgery guard. It is the first thing that touches a
customer-supplied URL and the last thing that touches a redirect hop; nothing
in the fetch path is allowed to bypass it.

Public surface:

- `struct Guard` — `new`, `policy`, `check_url`, `parse`, `resolve_and_check`,
  `check_redirect`, `follow`
- `struct SsrfPolicy` — `max_redirects`, `allow_http`, `schemes`,
  `allow_private_targets`; `https_only`, `production`, `blocks`, `allows_scheme`
- `struct GuardedTarget` — `url`, `host_port`, `is_https`
- `struct ParsedUrl` — `scheme`, `host`, `port`
- `enum SsrfError` — `stable_code`, `message`, `is_client_error`
- `fn is_denied_host`, `fn fold_numeric_host`, `fn is_blocked_address`,
  `fn embedded_ipv4`, `fn all_public`, `fn check_url_with_policy`

What is refused, unconditionally:

* Any scheme other than `https`, or `http` unless the policy opts in.
* `file:`, `gopher:`, `ftp:`, `data:`, `javascript:` — the scheme list is an
  allow-list, not a block-list.
* Hosts that are `metadata`, `localhost`, or otherwise denied by name.
* Literal IPv4, IPv6 and IPv4-mapped IPv6 addresses in a refused range.
* Every address a hostname resolves to. A hostname that resolves to *one*
  public address and *one* loopback address is refused: DNS answers are not
  trusted to be uniform.
* Unspecified, multicast, link-local, and the cloud metadata ranges
  (`169.254.169.254`, `192.0.0.0/24`, `198.18.0.0/15`, `203.0.113.0/24`,
  `100.64.0.0/10`, RFC 1918, `::1`, `fc00::/7`, and their IPv4-mapped forms).
* More than `max_redirects` hops. Redirects are followed by hand — the HTTP
  client is built with `redirect(Policy::none())` — so every `Location` goes
  through `check_redirect` again, on the same policy.

`allow_private_targets` is the operator's documented test/demo opt-in. It
relaxes exactly two things and nothing else: loopback and RFC 1918 space
become reachable, and `http` becomes an allowed scheme (the local demo origin
is plain HTTP). Unspecified, multicast, link-local and metadata addresses stay
refused either way. It defaults to `false`, and `Config::validate` logs a
startup warning the moment it is turned on — it is never a production setting.

Both the literal-address paths and the resolver path consult
`policy.blocks(ip)`, so a literal `127.0.0.1` and a hostname that resolves to
`127.0.0.1` are judged by the same rule.
