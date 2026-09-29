# Module `export`

Source: [`src/export.rs`](../../src/export.rs) · Source: [`src/export/bundle.rs`](../../src/export/bundle.rs) · Source: [`src/export/xlsx.rs`](../../src/export/xlsx.rs) · Source: [`src/export/pdf.rs`](../../src/export/pdf.rs) · Source: [`src/export/parquet.rs`](../../src/export/parquet.rs) · Source: [`src/export/xml.rs`](../../src/export/xml.rs)

Unit tests: [`src/export/tests.rs`](../../src/export/tests.rs)

Public surface (65 items):

**src/export.rs**

- `enum ExportFormat`
  <!-- pub enum ExportFormat -->
- `fn parse_formats` — Parse a `--format` value list ("csv,jsonl" or repeated flags).
  <!-- pub fn parse_formats(values: &[String]) -> anyhow::Result<Vec<ExportFormat>> -->
- `struct ExportOpts`
  <!-- pub struct ExportOpts -->
- `struct FileEntry`
  <!-- pub struct FileEntry -->
- `struct Manifest`
  <!-- pub struct Manifest -->
- `struct BundleInfo`
  <!-- pub struct BundleInfo -->
- `struct Report`
  <!-- pub struct Report -->
- `fn run`
  <!-- pub fn run(o: &ExportOpts) -> Result<Report> -->
- `const PROGRESS_FILE`
  <!-- pub const PROGRESS_FILE: &str = ".export-progress.json"; -->
- `struct FileCheck`
  <!-- pub struct FileCheck -->
- `struct Verification`
  <!-- pub struct Verification -->
- `fn verify` — Check an export directory against its manifest. Read-only; safe to run
  <!-- pub fn verify(dir: &Path, key: Option<&Path>) -> Result<Verification> -->

**src/export/bundle.rs**

- `const COPY_CHUNK` — Bytes copied per read: small enough to keep memory flat, large enough to
  <!-- pub const COPY_CHUNK: usize = 64 * 1024; -->
- `const ZIP32_MAX` — ZIP's 32-bit size ceiling. Refused explicitly rather than truncated.
  <!-- pub const ZIP32_MAX: u64 = u32::MAX as u64; -->
- `struct Crc32`
  <!-- pub struct Crc32(u32); -->
- `fn crc32` — One-file convenience wrapper around [`Crc32`].
  <!-- pub fn crc32(bytes: &[u8]) -> u32 -->
- `struct ZipEntry`
  <!-- pub struct ZipEntry -->
- `struct ZipReport`
  <!-- pub struct ZipReport -->
- `fn safe_archive_path` — Reject unsafe archive member names. Refusal — not silent sanitizing — is
  <!-- pub fn safe_archive_path(name: &str) -> Result<String> -->
- `fn checksums_txt` — `sha256sum`-compatible text block, sorted by member name.
  <!-- pub fn checksums_txt(entries: &[(String, String)]) -> String -->
