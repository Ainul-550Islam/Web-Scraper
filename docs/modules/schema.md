# Module `schema`

Source: [`src/schema.rs`](../../src/schema.rs) · unit tests: [`src/schema/tests.rs`](../../src/schema/tests.rs)

Public surface (6 items):

- `enum Enforce`
  <!-- pub enum Enforce -->
- `enum DeclaredType`
  <!-- pub enum DeclaredType -->
- `struct Schema`
  <!-- pub struct Schema -->
- `struct ApplyReport`
  <!-- pub struct ApplyReport -->
- `fn json_schema_for` — Draft-07 for one target: types from the declared schema when present, otherwise from the field's transform chain (the inference that makes `extract` rules self-documenting). `harvest schema` prints this.
  <!-- pub fn json_schema_for(target: &TargetCfg) -> Value -->
- `fn append_line` — Quarantine writer for `coerce`-mode logging parity (append-only; one fs open per record is acceptable at quarantining rates — failures are).
  <!-- pub fn append_line(path: &Path, line: &str) -> anyhow::Result<()> -->
