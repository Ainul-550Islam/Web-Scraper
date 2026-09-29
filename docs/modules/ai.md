# Module `ai`

Source: [`src/ai.rs`](../../src/ai.rs) · Source: [`src/ai/schema.rs`](../../src/ai/schema.rs)

Public surface (34 items):

**src/ai.rs**

- `const REDACTION_MARK` — Marker substituted for the value of any field whose name looks sensitive.
  <!-- pub const REDACTION_MARK: &str = "***"; -->
- `const SENSITIVE_FIELD_HINTS` — Field-name fragments that are redacted before a request is built. Matched
  <!-- pub const SENSITIVE_FIELD_HINTS: [&str; 12] = [ -->
- `enum AiTask`
  <!-- pub enum AiTask -->
- `struct AiField`
  <!-- pub struct AiField -->
- `struct AiRequest`
  <!-- pub struct AiRequest -->
- `struct RawSuggestion`
  <!-- pub struct RawSuggestion -->
- `struct AiResponse`
  <!-- pub struct AiResponse -->
- `enum AiError`
  <!-- pub enum AiError -->
- `trait AiProvider` — Provider interface. Implementations are injected; nothing in this crate
  <!-- pub trait AiProvider: Send + Sync -->
- `struct DisabledProvider`
  <!-- pub struct DisabledProvider; -->
- `struct FailingProvider`
  <!-- pub struct FailingProvider; -->
- `struct DeterministicProvider`
  <!-- pub struct DeterministicProvider; -->
- `struct AiCfg`
  <!-- pub struct AiCfg -->
- `struct ProviderStatus`
  <!-- pub struct ProviderStatus -->
- `struct RedactionReport`
  <!-- pub struct RedactionReport -->
- `struct ApplyReport`
  <!-- pub struct ApplyReport -->
- `struct AiClient` — Budgeted, redacting, validating wrapper around an [`AiProvider`].
  <!-- pub struct AiClient -->
- `fn redact` — Replace the values of fields whose *name* looks sensitive. Applied before a
  <!-- pub fn redact(cfg: &AiCfg, fields: Vec<AiField>) -> (Vec<AiField>, RedactionReport) -->
- `fn apply_suggestions` — Merge an AI outcome into a record. Rules, in order:
  <!-- pub fn apply_suggestions( -->
- `fn normalization_for` — Build a normalization suggestion for a messy value, naming an existing
  <!-- pub fn normalization_for(field: &str, value: &str) -> Option<NormalizationSuggestion> -->
- `fn candidate` — Turn suggestion candidates into typed extraction candidates (used by the
  <!-- pub fn candidate(field: &str, value: Value, confidence: f64, rationale: &str) -> ExtractionCandidate -->

**src/ai/schema.rs**

- `enum Provenance`
  <!-- pub enum Provenance -->
- `struct Confidence`
  <!-- pub struct Confidence(f64); -->
- `enum Band`
  <!-- pub enum Band -->
- `enum SuggestedType`
  <!-- pub enum SuggestedType -->
- `struct FieldSuggestion`
  <!-- pub struct FieldSuggestion -->
- `struct ExtractionCandidate`
  <!-- pub struct ExtractionCandidate -->
- `struct NormalizationSuggestion`
  <!-- pub struct NormalizationSuggestion -->
- `struct ValidationResult`
  <!-- pub struct ValidationResult -->
- `enum ReviewAction`
  <!-- pub enum ReviewAction -->
- `struct Thresholds`
  <!-- pub struct Thresholds -->
- `struct ProviderMetadata`
  <!-- pub struct ProviderMetadata -->
- `enum AiState`
  <!-- pub enum AiState -->
- `struct AiOutcome`
  <!-- pub struct AiOutcome -->

