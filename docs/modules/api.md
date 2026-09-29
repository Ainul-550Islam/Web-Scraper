# Module `api`

Source: [`src/api.rs`](../../src/api.rs) · unit tests: [`src/api/tests.rs`](../../src/api/tests.rs)

Public surface (43 items):

- `struct ApiError`
  <!-- pub struct ApiError -->
- `struct Query`
  <!-- pub struct Query -->
- `struct Page`
  <!-- pub struct Page -->
- `const PAGE_DEFAULT_LIMIT`
  <!-- pub const PAGE_DEFAULT_LIMIT: usize = 50 -->
- `const PAGE_MAX_LIMIT`
  <!-- pub const PAGE_MAX_LIMIT: usize = 200 -->
- `const PAGE_MAX_OFFSET`
  <!-- pub const PAGE_MAX_OFFSET: u64 = 1_000_000 -->
- `struct PageWindow`
  <!-- pub struct PageWindow<T> -->
- `enum JobStatus`
  <!-- pub enum JobStatus -->
- `struct JobInfo`
  <!-- pub struct JobInfo -->
- `struct JobSummary`
  <!-- pub struct JobSummary -->
- `struct RunInfo`
  <!-- pub struct RunInfo -->
- `enum SortKey`
  <!-- pub enum SortKey -->
- `enum SortDir`
  <!-- pub enum SortDir -->
- `struct ListFilter`
  <!-- pub struct ListFilter -->
- `enum Status`
  <!-- pub enum Status -->
- `enum ComponentStatus`
  <!-- pub enum ComponentStatus -->
- `struct ComponentReport`
  <!-- pub struct ComponentReport -->
- `struct HealthReport`
  <!-- pub struct HealthReport -->
- `struct QueueHealth`
  <!-- pub struct QueueHealth -->
- `struct WorkerHealth`
  <!-- pub struct WorkerHealth -->
- `struct ProxyPoolInfo`
  <!-- pub struct ProxyPoolInfo -->
- `struct ProxyEgressInfo`
  <!-- pub struct ProxyEgressInfo -->
- `struct PipelineStats`
  <!-- pub struct PipelineStats -->
- `struct PipelineTargetStats`
  <!-- pub struct PipelineTargetStats -->
- `struct StorageHealth`
  <!-- pub struct StorageHealth -->
- `struct LaneHealth`
  <!-- pub struct LaneHealth -->
- `fn path_label` — Redacted path display: basename plus one parent component. Private filesystem layout must not leak wholesale, but an operator must be able to tell `dataset.p1.jsonl` from `history.jsonl`.
  <!-- pub fn path_label(p: &std::path::Path) -> String -->
- `fn dir_size_bounded` — Non-recursive dir size over at most `cap` entries (datasets can hold unbounded part files; the scan must not).
  <!-- pub fn dir_size_bounded(dir: &std::path::Path, cap: usize) -> u64 -->
- `const HISTORY_WINDOW_LINES`
  <!-- pub const HISTORY_WINDOW_LINES: usize = 512 -->
- `fn history_window`
  <!-- pub fn history_window(path: &std::path::Path, max_lines: usize) -> Vec<Value> -->
- `fn recent_runs` — Newest `limit` valid history lines as `RunInfo` rows (file order).
  <!-- pub fn recent_runs(path: &std::path::Path, limit: usize) -> Vec<RunInfo> -->
- `fn runs_listing` — Full `/runs`-style listing shared by the admin route and the CLI: filter, deterministic sort, bounded page. Returns (window_total, page).
  <!-- pub fn runs_listing( path: &std::path::Path, -->
- `fn redact_config` — Recursively redact + annotate a config value tree. Returns the redacted value with a parallel `_source` map: per top-level key, `default` | `explicit` | `env:NAME` (raw whole-value `${VAR}` in the file), so the operator can see *where* each setting came from without us storing any extra provenance state.
  <!-- pub fn redact_config(loaded: &mut Value, raw: Option<&Value>) -->
- `fn config_inspect` — Full `/config` payload: redacted effective config + validation status + backend availability for this binary. `path` (when known) adds provenance by re-reading the raw file and diffing against the expanded value.
  <!-- pub fn config_inspect(cfg: &Config, path: Option<&std::path::Path>) -> Value -->
- `fn render_table` — Render a bounded, aligned text table (CLI default output). Cells are stringified already; no width heuristics beyond trimming to 64 chars.
  <!-- pub fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String -->
- `fn snapshot_log_line` — Compact single-line snapshot for logs (`harvest status --log`).
  <!-- pub fn snapshot_log_line(snap: &Value) -> String -->
- `struct AdminReply`
  <!-- pub struct AdminReply -->
- `struct AdminClient`
  <!-- pub struct AdminClient -->
- `fn admin_get_json` — `GET addr+path` and, for 401/403/503, surface the *server's* error message (which is safe by construction) instead of a parse error.
  <!-- pub fn admin_get_json(addr: &str, path: &str, token: Option<String>) -> Result<Value, ApiError> -->
- `fn build_query` — Build a query string from present-only params, percent-encoding values.
  <!-- pub fn build_query(params: &[(&str, Option<String>)]) -> String -->
- `const CONFIG_KNOBS` — Static operator docs for the config surface: (path, type, default, summary). Kept here (next to the API models it feeds) and pinned by a test that every path resolves in a serialized `Config::default()` — so the docs table cannot drift from the struct.
  <!-- pub const CONFIG_KNOBS: &[(&str, &str, &str, &str)] = &[ ( "name", -->
- `fn knob_lookup` — Resolve a dotted path inside a (possibly redacted) serialized config.
  <!-- pub fn knob_lookup(v: &Value, path: &str) -> Option<Value> -->
- `fn explain_config` — Rows for `harvest config explain`: docs + (when a loaded config is given) the current redacted value and whether it came from the file, the environment, or stayed at its default.
  <!-- pub fn explain_config(cfg: &Value, sources: Option<&Value>, filter: Option<&str>) -> Vec<Value> -->
