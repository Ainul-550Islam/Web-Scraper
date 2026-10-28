# Review hardening and compatibility notes

This review fixes concrete failures, adds regression coverage and verifies the
configured local workflows. It is not a claim that every possible bug has
been eliminated or that an arbitrary production deployment has been tested.

## Collection and distributed workers

- Global and target page limits are now **pre-request admission limits**, not
  post-fetch discovery hints. A limit of 3 cannot fetch 41 queued pages. Failed
  page attempts consume admission; robots/sitemap exchanges and bounded retry/
  redirect attempts are auxiliary HTTP requests, not additional record pages.
- Recovered checkpoint jobs, Redis jobs, cached links and redirects must stay
  within the approved origin, credentials, target include/exclude and depth.
  Explicit sitemap origins are approved; untrusted child documents cannot add
  arbitrary new origins. Sitemap expansion is bounded before materialization.
- Redirects are followed manually, at most five hops. Cross-origin redirects
  fail closed rather than forwarding credentials or bypassing the original
  approval. Use the final HTTPS URL as the configured start URL when a source
  upgrades HTTP to HTTPS.
- Robots absence is recognized only for 404/410. Refusals, network failures,
  malformed/excessive policies and 5xx fail closed. Rules support wildcard,
  final `$`, allow/disallow tie precedence, case-insensitive agent matching,
  percent-encoded octets, UTF-8 paths and query matching. Unknown records do
  not split user-agent groups. A bounded regex matcher avoids repeated
  wildcard backtracking. Parsing supports 512 KiB, 8192 lines, 4096 rules and
  8192 bytes per rule; unsupported complexity refuses, never silently permits.
- Every engine robots/page/redirect exchange is paced. Token state, spacing
  and live refusal floors survive scheduled/control run boundaries in a
  shared RuntimeSlot. Explicit delays above one hour refuse the host instead
  of shortening the site's restriction. Long Retry-After is not clamped into
  an earlier request.
- Redis uses server-time atomic token buckets, including fractional rates,
  burst, minimum spacing, crawl-delay, refusal floors and shared page/record
  admission. A ConnectionManager handles connection replacement without
  silently replaying non-idempotent HTTP delivery. Leases/heartbeats are owned
  and expired jobs are acked rather than left permanently inflight.
- Remote Redis requires `rediss://`, platform CA and hostname verification;
  `#insecure` is forbidden. Plain Redis is loopback-only. Remote PostgreSQL
  requires `sslmode=require`, verified native CA/hostname, and has no plaintext
  fallback. Install private deployment CAs into the platform trust store;
  do not disable verification. Local test fixtures use temporary private CAs.
- HTML is static, not a browser renderer. Production HTML extraction has a
  100000-tag complexity cap, 4096 matches/field, 1 MiB text/field and 8 MiB
  extracted-record cap. JSON is hashed as canonical JSON, not parsed as HTML,
  and JSON fields are byte-checked before cloning. Distributed JSON targets
  use the same typed path/extraction behavior.

## Lifecycle, output and delivery

- SIGINT and SIGTERM stop work cooperatively. Request send/body/redirect,
  retry, pacing and storage-lock waits observe cancellation. Synchronous
  export/analysis work has a scoped cancellation check. Deadline cleanup
  awaits physical work; it does not free task slots while detached work is
  still writing. Failed/cancelled sibling artifacts are revoked.
- Configured output mutations, snapshots and control jobs are serialized.
  Crawl snapshots are immutable job-specific copies, so later scheduler runs
  cannot silently change an earlier download.
- HTTP/PostgreSQL sink threads are owned and joined. The final barrier uses
  an **aggregate 15s** delivery deadline (10s operation maximum), not a fresh
  30s allowance for every buffered record. PostgreSQL sets a 5s server-side
  statement/lock timeout. HTTP POST redirects are refused and failed POSTs are
  sticky; they are never automatically replayed. A remote system may have
  accepted a request before an acknowledgement was lost: no general exactly-
  once guarantee is made. Filesystem/kernel stalls cannot be reliably killed
  by application timeouts.
- SQLite WAL is checkpointed before an output snapshot. CSV columns are
  configuration-stable across targets/reopening, CR/quotes are handled and
  spreadsheet formula strings are neutralized.
- Downloads pin a regular file descriptor, enforce size/SHA and an approved
  root, and hold a two-download concurrency permit through socket writing.
  Registry admission is atomic and a producing job does not evict its own
  just-completed siblings. Unix file helpers reject symlinks and special files
  using directory-relative nofollow opens. Non-Unix protection detects
  ordinary symlinks but has not been proven race-safe for Windows reparse
  points; Windows/macOS builds were not executed in this Linux review.
