# Module `ops`

Source: [`src/ops.rs`](../../src/ops.rs) · unit tests: [`src/ops/tests.rs`](../../src/ops/tests.rs)

Public surface (7 items):

- `const OPS_CAP` — Maximum retained operator events / audit records. A daemon running for months still only holds these many rows — the point is that the newest history is always on hand and memory never grows.
  <!-- pub const OPS_CAP: usize = 256 -->
- `const AUDIT_CAP`
  <!-- pub const AUDIT_CAP: usize = 128 -->
- `struct OpsEvent`
  <!-- pub struct OpsEvent -->
- `struct AuditRecord`
  <!-- pub struct AuditRecord -->
- `fn sanitize` — Clip to `limit` chars and replace anything that could forge log lines (control bytes, DEL) with '?'. Used for every caller-supplied string stored in the rings.
  <!-- pub fn sanitize(s: &str, limit: usize) -> String -->
- `struct OpsLog`
  <!-- pub struct OpsLog -->
- `struct AuditLedger`
  <!-- pub struct AuditLedger -->
