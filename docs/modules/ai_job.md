# Module `ai_job`

Source: [`src/ai_job.rs`](../../src/ai_job.rs)

Unit tests: [`src/ai_job/tests.rs`](../../src/ai_job/tests.rs)

Public surface (7 items):

**src/ai_job.rs**

- `struct AiJobSpec`
  <!-- pub struct AiJobSpec -->
- `struct SuggestionView`
  <!-- pub struct SuggestionView -->
- `struct AiJobCounts`
  <!-- pub struct AiJobCounts -->
- `struct AiJobResult`
  <!-- pub struct AiJobResult -->
- `fn fields_from_run_output` — Turn the configured output into bounded [`AiField`]s.
  <!-- pub fn fields_from_run_output( -->
- `fn resolve_fields` — Resolve the job's field set.
  <!-- pub fn resolve_fields(spec: &AiJobSpec, cfg: &Config) -> Result<Vec<AiField>, String> -->
- `fn run` — Run one AI job. Never panics and never writes to the run output: the
  <!-- pub fn run(spec: &AiJobSpec, cfg: &Config) -> AiJobResult -->
