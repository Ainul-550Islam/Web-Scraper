# Module `runner`

Source: [`src/runner.rs`](../../src/runner.rs) · unit tests: [`src/runner/tests.rs`](../../src/runner/tests.rs)

Public surface (2 items):

- `fn history_entry`
  <!-- pub fn history_entry(cfg: &Config, summary: &Summary, status: &str) -> Value -->
- `fn record_history` — Append one line to the JSONL history log (creates parents as needed).
  <!-- pub fn record_history(cfg: &Config, entry: &Value) -> Result<()> -->
