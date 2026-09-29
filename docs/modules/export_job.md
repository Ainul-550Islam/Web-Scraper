# Module `export_job`

Source: [`src/export_job.rs`](../../src/export_job.rs)

Unit tests: [`src/export_job/tests.rs`](../../src/export_job/tests.rs)

Public surface (9 items):

**src/export_job.rs**

- `const MAX_SOURCE_FILES` — Upper bound on source files an export job will read from a partitioned
  <!-- pub const MAX_SOURCE_FILES: usize = 64 -->
- `fn source_files` — The run's declared output files, as the export job's only permitted
  <!-- pub fn source_files(cfg: &Config) -> Result<Vec<PathBuf>, ControlError> -->
- `struct ExportJobSpec`
  <!-- pub struct ExportJobSpec -->
- `struct ProducedFile`
  <!-- pub struct ProducedFile -->
- `struct ExportResult`
  <!-- pub struct ExportResult -->
- `struct ProducedFileName`
  <!-- pub struct ProducedFileName -->
- `fn truncate_inputs` — Copy at most `max_records` JSONL lines out of `inputs` into `work_dir`.
  <!-- pub fn truncate_inputs( -->
- `fn run` — Run one export with the existing exporter and register nothing. Returns
  <!-- pub fn run( -->
- `fn verify_dir` — `harvest verify` semantics, without a signing key (the control plane never
  <!-- pub fn verify_dir(dir: &Path) -> VerificationState -->