- Bound in-memory registries are not a filesystem retention policy. Rotate
  original output/history/events and prune private control-root files after
  they are no longer needed; configure disk/tmpfs quotas. Interrupted CLI
  export staging is intentionally retained for recovery, not silently lost.

## Export/source compatibility

- Export byte limits apply **while writing** CSV/JSONL/adapters, manifest,
  signatures and ZIP, not only after a huge file already landed. The write
  accounting is conservative: archive headers/backpatch writes and repeated
  representations consume budget. CLI hard write cap: 256 MiB; API command
  limits can be stricter. Inputs are capped at 64 MiB combined, 100000 records,
  1024 payload columns, 256-byte flattened names and 2000000 table cells.
  A delivery has at most 256 files. Split larger source ranges explicitly.
- Output is locked, source/output collisions are refused, reused parts must
  match the **current expected content**, reads/hashes are growth-bounded,
  manifests/signatures are checked against pinned bytes and completion
  publication is atomic. Failed refreshes do not retain a stale completion
  marker. A checksums-only folder cannot claim to deliver records.
- ZIP-only requests include the canonical CSV/JSONL payload pair; they do not
  produce a metadata-only archive. Source labels contain basenames rather
  than leaking host paths. Reserved metadata field names use `fields:` in
  flattened delivery columns; literal `.`, `\\` and `:` are escaped, preserving
  nested/literal-key collisions and empty objects. Sparse adapter rows do not
  allocate every absent key for every row.
- Managed source preparation supports JSONL, CSV, SQLite and recursively
  discovered dataset day partitions. More than 64 source parts is an explicit
  error, not silent truncation. Descendant symlinks are refused. Temporary
  normalized inputs are private, outside the final output directory, and
  cleaned on success/error. AI sampling uses all partitions as needed too.
- **CSV is intrinsically lossy**: it has no original JSON type information,
  and formula-neutralization cannot always be reversed. CSV input fields are
  preserved as strings (empty cells become null); use JSONL/SQLite when exact
  JSON typing matters. XLSX enforces Excel's header/row and UTF-16 string
  limits; a long cell may be truncated. XML invalid control characters and
  unsuitable column names remain subject to its documented format limits.
  PDF is a bounded sample/report, not a complete Unicode archive. Preserve
  canonical JSONL alongside these presentation formats.

## Console and deployment

- Node 24 LTS is the reference environment (22.12+ LTS is also allowed).
  Vite/Vitest and vulnerable transitive dependencies were upgraded. Tokens
  remain memory-only; endpoints reject credentials/query/fragments, fetches
  omit ambient cookies, mutations are not replayed, deadlines include body
  reads and response/download sizes are bounded.
- Polling generations/refresh errors are isolated, manual pause does not
  issue another request, visibility catch-up respects pause, and changing a
  source key clears old data. Deep redaction includes query credentials,
  arrays, DSNs and auth strings. Vite allows local/explicit hosts and the
  preview `.e2b.app` domain, not every arbitrary hostname. Set
  `VITE_ALLOWED_HOST` to a comma-separated deliberate allowlist when needed.
- Docker build contexts exclude credentials/build outputs. Compose runs
  `serve`, handles SIGTERM, requires real admin/Grafana secrets, and Prometheus
  uses a private runtime credentials file rather than literal `$VAR` YAML.
  The Compose starter config binds the engine on `0.0.0.0:9678` only inside
  its service network, stores mutable paths under `/var/lib/harvest`, and is
  paired with explicit Git ignores for `ops/harvest/config.json` and
  `admin-token`. The console runtime is unprivileged nginx. Its static root
  now serves `/` through the configured index; the prior `try_files $uri =404`
  rejected it, while unknown file paths still return 404. Security headers
  are repeated in the JS/CSS/HTML location because nginx does not inherit
  parent `add_header` directives after that location defines `Cache-Control`.
  Reverse proxies preserve `$http_host` (including port) for same-origin
  authenticated mutations. Kubernetes manifests remain a configuration
  surface, not a tested cluster rollout. Systemd runs the supported engine
  CLI and uses an EnvironmentFile.

The engine and console images were built, then the full Compose stack
(engine, console, Prometheus, Grafana) was started with health checks. The
engine/console auth path, Prometheus scrape (`harvest` target `up`), Grafana
Prometheus datasource and provisioned dashboard were verified. This is a
local Compose smoke, not a production rollout or Kubernetes execution. See
`REVIEW_BN.md` for exact counts, versions and explicit limitations. Reference
protocol: [RFC 9309](https://www.rfc-editor.org/rfc/rfc9309.html).
