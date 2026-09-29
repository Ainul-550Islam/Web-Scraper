# Module `control`

Source: [`src/control.rs`](../../src/control.rs)

Unit tests: [`src/control/tests.rs`](../../src/control/tests.rs)

Public surface (35 items):

**src/control.rs**

- `const COMMAND_BODY_LIMIT` — Hard cap on a control-plane request body (64 KiB). Big enough for an AI
  <!-- pub const COMMAND_BODY_LIMIT: u64 = 65_536 -->
- `const MAX_RECORDS_CEILING` — Ceilings a command may never exceed. They are deliberately coarse and
  <!-- pub const MAX_RECORDS_CEILING: u64 = 5_000_000 -->
- `const MAX_EXPORT_BYTES_CEILING`
  <!-- pub const MAX_EXPORT_BYTES_CEILING: u64 = 1_073_741_824; // 1 GiB -->
- `const MAX_AI_INPUT_CEILING`
  <!-- pub const MAX_AI_INPUT_CEILING: u64 = 4_194_304; // 4 MiB -->
- `const MAX_TIMEOUT_SECS`
  <!-- pub const MAX_TIMEOUT_SECS: u64 = 3_600 -->
- `const MAX_AI_FIELDS`
  <!-- pub const MAX_AI_FIELDS: usize = 512 -->
- `const MAX_AI_FIELD_VALUE_BYTES`
  <!-- pub const MAX_AI_FIELD_VALUE_BYTES: usize = 4_096 -->
- `const MAX_CONCURRENT_JOBS` — Bounded concurrency/queue limits for the control plane. Rejecting cleanly
  <!-- pub const MAX_CONCURRENT_JOBS: usize = 8 -->
- `const MAX_QUEUE_DEPTH`
  <!-- pub const MAX_QUEUE_DEPTH: usize = 64 -->
- `const MAX_DOWNLOAD_BYTES` — Hard cap for one artifact download (256 MiB).
  <!-- pub const MAX_DOWNLOAD_BYTES: u64 = 268_435_456 -->
- `const IDEMPOTENCY_CAP` — Retained idempotency keys. Oldest-key eviction; memory is O(cap).
  <!-- pub const IDEMPOTENCY_CAP: usize = 256 -->
- `struct CommandId`
  <!-- pub struct CommandId(pub String) -->
- `struct ControlJobId`
  <!-- pub struct ControlJobId(pub String) -->
- `struct ArtifactId`
  <!-- pub struct ArtifactId(pub String) -->
- `enum Operation`
  <!-- pub enum Operation -->
- `struct CommandLimits`
  <!-- pub struct CommandLimits -->
- `struct CrawlRequest`
  <!-- pub struct CrawlRequest -->
- `struct ExportRequest`
  <!-- pub struct ExportRequest -->
- `enum AiSource`
  <!-- pub enum AiSource -->
- `struct AiFieldInput`
  <!-- pub struct AiFieldInput -->
- `struct AiRequest`
  <!-- pub struct AiRequest -->
- `struct VerifyRequest`
  <!-- pub struct VerifyRequest -->
- `struct ArtifactInspectRequest`
  <!-- pub struct ArtifactInspectRequest -->
- `enum CommandSpec`
  <!-- pub enum CommandSpec -->
- `const LOCAL_PROVIDER_ID` — The only provider id this build can resolve locally.
  <!-- pub const LOCAL_PROVIDER_ID: &str = "deterministic-local" -->
- `const DISABLED_PROVIDER_ID` — The provider id used when AI is switched off.
  <!-- pub const DISABLED_PROVIDER_ID: &str = "disabled" -->
- `struct ControlCommand`
  <!-- pub struct ControlCommand -->
- `enum ControlErrorKind`
  <!-- pub enum ControlErrorKind -->
- `struct ControlError`
  <!-- pub struct ControlError -->
- `enum Idempotency`
  <!-- pub enum Idempotency -->
- `struct IdempotencyRecord`
  <!-- pub struct IdempotencyRecord -->
- `struct IdempotencyIndex`
  <!-- pub struct IdempotencyIndex -->
- `fn is_safe_id` — Conservative id charset for ids that appear in audit rows, artifact
  <!-- pub fn is_safe_id(s: &str) -> bool -->
- `fn validate_idempotency_key` — Idempotency keys are client-supplied: validate before they reach a map.
  <!-- pub fn validate_idempotency_key(raw: Option<&str>) -> Result<Option<String>, ControlError> -->
- `fn sha256_hex`
  <!-- pub fn sha256_hex(bytes: &[u8]) -> String -->
