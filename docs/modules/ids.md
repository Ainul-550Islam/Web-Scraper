# Module `ids`

Source: [`src/ids.rs`](../../src/ids.rs) · unit tests: [`src/ids/tests.rs`](../../src/ids/tests.rs)

Public surface (9 items):

- `struct IdError`
  <!-- pub struct IdError(pub String) -->
- `fn is_safe_key_token` — Validity for a *Redis* key token (queue prefixes). Redis namespaces conventionally use `:` — keys are not paths — so this is deliberately wider than the slug rules while still excluding every byte that could break out of a key pattern (`*`, `}`, whitespace, `;`, quotes, …).
  <!-- pub fn is_safe_key_token(s: &str) -> bool -->
- `struct RecordId`
  <!-- pub struct RecordId(String) -->
- `struct LeaseId`
  <!-- pub struct LeaseId(String) -->
- `struct Timestamp`
  <!-- pub struct Timestamp(pub i64) -->
- `struct DurationMs`
  <!-- pub struct DurationMs(pub u64) -->
- `enum DatasetPartition`
  <!-- pub enum DatasetPartition -->
- `fn civil_from_days` — Days-since-epoch → proleptic Gregorian `y/m/d` (Howard Hinnant's civil algorithm). Moved here from the sink so exports, partitions, and reports compute the UTC calendar identically.
  <!-- pub fn civil_from_days(days: i64) -> (i64, u32, u32) -->
- `struct CorrelationId`
  <!-- pub struct CorrelationId(String) -->
