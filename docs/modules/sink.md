# Module `sink`

Source: [`src/sink.rs`](../../src/sink.rs) · unit tests: [`src/sink/tests.rs`](../../src/sink/tests.rs)

Public surface (9 items):

- `struct Record`
  <!-- pub struct Record -->
- `trait Sink`
  <!-- pub trait Sink: Send + Sync -->
- `struct NullSink`
  <!-- pub struct NullSink -->
- `struct HttpSink` — Batched webhook sink, fully off-thread: async callers only hand record JSON to an unbounded mpsc. A dedicated std thread owns everything `reqwest::blocking` related (a blocking client builds and drops its own runtime, so it must never touch a tokio worker), batches POSTs, and reports failures back through the `flush()` ack. `flush()` blocks the calling task only at end-of-run, when nothing else needs it.
  <!-- pub struct HttpSink -->
- `struct DatasetSink` — Directory-partitioned dataset delivery: `<dir>/dt=YYYY-MM-DD/part-<run>-<seq>.jsonl` — an S3-friendly layout with zero cloud deps (sync the dir with any tool).
  <!-- pub struct DatasetSink -->
- `fn build`
  <!-- pub fn build(cfg: &SinkCfg) -> Result<(std::sync::Arc<dyn Sink>, PathBuf)> -->
- `struct JsonlSink`
  <!-- pub struct JsonlSink -->
- `struct CsvSink`
  <!-- pub struct CsvSink -->
- `struct SqliteSink`
  <!-- pub struct SqliteSink -->
