# Module `alerts`

Source: [`src/alerts.rs`](../../src/alerts.rs) · unit tests: [`src/alerts/tests.rs`](../../src/alerts/tests.rs)

Public surface (6 items):

- `enum Severity`
  <!-- pub enum Severity -->
- `struct AlertDetail`
  <!-- pub struct AlertDetail -->
- `fn evaluate_detailed` — Full evaluation; `evaluate` is the reason-string projection of this.
  <!-- pub fn evaluate_detailed(cfg: &Config, status: &str, summary: &Summary) -> Vec<AlertDetail> -->
- `fn evaluate` — Reason strings; empty = healthy (or alerts disabled).
  <!-- pub fn evaluate(cfg: &Config, status: &str, summary: &Summary) -> Vec<String> -->
- `struct AlertState`
  <!-- pub struct AlertState -->
- `struct AlertLedger`
  <!-- pub struct AlertLedger -->