- `fn bundle_manifest` — The bundle manifest: every member, its size and digest, plus the dataset
  <!-- pub fn bundle_manifest( -->
- `enum Member`
  <!-- pub enum Member<'a> -->
- `fn write_zip_from_files` — Stream every member from disk into one stored-mode ZIP at `out`.
  <!-- pub fn write_zip_from_files(out: &Path, files: &[(String, PathBuf)]) -> Result<ZipReport> -->
- `fn write_zip` — Write a stored-mode ZIP at `out` from `members`, in order. Duplicates and
  <!-- pub fn write_zip(out: &Path, members: &[(String, Member<'_>)]) -> Result<ZipReport> -->

**src/export/xlsx.rs**

- `const EXCEL_MAX_ROWS` — Excel's own limits; our defaults sit well inside them.
  <!-- pub const EXCEL_MAX_ROWS: usize = 1_048_576; -->
- `const EXCEL_MAX_COLS`
  <!-- pub const EXCEL_MAX_COLS: usize = 16_384; -->
- `const EXCEL_MAX_CELL_CHARS`
  <!-- pub const EXCEL_MAX_CELL_CHARS: usize = 32_767; -->
- `struct XlsxOpts`
  <!-- pub struct XlsxOpts -->
- `enum Cell`
  <!-- pub enum Cell -->
- `struct Sheet`
  <!-- pub struct Sheet -->
- `struct XlsxReport`
  <!-- pub struct XlsxReport -->
- `fn write_xlsx` — Write `sheet` to `path` as a valid `.xlsx`.
  <!-- pub fn write_xlsx(path: &Path, sheet: &Sheet, opts: &XlsxOpts) -> Result<XlsxReport> -->
- `fn col_letter` — Column index (0-based) to SpreadsheetML letters: 0 → A, 25 → Z, 26 → AA.
  <!-- pub fn col_letter(idx: usize) -> String -->
- `fn safe_sheet_name` — Excel workbook sheet names: ≤ 31 chars, no `[]:*?/\`, non-empty.
  <!-- pub fn safe_sheet_name(name: &str) -> String -->
- `fn date_serial` — Days since Excel's 1899-12-30 epoch (the 1900 date system, with the
  <!-- pub fn date_serial(y: i32, m: u32, d: u32) -> i64 -->
- `fn parse_iso_date` — Parse a strict `YYYY-MM-DD` (optionally `THH:MM…`) prefix.
  <!-- pub fn parse_iso_date(s: &str) -> Option<(i32, u32, u32)> -->

**src/export/pdf.rs**

- `struct PdfOpts`
  <!-- pub struct PdfOpts -->
- `struct PdfTable`
  <!-- pub struct PdfTable -->
- `struct PdfSection`
  <!-- pub struct PdfSection -->
- `struct PdfDoc`
  <!-- pub struct PdfDoc -->
- `struct PdfReport`
  <!-- pub struct PdfReport -->
- `struct Overflow`
  <!-- pub struct Overflow; -->
- `fn write_pdf`
  <!-- pub fn write_pdf(path: &Path, doc: &PdfDoc, opts: &PdfOpts) -> Result<PdfReport> -->
- `fn pdf_escape` — Escape the three characters that are special inside a PDF string literal,
  <!-- pub fn pdf_escape(s: &str) -> String -->
- `fn pdf_date` — `D:YYYYMMDDHHMMSS` in UTC — deterministic for a given timestamp.
  <!-- pub fn pdf_date(ts: i64) -> String -->
- `fn clip` — Clip to `max` characters, marking truncation with an ellipsis.
  <!-- pub fn clip(s: &str, max: usize) -> String -->
- `fn wrap` — Greedy word wrap at `width` characters (paragraphs are report prose, so
  <!-- pub fn wrap(s: &str, width: usize) -> Vec<String> -->

**src/export/parquet.rs**

- `enum ParquetType`
  <!-- pub enum ParquetType -->
- `struct ParquetColumn`
  <!-- pub struct ParquetColumn -->
- `enum ParquetValue`
  <!-- pub enum ParquetValue -->
- `struct ParquetTable`
  <!-- pub struct ParquetTable -->
- `struct ParquetOpts`
  <!-- pub struct ParquetOpts -->
- `struct ParquetReport`
  <!-- pub struct ParquetReport -->
- `fn write_parquet` — Write `table` to `path` as a Parquet file.
  <!-- pub fn write_parquet( -->
- `fn value_from_json` — Map a JSON value onto a Parquet value. Strings stay strings (including
  <!-- pub fn value_from_json(v: &Value) -> ParquetValue -->
- `fn infer_columns` — Infer typed columns from JSON rows, scanning at most `scan` rows per column.
  <!-- pub fn infer_columns(columns: &[String], rows: &[Value], scan: usize) -> Vec<ParquetColumn> -->
- `fn table_from_rows` — Build a [`ParquetTable`] from JSON rows and column names, inferring types.
  <!-- pub fn table_from_rows(columns: &[String], rows: &[Value], scan: usize) -> ParquetTable -->

**src/export/xml.rs**

- `fn escape_xml` — Escape `&`, `<`, `>`, `"`, `'` and strip characters XML 1.0 cannot carry.
  <!-- pub fn escape_xml(s: &str) -> String -->
- `fn is_xml_name` — True when `name` is a legal XML element/attribute `Name`: starts with a
  <!-- pub fn is_xml_name(name: &str) -> bool -->
- `fn sanitize_name` — Suggest a usable element name for a data-derived column. Returns `None`
  <!-- pub fn sanitize_name(name: &str) -> Option<String> -->
- `struct XmlOpts`
  <!-- pub struct XmlOpts -->
- `struct XmlReport`
  <!-- pub struct XmlReport -->
- `fn type_of` — JSON type label used in the `type` attribute (stable vocabulary).
  <!-- pub fn type_of(v: &Value) -> &'static str -->
- `fn text_of` — Render one value as escaped element text. Arrays/objects become compact
  <!-- pub fn text_of(v: &Value, max_chars: usize) -> (String, bool) -->
- `fn write_xml` — Write `rows` (a JSON array of objects) as one XML document at `path`.
  <!-- pub fn write_xml( -->

