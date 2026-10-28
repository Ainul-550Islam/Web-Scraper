//! Client delivery: turn the engine's JSONL output into a verifiable
//! delivery folder — `records.csv` + canonical `records.jsonl` + a
//! `manifest.json` whose per-file sha256s let the client (or your own audit
//! bucket) prove bit-for-bit what was shipped. No cloud credentials here:
//! push the folder anywhere with rclone; the manifest travels with it.
//!
//! Design choices that matter for a paid deliverable:
//! * **Deterministic**: same input => byte-identical CSV/JSONL (stable column
//!   order: meta columns first, field keys sorted; rows keep input order).
//!   Only `generated_ts` in the manifest moves between runs.
//! * **Wide CSV with dot-flattened objects**: a json-mode target returning
//!   nested API payloads still lands in a spreadsheet-shaped file; arrays and
//!   primitives render as compact JSON inside cells.
//! * **Never silently drops**: unparsable lines are counted (`skipped_lines`)
//!   and reported, not swallowed.
//! * **Self-attesting**: `--history` pulls the run's `config_sha256` (policy
//!   hash) into the manifest, so the delivery proves which politeness config
//!   produced it.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// Meta columns every record carries; these lead the CSV header so consumers
/// can split by `run`/`target` without parsing the payload columns.
const META_COLS: [&str; 7] = [
    "run",
    "target",
    "url",
    "depth",
    "ts",
    "fingerprint",
    "egress",
];

/// Delivery formats. CSV + JSONL are the original pair; every other variant
/// is an adapter over the *same* record set, rendered in the same run, covered
/// by the same manifest — there is no second export pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat {
    Csv,
    Jsonl,
    Xlsx,
    Pdf,
    Parquet,
    Xml,
    /// ZIP bundle of every other selected format + manifest + checksums.
    Zip,
}

impl ExportFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Jsonl => "jsonl",
            Self::Xlsx => "xlsx",
            Self::Pdf => "pdf",
            Self::Parquet => "parquet",
            Self::Xml => "xml",
            Self::Zip => "zip",
        }
    }

    pub fn ext(self) -> &'static str {
        self.as_str()
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "csv" => Self::Csv,
            "jsonl" | "json-lines" => Self::Jsonl,
            "xlsx" | "excel" => Self::Xlsx,
            "pdf" => Self::Pdf,
            "parquet" => Self::Parquet,
            "xml" => Self::Xml,
            "zip" | "bundle" => Self::Zip,
            _ => return None,
        })
    }

    /// One-line factual purpose, used by the console's format picker and by
    /// `harvest export --help`.
    pub fn purpose(self) -> &'static str {
        match self {
            Self::Csv => "wide spreadsheet-shaped text, one row per record",
            Self::Jsonl => "canonical record stream (the manifest's source of truth)",
            Self::Xlsx => "typed Excel workbook for business review",
            Self::Pdf => "summary report: identity, quality, integrity, sample",
            Self::Parquet => "columnar analytics file (pandas, DuckDB, Spark)",
            Self::Xml => "system interchange with escaped, deterministic elements",
            Self::Zip => "one bundle: selected formats + manifest + checksums",
        }
    }

    /// Every format this build can write. Exposed so the console and the CLI
    /// can advertise capability without guessing.
    pub fn all() -> &'static [ExportFormat] {
        &[
            Self::Csv,
            Self::Jsonl,
            Self::Xlsx,
            Self::Pdf,
            Self::Parquet,
            Self::Xml,
            Self::Zip,
        ]
    }
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parse a `--format` value list ("csv,jsonl" or repeated flags).
pub fn parse_formats(values: &[String]) -> anyhow::Result<Vec<ExportFormat>> {
    let mut out: Vec<ExportFormat> = Vec::new();
    for raw in values {
        for part in raw.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let f = ExportFormat::parse(part).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown export format `{part}` (expected one of: {})",
                    ExportFormat::all()
                        .iter()
                        .map(|f| f.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct ExportOpts {
    pub inputs: Vec<PathBuf>,
    pub out_dir: PathBuf,
    /// Write `records.csv` / `records.jsonl` (false) or one pair per target (true).
    pub split_target: bool,
    /// Look up `config_sha256` for the delivered run(s) in this history file.
    pub history: Option<PathBuf>,
    /// HMAC key file: signs `manifest.json` (detached `manifest.json.sig`).
    /// The manifest covers every payload file's sha256, so one signature
    /// transitively attests the whole folder.
    pub key: Option<PathBuf>,
    /// Byte cap per part file when the export is chunked (0 = one file per
    /// stream). Chunking cuts at record boundaries: a part is a whole number
    /// of records; a record larger than the cap becomes a one-record part.
    /// CSV parts additionally carry their header row on top of the cap —
    /// parts are self-describing by design.
    pub max_part_bytes: u64,
    /// Keep parts already written by an interrupted run. Each finished part
    /// is validated by sha256 (and the whole run by an input fingerprint)
    /// before being reused; anything stale is regenerated. Resume changes
    /// what is *written*, never what is *in* the export.
    pub resume: bool,
    /// Formats to write. Empty means the historical default: csv + jsonl.
    pub formats: Vec<ExportFormat>,
    /// Dataset id used for the newer formats' file names
    /// (`dataset-<id>.<ext>`); with `None` they follow the existing
    /// `records.<ext>` / `<target>.<ext>` convention.
    pub dataset: Option<String>,
    /// Rows rendered into the PDF sample table (bounded).
    pub pdf_sample_rows: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileEntry {
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
    pub lines: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub v: u8,
    pub generated_ts: i64,
    pub source_files: Vec<String>,
    pub split_target: bool,
    pub records: u64,
    pub unique_urls: u64,
    pub skipped_lines: u64,
    pub runs: Vec<String>,
    pub targets: std::collections::BTreeMap<String, u64>,
    /// Latest `config_sha256` per run, from `--history` (absent if not given).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_sha256: Option<std::collections::BTreeMap<String, String>>,
    pub columns: Vec<String>,
    pub files: Vec<FileEntry>,
}

/// The ZIP bundle, when `--format zip` was requested.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleInfo {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
    /// Number of members inside the archive.
    pub members: usize,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub out_dir: String,
    pub manifest: Manifest,
    /// Path of the detached manifest signature, when `key` was given.
    pub manifest_sig: Option<String>,
    /// Formats actually written, in render order.
    pub formats: Vec<String>,
    /// Set only when `zip` was among the requested formats.
    pub bundle: Option<BundleInfo>,
}

/// One parsed delivery row: meta values + flattened payload.
type Row = (Vec<Value>, Map<String, Value>);

pub fn run(o: &ExportOpts) -> Result<Report> {
    run_bounded(o, 256 * 1024 * 1024)
}

pub(crate) fn run_bounded(o: &ExportOpts, output_cap: u64) -> Result<Report> {
    let _write_budget = crate::export_io::begin(output_cap);
    if o.inputs.is_empty() {
        bail!("export needs at least one --from file");
    }
    if o.inputs.len() > 1024 {
        bail!("export supports at most 1024 input files");
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut skipped = 0u64;
    let mut keyset: BTreeSet<String> = BTreeSet::new();

    const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
    const MAX_RECORDS: usize = 100_000;
    const MAX_COLUMNS: usize = 1024;
    let mut input_bytes = 0u64;
    for input in &o.inputs {
        // Read through a no-follow descriptor and charge the bytes actually
        // consumed, not a pathname metadata snapshot that can race replacement
        // or a growing source file.
        let remaining = MAX_INPUT_BYTES.saturating_sub(input_bytes);
        let bytes = crate::fsutil::read_bounded(input, remaining)
            .with_context(|| format!("read bounded export input {}", input.display()))?;
        input_bytes = input_bytes.saturating_add(bytes.len() as u64);
        if input_bytes > MAX_INPUT_BYTES {
            bail!("export input exceeds the 64 MiB total input cap; split the source first");
        }
        let txt = String::from_utf8(bytes)
            .with_context(|| format!("export input {} is not valid UTF-8", input.display()))?;
        for line in txt.lines() {
            crate::work_cancel::check()?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(line) {
                Ok(v @ Value::Object(_)) => {
                    let (meta, payload) = split_row(&v);
                    if payload.keys().any(|key| key.len() > 256) {
                        bail!("export field names must be at most 256 bytes");
                    }
                    keyset.extend(payload.keys().cloned());
                    if keyset.len() > MAX_COLUMNS {
                        bail!("export exceeds the 1024 field-column cap");
                    }
                    if rows.len() >= MAX_RECORDS {
                        bail!("export exceeds the 100000 record cap; split the source first");
                    }
                    rows.push((meta, payload));
                }
                Ok(_) => skipped += 1,
                Err(_) => skipped += 1,
            }
        }
    }
    if rows.is_empty() {
        bail!(
            "no valid records in {:?}",
            o.inputs
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
        );
    }

    if o.split_target
        && rows.iter().any(|r| {
            r.0[1]
                .as_str()
                .is_none_or(|t| crate::ids::TargetId::parse(t).is_err())
        })
    {
        bail!("split-target export requires safe target ids in every input record");
    }
    let mut columns: Vec<String> = META_COLS.iter().map(|s| s.to_string()).collect();
    columns.extend(keyset);
    if rows.len().saturating_mul(columns.len()) > 2_000_000 {
        bail!("export exceeds the 2000000 table-cell cap; split the source first");
    }

    crate::fsutil::reject_symlink_components(&o.out_dir)?;
    std::fs::create_dir_all(&o.out_dir)
        .with_context(|| format!("mkdir {}", o.out_dir.display()))?;
    let lock = crate::fsutil::open_within(&o.out_dir, &o.out_dir.join(".export.lock"), true)?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .context("another export already owns this output directory")?;
    let out = std::fs::canonicalize(&o.out_dir)?;
    for input in o.inputs.iter().chain(o.history.iter()).chain(o.key.iter()) {
        if std::fs::canonicalize(input)?.starts_with(&out) {
            bail!("export source/history/key must be outside the delivery directory");
        }
    }
    for entry in std::fs::read_dir(&o.out_dir)? {
        if entry?.file_type()?.is_symlink() {
            bail!("delivery directory must not contain symlink entries");
        }
    }
    // Invalidate the prior completion marker before changing payloads. A
    // failed/interrupted refresh is partial, never a stale complete delivery.
    for name in ["manifest.json", "manifest.json.sig"] {
        let path = o.out_dir.join(name);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    let staging = o.out_dir.join(".staging");
    crate::fsutil::reject_symlink_components(&staging)?;
    if !o.resume && staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    for entry in std::fs::read_dir(&staging)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            bail!("staging entries must be regular, non-symlink files");
        }
    }

    // ---- render every output part as a list of complete record lines ----
    // Payload rendering happens fully up front: parts (and their line
    // counts) are a pure function of the inputs + options, which is what
    // makes `resume` verifiable rather than best-effort.
    let mut formats = if o.formats.is_empty() {
        vec![ExportFormat::Csv, ExportFormat::Jsonl]
    } else {
        o.formats.clone()
    };
    if formats.iter().all(|f| *f == ExportFormat::Zip) {
        formats.insert(0, ExportFormat::Jsonl);
        formats.insert(0, ExportFormat::Csv);
    }
    let want = |f: ExportFormat| formats.contains(&f);

    let mut parts: Vec<(String, Vec<String>)> = Vec::new();
    let mut targets: std::collections::BTreeMap<String, u64> = Default::default();
    let mut runs: BTreeSet<String> = Default::default();
    let mut urls: BTreeSet<String> = Default::default();
    for (meta, _) in &rows {
        if targets.len() >= 1024 && !targets.contains_key(meta[1].as_str().unwrap_or("")) {
            bail!("export supports at most 1024 target identities");
        }
        targets
            .entry(meta[1].as_str().unwrap_or("").to_string())
            .and_modify(|x| *x += 1)
            .or_insert(1);
        runs.insert(meta[0].as_str().unwrap_or("").to_string());
        urls.insert(meta[2].as_str().unwrap_or("").to_string());
    }
    if o.split_target {
        let mut by_target: std::collections::BTreeMap<String, Vec<&Row>> = Default::default();
        for r in &rows {
            by_target
                .entry(r.0[1].as_str().unwrap_or("_").to_string())
                .or_default()
                .push(r);
        }
        for (t, group) in by_target {
            crate::ids::TargetId::parse(&t)
                .map_err(|_| anyhow::anyhow!("split-target export requires a safe target id"))?;
            let cols = target_columns(&group);
            let header = csv_header(&cols);
            if want(ExportFormat::Csv) {
                push_stream(
                    &mut parts,
                    &t,
                    "csv",
                    &header,
                    csv_data(&group, &cols),
                    o.max_part_bytes,
                )?;
            }
            if want(ExportFormat::Jsonl) {
                push_stream(
                    &mut parts,
                    &t,
                    "jsonl",
                    "",
                    jsonl_lines(&group),
                    o.max_part_bytes,
                )?;
            }
        }
    } else {
        let header = csv_header(&columns);
        if want(ExportFormat::Csv) {
            push_stream(
                &mut parts,
                "records",
                "csv",
                &header,
                csv_data(&rows.iter().collect::<Vec<_>>(), &columns),
                o.max_part_bytes,
            )?;
        }
        if want(ExportFormat::Jsonl) {
            push_stream(
                &mut parts,
                "records",
                "jsonl",
                "",
                jsonl_lines(&rows.iter().collect::<Vec<_>>()),
                o.max_part_bytes,
            )?;
        }
    }

    // ---- resume gate: fingerprint over inputs + options ----
    let mut planned_names = std::collections::HashSet::new();
    if parts
        .iter()
        .any(|(name, _)| !safe_payload_name(name) || !planned_names.insert(name.as_str()))
    {
        bail!(
            "generated payload filenames are unsafe or collide; rename targets/change chunk size"
        );
    }
    let fingerprint = run_fingerprint(o)?;
    let mut done: std::collections::HashMap<String, FileEntry> = Default::default();
    if o.resume {
        if let Some(prog) = load_progress(&o.out_dir) {
            if prog.fingerprint == fingerprint {
                for f in prog.files {
                    // Never read paths from a progress sidecar without validating
                    // them first (the sidecar is untrusted after a crash/copy).
                    if !safe_payload_name(&f.name) {
                        continue;
                    }
                    // trust only what validates on disk now
                    if file_matches(&o.out_dir.join(&f.name), &f) {
                        done.insert(f.name.clone(), f);
                    } else if file_matches(&staging.join(&f.name), &f) {
                        std::fs::rename(staging.join(&f.name), o.out_dir.join(&f.name))
                            .context("recover staged export part")?;
                        done.insert(f.name.clone(), f);
                    }
                }
            }
        }
    }

    let mut files: Vec<FileEntry> = Vec::new();
    for (name, lines) in &parts {
        let mut body = String::new();
        for l in lines {
            body.push_str(l);
            body.push('\n');
        }
        if let Some(prev) = done.get(name) {
            // A forged sidecar must not replace the current source data,
            // even if its own malicious payload/hash match each other.
            if prev.sha256 == digest_hex(Sha256::digest(body.as_bytes()))
                && prev.bytes == body.len() as u64
            {
                crate::export_io::charge(prev.bytes)?;
                files.push(prev.clone());
                continue;
            }
        }
        done.remove(name);
        let entry = write_with_sha(&staging, name, body.as_bytes())?;
        files.push(entry.clone());
        save_progress(
            &o.out_dir,
            &Progress {
                fingerprint: fingerprint.clone(),
                files: files.clone(),
            },
        )?;
    }
    // atomic finalize: every payload file is moved into place, THEN the
    // manifest appears, THEN signing. A directory with a manifest is a
    // complete export by construction; a crash before finalize leaves the
    // manifest absent (and the sidecar + staged parts for `resume`).
    for (name, _) in &parts {
        if !done.contains_key(name) {
            std::fs::rename(staging.join(name), o.out_dir.join(name))
                .with_context(|| format!("move {name} into place"))?;
        }
    }

    let policy = o
        .history
        .as_ref()
        .map(|h| policy_for_runs(h, &runs))
        .transpose()?;

    let generated_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // ---- format adapters -------------------------------------------------
    // Same rows, different representations. Every adapter writes into the
    // staging directory and is moved into place only once complete, so the
    // "directory is either complete or incomplete" invariant holds for the new
    // formats exactly as it does for csv/jsonl.
    let rows_ref: Vec<&Row> = rows.iter().collect();
    // Newest record timestamp in the delivery. Used as the XML metadata's
    // `generated_ts` so that file is a pure function of the records (the
    // manifest keeps the wall-clock generation time).
    let data_ts = rows
        .iter()
        .filter_map(|(meta, _)| meta.get(4).and_then(|v| v.as_i64()))
        .max()
        .unwrap_or(generated_ts);

    let mut streams: Vec<(String, Vec<String>, Vec<Value>)> = Vec::new();
    if o.split_target {
        let mut by_target: std::collections::BTreeMap<String, Vec<&Row>> = Default::default();
        for r in &rows_ref {
            by_target
                .entry(r.0[1].as_str().unwrap_or("_").to_string())
                .or_default()
                .push(*r);
        }
        for (t, group) in by_target {
            crate::ids::TargetId::parse(&t)
                .map_err(|_| anyhow::anyhow!("split-target export requires a safe target id"))?;
            let cols = target_columns(&group);
            let recs = records_json(&group, &cols);
            streams.push((t, cols, recs));
        }
    } else {
        streams.push((
            "records".to_string(),
            columns.clone(),
            records_json(&rows_ref, &columns),
        ));
    }

    let planned_files = files.len()
        + streams.len()
            * [ExportFormat::Xlsx, ExportFormat::Parquet, ExportFormat::Xml]
                .iter()
                .filter(|f| want(**f))
                .count()
        + usize::from(want(ExportFormat::Pdf))
        + usize::from(want(ExportFormat::Zip))
        + usize::from(o.key.is_some())
        + 2;
    if planned_files > 256 {
        bail!("export exceeds the 256-file delivery cap; select fewer formats/targets");
    }
    let mut written_formats: Vec<ExportFormat> = Vec::new();
    if want(ExportFormat::Csv) && !written_formats.contains(&ExportFormat::Csv) {
        written_formats.push(ExportFormat::Csv);
    }
    if want(ExportFormat::Jsonl) {
        written_formats.push(ExportFormat::Jsonl);
    }

    for fmt in [ExportFormat::Xlsx, ExportFormat::Parquet, ExportFormat::Xml] {
        if !want(fmt) {
            continue;
        }
        for (stem_base, cols, recs) in &streams {
            let stem = file_stem(o, stem_base);
            let name = format!("{stem}.{}", fmt.ext());
            let path = staging.join(&name);
            match fmt {
                ExportFormat::Xlsx => {
                    let sheet = xlsx::Sheet::from_rows(stem_base, cols, recs);
                    xlsx::write_xlsx(
                        &path,
                        &sheet,
                        &xlsx::XlsxOpts {
                            max_cols: cols.len(),
                            max_cell_chars: xlsx::EXCEL_MAX_CELL_CHARS,
                            ..Default::default()
                        },
                    )?;
                }
                ExportFormat::Parquet => {
                    let table = parquet::table_from_rows(cols, recs, 1_000);
                    parquet::write_parquet(&path, &table, &parquet::ParquetOpts::default())?;
                }
                ExportFormat::Xml => {
                    let source_count = o.inputs.len();
                    xml::write_xml(
                        &path,
                        &file_stem(o, stem_base),
                        cols,
                        recs,
                        &xml::XmlOpts::default(),
                        data_ts,
                        source_count,
                    )?;
                }
                _ => unreachable!("only adapter formats reach this loop"),
            }
            let entry = file_entry(&staging, &name)?;
            std::fs::rename(staging.join(&name), o.out_dir.join(&name))
                .with_context(|| format!("move {name} into place"))?;
            files.push(entry);
        }
        written_formats.push(fmt);
    }

    let mut manifest = Manifest {
        v: 1,
        generated_ts,
        source_files: o
            .inputs
            .iter()
            .map(|p| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            })
            .collect(),
        split_target: o.split_target,
        records: rows.len() as u64,
        unique_urls: urls.len() as u64,
        skipped_lines: skipped,
        runs: runs.clone().into_iter().collect(),
        targets,
        policy_sha256: policy,
        columns,
        files,
    };

    // The PDF cites the manifest's payload digests, so it is rendered after
    // the manifest object exists but before the manifest file is written — and
    // then added to it. Its own entry is therefore the last one in `files`.
    if want(ExportFormat::Pdf) {
        let doc = pdf_doc(
            o,
            &manifest,
            &streams,
            generated_ts,
            o.pdf_sample_rows.clamp(1, 40),
        );
        let stem = file_stem(o, "records");
        let name = format!("{stem}.pdf");
        let path = staging.join(&name);
        pdf::write_pdf(&path, &doc, &pdf::PdfOpts::default())?;
        let entry = file_entry(&staging, &name)?;
        std::fs::rename(staging.join(&name), o.out_dir.join(&name))
            .with_context(|| format!("move {name} into place"))?;
        manifest.files.push(entry);
        written_formats.push(ExportFormat::Pdf);
    }

    // checksums.txt covers every payload file (never itself, and never
    // manifest.json, which is written next and covers this file). It is written
    // after the PDF so a delivery that includes a report still hashes it.
    let payload_digests: Vec<(String, String)> = manifest
        .files
        .iter()
        .map(|f| (f.name.clone(), f.sha256.clone()))
        .collect();
    let checksums = bundle::checksums_txt(&payload_digests);
    let cs_name = "checksums.txt".to_string();
    crate::export_io::write(&staging.join(&cs_name), checksums.as_bytes())
        .context("write checksums.txt")?;
    let cs_entry = file_entry(&staging, &cs_name)?;
    std::fs::rename(staging.join(&cs_name), o.out_dir.join(&cs_name))
        .context("move checksums.txt into place")?;
    manifest.files.push(cs_entry);

    let body = serde_json::to_string_pretty(&manifest)?;
    let manifest_tmp = o.out_dir.join(".manifest.json.tmp");
    crate::export_io::write(&manifest_tmp, format!("{body}\n").as_bytes())
        .context("write manifest")?;
    std::fs::rename(manifest_tmp, o.out_dir.join("manifest.json")).context("publish manifest")?;

    let mut manifest_sig = None;
    if let Some(kp) = &o.key {
        let k =
            crate::sign::Key::load(kp).with_context(|| format!("attest key {}", kp.display()))?;
        let p = o.out_dir.join("manifest.json.sig");
        let signature = k.sign_bytes(format!("{body}\n").as_bytes())?;
        crate::export_io::write(&p, format!("{signature}\n").as_bytes())?;
        manifest_sig = Some(p.display().to_string());
    }

    // ---- optional ZIP bundle ---------------------------------------------
    // Members are the files the manifest already describes (plus the manifest,
    // its signature and an in-archive manifest that names every member). The
    // bundle is deliberately not listed in the folder manifest: a folder
    // manifest cannot describe an archive that contains it.
    let mut bundle_info = None;
    if want(ExportFormat::Zip) {
        let stem = file_stem(o, "records");
        let zip_name = format!("{stem}.zip");
        let mut members: Vec<(String, std::path::PathBuf)> = manifest
            .files
            .iter()
            .map(|f| (f.name.clone(), o.out_dir.join(&f.name)))
            .collect();
        members.push(("manifest.json".to_string(), o.out_dir.join("manifest.json")));
        if let Some(sig) = &manifest_sig {
            let sp = std::path::PathBuf::from(sig);
            if sp.exists() {
                members.push((
                    sp.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "manifest.json.sig".to_string()),
                    sp,
                ));
            }
        }
        let manifest_bytes =
            crate::fsutil::read_bounded(&o.out_dir.join("manifest.json"), 16 * 1024 * 1024)
                .context("read finalized manifest for the ZIP bundle")?;
        let manifest_digest = digest_hex(Sha256::digest(&manifest_bytes));
        let bundle_entries = members
            .iter()
            .filter_map(|(name, _)| {
                manifest
                    .files
                    .iter()
                    .find(|file| &file.name == name)
                    .map(|file| (file.name.clone(), file.bytes, file.sha256.clone()))
            })
            .chain(std::iter::once((
                "manifest.json".to_string(),
                manifest_bytes.len() as u64,
                manifest_digest,
            )))
            .collect::<Vec<_>>();
        let inner = bundle::bundle_manifest(
            &stem,
            manifest.generated_ts,
            manifest.records,
            &bundle_entries,
            manifest_sig.as_deref(),
        );
        let inner_name = "bundle-manifest.json".to_string();
        let inner_path = staging.join(&inner_name);
        crate::export_io::write(
            &inner_path,
            format!("{}\n", serde_json::to_string_pretty(&inner)?).as_bytes(),
        )
        .context("write bundle-manifest.json")?;
        members.push((inner_name.clone(), inner_path.clone()));

        let report = bundle::write_zip_from_files(&staging.join(&zip_name), &members)?;
        std::fs::rename(staging.join(&zip_name), o.out_dir.join(&zip_name))
            .with_context(|| format!("move {zip_name} into place"))?;
        bundle_info = Some(BundleInfo {
            name: zip_name,
            bytes: report.bytes,
            sha256: report.sha256.clone(),
            members: report.entries.len(),
        });
        written_formats.push(ExportFormat::Zip);
    }

    let _ = std::fs::remove_file(o.out_dir.join(PROGRESS_FILE));
    let _ = std::fs::remove_dir_all(&staging);
    Ok(Report {
        out_dir: o.out_dir.display().to_string(),
        manifest,
        manifest_sig,
        formats: written_formats
            .iter()
            .map(|f| f.as_str().to_string())
            .collect(),
        bundle: bundle_info,
    })
}

/// File stem for the newer formats: `dataset-<id>` when a dataset id is
/// configured (sanitized with the engine's key-token rules), otherwise the
/// historical `records` / `<target>` stem.
fn file_stem(o: &ExportOpts, base: &str) -> String {
    // The input target was validated before generating any format.
    match o.dataset.as_deref() {
        Some(id) if !id.trim().is_empty() => {
            let id = crate::document::safe_dataset_name(id.trim());
            if o.split_target {
                format!("{id}-{base}")
            } else {
                id
            }
        }
        _ => base.to_string(),
    }
}

/// One row as a JSON object, using the exact same column/value mapping as the
/// CSV writer — the adapters cannot disagree with the CSV about a cell.
fn records_json(rows: &[&Row], _columns: &[String]) -> Vec<Value> {
    rows.iter()
        .map(|(meta, payload)| {
            let mut m = payload.clone();
            for (i, col) in META_COLS.iter().enumerate() {
                m.insert(col.to_string(), meta[i].clone());
            }
            Value::Object(m)
        })
        .collect()
}

/// Build a [`FileEntry`] for an already-written file (binary-safe: line counts
/// are newline counts, and the sha256 is the real integrity check).
fn file_entry(dir: &Path, name: &str) -> Result<FileEntry> {
    let (sha, bytes, lines) = hash_file(&dir.join(name))?;
    Ok(FileEntry {
        name: name.to_string(),
        sha256: sha,
        bytes,
        lines,
    })
}

/// Assemble the PDF report model from what this run actually knows. Counters
/// the export input does not carry (quarantine, retries, schema drift) come
/// from `--history` when it was supplied, and are reported as unavailable when
/// it was not — the report never invents them.
fn pdf_doc(
    o: &ExportOpts,
    manifest: &Manifest,
    streams: &[(String, Vec<String>, Vec<Value>)],
    generated_ts: i64,
    sample_rows: usize,
) -> pdf::PdfDoc {
    let mut sections: Vec<pdf::PdfSection> = Vec::new();

    let mut delivery = pdf::PdfSection {
        heading: "Delivery".to_string(),
        ..Default::default()
    };
    delivery
        .rows
        .push(("records".to_string(), manifest.records.to_string()));
    delivery
        .rows
        .push(("unique urls".to_string(), manifest.unique_urls.to_string()));
    delivery
        .rows
        .push(("columns".to_string(), manifest.columns.len().to_string()));
    delivery.rows.push((
        "skipped input lines".to_string(),
        manifest.skipped_lines.to_string(),
    ));
    delivery
        .rows
        .push(("generated".to_string(), iso_ts(generated_ts)));
    delivery.rows.push((
        "sources".to_string(),
        manifest.source_files.len().to_string(),
    ));
    sections.push(delivery);

    let mut runs = pdf::PdfSection {
        heading: "Runs".to_string(),
        ..Default::default()
    };
    for r in manifest.runs.iter().take(8) {
        runs.rows.push((r.clone(), "included".to_string()));
    }
    if manifest.runs.len() > 8 {
        runs.rows.push((
            "…".to_string(),
            format!("{} further run(s)", manifest.runs.len() - 8),
        ));
    }
    sections.push(runs);

    let mut targets = pdf::PdfSection {
        heading: "Targets".to_string(),
        ..Default::default()
    };
    for (t, n) in manifest.targets.iter().take(12) {
        targets.rows.push((t.clone(), format!("{n} record(s)")));
    }
    if manifest.targets.len() > 12 {
        targets.rows.push((
            "…".to_string(),
            format!("{} further target(s)", manifest.targets.len() - 12),
        ));
    }
    sections.push(targets);

    // Quality: from the run history when `--history` was supplied.
    let mut quality = pdf::PdfSection {
        heading: "Data quality".to_string(),
        ..Default::default()
    };
    match o.history.as_ref().and_then(|h| history_counters(h, &manifest.runs)) {
        Some(rows) => quality.rows = rows,
        None => quality.paragraphs.push(
            "Run-level quality counters (quarantined records, schema drift, retries,              robots denials) are not part of the export input and no --history file was              supplied, so they are not reported here rather than guessed. Re-run with              `harvest export --history <schedule.history_path>` to include them."
                .to_string(),
        ),
    }
    sections.push(quality);

    let mut integrity = pdf::PdfSection {
        heading: "Integrity".to_string(),
        ..Default::default()
    };
    if let Some(policy) = &manifest.policy_sha256 {
        for (run, sha) in policy.iter().take(4) {
            integrity
                .rows
                .push((format!("policy sha · {run}"), short(sha)));
        }
    } else {
        integrity.paragraphs.push(
            "No --history file supplied: the run's config_sha256 (the politeness policy hash)              is therefore not attested in this report."
                .to_string(),
        );
    }
    integrity.rows.push((
        "manifest files".to_string(),
        manifest.files.len().to_string(),
    ));
    for f in manifest.files.iter().take(10) {
        integrity.rows.push((f.name.clone(), short(&f.sha256)));
    }
    if manifest.files.len() > 10 {
        integrity.rows.push((
            "…".to_string(),
            format!(
                "{} further file(s) in manifest.json",
                manifest.files.len() - 10
            ),
        ));
    }
    sections.push(integrity);

    // Bounded sample: first rows, first columns, of the first stream.
    if let Some((_, cols, recs)) = streams.first() {
        let show_cols = cols.len().min(6);
        let show_rows = recs.len().min(sample_rows);
        let mut table = pdf::PdfTable {
            columns: cols.iter().take(show_cols).cloned().collect(),
            rows: Vec::new(),
            omitted_rows: recs.len().saturating_sub(show_rows),
            omitted_cols: cols.len().saturating_sub(show_cols),
        };
        for r in recs.iter().take(show_rows) {
            table.rows.push(
                cols.iter()
                    .take(show_cols)
                    .map(|c| {
                        let v = r.get(c).unwrap_or(&Value::Null);
                        crate::export::cell(v)
                    })
                    .collect(),
            );
        }
        sections.push(pdf::PdfSection {
            heading: "Sample".to_string(),
            rows: Vec::new(),
            table: Some(table),
            paragraphs: Vec::new(),
        });
    }

    pdf::PdfDoc {
        title: format!(
            "harvest delivery report — {}",
            o.dataset.clone().unwrap_or_else(|| "records".to_string())
        ),
        generated_ts,
        subtitle: Some(format!(
            "{} record(s) · {} file(s) · verify with `harvest verify --dir {}`",
            manifest.records,
            manifest.files.len(),
            o.out_dir.display()
        )),
        sections,
        notes: vec![
            "This report is a summary, not a data dump: row and column counts are bounded by design."
                .to_string(),
            format!(
                "Byte-level integrity is proven by manifest.json (sha256 per file) and                  checksums.txt; re-check any delivery with `harvest verify --dir {}`.",
                o.out_dir.display()
            ),
        ],
    }
}

fn short(sha: &str) -> String {
    if sha.len() > 16 {
        format!("{}…", &sha[..16])
    } else {
        sha.to_string()
    }
}

/// ISO-8601 UTC rendering, reusing the engine's civil-date helper.
fn iso_ts(ts: i64) -> String {
    let (y, m, d) = crate::ids::civil_from_days(ts.div_euclid(86_400));
    let rem = ts.rem_euclid(86_400);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Aggregate run-level counters from the history log for the runs in this
/// export. Returns `None` when the file is missing or has no matching entry.
fn history_counters(history: &Path, runs: &[String]) -> Option<Vec<(String, String)>> {
    const MAX_HISTORY_BYTES: u64 = 64 * 1024 * 1024;
    let bytes = crate::fsutil::read_bounded(history, MAX_HISTORY_BYTES).ok()?;
    let txt = String::from_utf8(bytes).ok()?;
    let mut latest: std::collections::BTreeMap<String, (i64, Value)> = Default::default();
    for line in txt.lines() {
        let v: Value = serde_json::from_str(line).ok()?;
        let run = v.get("run")?.as_str()?.to_string();
        if !runs.contains(&run) {
            continue;
        }
        let ts = v.get("ts").and_then(|t| t.as_i64()).unwrap_or(0);
        match latest.get(&run) {
            Some((prev, _)) if *prev >= ts => {}
            _ => {
                latest.insert(run, (ts, v));
            }
        }
    }
    if latest.is_empty() {
        return None;
    }
    let mut acc = Counters::default();
    for (_, v) in latest.values() {
        let s = v.get("summary")?;
        acc.pages += s.get("pages_fetched").and_then(|x| x.as_u64()).unwrap_or(0);
        acc.records += s.get("records").and_then(|x| x.as_u64()).unwrap_or(0);
        acc.written += s.get("written").and_then(|x| x.as_u64()).unwrap_or(0);
        acc.deduped += s.get("deduped").and_then(|x| x.as_u64()).unwrap_or(0);
        acc.blocked += s.get("blocked").and_then(|x| x.as_u64()).unwrap_or(0);
        acc.retries += s.get("retries").and_then(|x| x.as_u64()).unwrap_or(0);
        acc.robots_denied += s.get("robots_denied").and_then(|x| x.as_u64()).unwrap_or(0);
        acc.quarantined += s.get("quarantined").and_then(|x| x.as_u64()).unwrap_or(0);
        acc.schema_drifts += s.get("schema_drifts").and_then(|x| x.as_u64()).unwrap_or(0);
        acc.duration_ms += s.get("duration_ms").and_then(|x| x.as_u64()).unwrap_or(0);
    }
    Some(vec![
        ("pages fetched".to_string(), acc.pages.to_string()),
        ("records extracted".to_string(), acc.records.to_string()),
        ("records written".to_string(), acc.written.to_string()),
        ("duplicates dropped".to_string(), acc.deduped.to_string()),
        (
            "quarantined (schema)".to_string(),
            acc.quarantined.to_string(),
        ),
        (
            "schema drift events".to_string(),
            acc.schema_drifts.to_string(),
        ),
        ("blocked".to_string(), acc.blocked.to_string()),
        ("robots denied".to_string(), acc.robots_denied.to_string()),
        ("retries".to_string(), acc.retries.to_string()),
        (
            "crawl duration".to_string(),
            format!("{} ms", acc.duration_ms),
        ),
    ])
}

#[derive(Default)]
struct Counters {
    pages: u64,
    records: u64,
    written: u64,
    deduped: u64,
    blocked: u64,
    retries: u64,
    robots_denied: u64,
    quarantined: u64,
    schema_drifts: u64,
    duration_ms: u64,
}

pub const PROGRESS_FILE: &str = ".export-progress.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Progress {
    fingerprint: String,
    files: Vec<FileEntry>,
}

fn load_progress(dir: &Path) -> Option<Progress> {
    use std::io::Read;
    let file = crate::fsutil::open_within(dir, &dir.join(PROGRESS_FILE), false).ok()?;
    let mut txt = String::new();
    file.take(16 * 1024 * 1024 + 1)
        .read_to_string(&mut txt)
        .ok()?;
    if txt.len() > 16 * 1024 * 1024 {
        return None;
    }
    serde_json::from_str(&txt).ok()
}

fn save_progress(dir: &Path, p: &Progress) -> Result<()> {
    crate::work_cancel::check().context("export cancelled before progress checkpoint")?;
    let bytes = serde_json::to_vec(p)?;
    crate::export_io::charge(bytes.len() as u64).context("charge export progress bytes")?;
    crate::fsutil::write_private_atomic(&dir.join(PROGRESS_FILE), &bytes)
        .context("atomically publish export progress")?;
    Ok(())
}

fn open_source_file(path: &Path) -> std::io::Result<std::fs::File> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "input needs a filename")
    })?;
    crate::fsutil::open_within(parent, &parent.join(name), false)
}

fn run_fingerprint(o: &ExportOpts) -> Result<String> {
    use std::io::Read;
    let mut total = 0u64;
    let mut h = Sha256::new();
    h.update(b"harvest-export-resume-v2");
    h.update(serde_json::to_vec(&serde_json::json!({
        "split_target": o.split_target, "max_part_bytes": o.max_part_bytes,
        "formats": o.formats, "dataset": o.dataset, "pdf_sample_rows": o.pdf_sample_rows, "out_dir": o.out_dir
    }))?);
    for input in o.inputs.iter().chain(o.history.iter()).chain(o.key.iter()) {
        h.update(input.display().to_string().as_bytes());
        h.update([0]);
        let mut file = open_source_file(input)
            .with_context(|| format!("open no-follow fingerprint input {}", input.display()))?;
        if !file.metadata()?.is_file() {
            bail!("fingerprint sources must be regular files");
        }
        let mut consumed = 0u64;
        let mut content = Sha256::new();
        let mut buf = [0u8; 65536];
        loop {
            crate::work_cancel::check()?;
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            consumed = consumed.saturating_add(n as u64);
            total = total.saturating_add(n as u64);
            if consumed > 64 * 1024 * 1024 || total > 256 * 1024 * 1024 {
                bail!("fingerprint input exceeds the bounded source cap");
            }
            content.update(&buf[..n]);
        }
        h.update(content.finalize());
    }
    Ok(digest_hex(h.finalize()))
}

fn safe_payload_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('.')
        && name != "manifest.json"
        && name != "manifest.json.sig"
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn hash_file(path: &Path) -> Result<(String, u64, u64)> {
    use std::io::Read;
    let mut f = open_source_file(path)
        .with_context(|| format!("open payload without following links: {}", path.display()))?;
    let before = f.metadata()?;
    if !before.is_file() {
        bail!("payload must be a regular file");
    }
    if before.len() > 256 * 1024 * 1024 {
        bail!("payload exceeds the 256 MiB file cap");
    }
    let mut sha = Sha256::new();
    let (mut bytes, mut lines) = (0u64, 0u64);
    let mut buf = [0u8; 65_536];
    loop {
        crate::work_cancel::check()?;
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sha.update(&buf[..n]);
        bytes = bytes.saturating_add(n as u64);
        if bytes > before.len() || bytes > 256 * 1024 * 1024 {
            bail!("payload changed or grew beyond its pinned size");
        }
        lines = lines.saturating_add(buf[..n].iter().filter(|b| **b == b'\n').count() as u64);
    }
    let after = f.metadata()?;
    if bytes != before.len()
        || after.len() != before.len()
        || before.modified().ok() != after.modified().ok()
    {
        bail!("payload changed or shrank while being read");
    }
    Ok((digest_hex(sha.finalize()), bytes, lines))
}

fn file_matches(path: &Path, entry: &FileEntry) -> bool {
    hash_file(path).is_ok_and(|(sha, bytes, lines)| {
        sha == entry.sha256 && bytes == entry.bytes && lines == entry.lines
    })
}

fn invalid_verification(message: &str) -> Verification {
    Verification {
        ok: false,
        records: 0,
        records_ok: false,
        records_basis: "not-checked",
        files: Vec::new(),
        extras: Vec::new(),
        signature: "not checked".into(),
        error: Some(message.into()),
    }
}

fn digest_hex(d: impl AsRef<[u8]>) -> String {
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Append a stream's parts to `parts`. `header` (possibly empty) is prepended
/// to every csv part; `cap == 0` keeps the stream in one file named
/// `{base}.{ext}`, chunked streams use `{base}.part-NNNNN.{ext}` (a single
/// resulting chunk keeps the plain name).
fn push_stream(
    parts: &mut Vec<(String, Vec<String>)>,
    base: &str,
    ext: &str,
    header: &str,
    data: Vec<String>,
    cap: u64,
) -> Result<()> {
    if parts.len() >= 250 {
        bail!(
            "export exceeds its 256-file delivery cap; use larger chunks or fewer targets/formats"
        );
    }
    if cap == 0 {
        let mut lines = Vec::with_capacity(data.len() + 1);
        if !header.is_empty() {
            lines.push(header.to_string());
        }
        lines.extend(data);
        parts.push((format!("{base}.{ext}"), lines));
        return Ok(());
    }
    let chunks = chunk_lines(&data, cap);
    let count = chunks.len();
    if parts.len().saturating_add(count) > 250 {
        bail!("export exceeds its 256-file delivery cap; use larger chunks");
    }
    for (i, chunk) in chunks.into_iter().enumerate() {
        let mut lines = Vec::with_capacity(chunk.len() + 1);
        if !header.is_empty() {
            lines.push(header.to_string());
        }
        lines.extend(chunk);
        let name = if count == 1 {
            format!("{base}.{ext}")
        } else {
            format!("{base}.part-{:05}.{ext}", i + 1)
        };
        parts.push((name, lines));
    }
    Ok(())
}

/// Greedy record-boundary chunking: parts stay under `cap` bytes where
/// possible; an oversized record is never split, it gets its own part.
fn chunk_lines(lines: &[String], cap: u64) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut cur_bytes = 0u64;
    for l in lines {
        let len = l.len() as u64 + 1; // + newline
        if !cur.is_empty() && cur_bytes + len > cap {
            out.push(std::mem::take(&mut cur));
            cur_bytes = 0;
        }
        cur_bytes += len;
        cur.push(l.clone());
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

fn csv_header(columns: &[String]) -> String {
    let header: Vec<String> = columns
        .iter()
        .map(|c| csv_escape(&spreadsheet_text(c)))
        .collect();
    header.join(",")
}

fn csv_data(rows: &[&Row], columns: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(rows.len());
    for (meta, payload) in rows {
        let mut cells = Vec::with_capacity(columns.len());
        for (i, col) in columns.iter().enumerate() {
            let v = if i < META_COLS.len() {
                meta[i].clone()
            } else {
                payload.get(col).cloned().unwrap_or(Value::Null)
            };
            let text = match &v {
                Value::String(s) => spreadsheet_text(s),
                _ => cell(&v),
            };
            cells.push(csv_escape(&text));
        }
        out.push(cells.join(","));
    }
    out
}

fn jsonl_lines(rows: &[&Row]) -> Vec<String> {
    let mut out = Vec::with_capacity(rows.len());
    for (meta, payload) in rows {
        let mut m = Map::new();
        for (i, k) in META_COLS.iter().enumerate() {
            m.insert(k.to_string(), meta[i].clone());
        }
        m.insert("fields".to_string(), Value::Object((*payload).clone()));
        out.push(Value::Object(m).to_string());
    }
    out
}

/// meta (run,target,url,depth,ts,fingerprint,egress) + flattened payload.
fn split_row(v: &Value) -> (Vec<Value>, Map<String, Value>) {
    let g = v.as_object().unwrap();
    let meta = META_COLS
        .iter()
        .map(|k| g.get(*k).cloned().unwrap_or(Value::Null))
        .collect();
    let mut payload = Map::new();
    if let Some(fields) = g.get("fields").and_then(|f| f.as_object()) {
        flatten_into("", fields, &mut payload);
    }
    for key in META_COLS {
        if let Some(value) = payload.remove(key) {
            payload.insert(format!("fields:{key}"), value);
        }
    }
    (meta, payload)
}

fn flatten_into(prefix: &str, obj: &Map<String, Value>, out: &mut Map<String, Value>) {
    for (k, val) in obj {
        let segment = k
            .replace('\\', "\\\\")
            .replace('.', "\\.")
            .replace(':', "\\:");
        let key = if prefix.is_empty() {
            segment
        } else {
            format!("{prefix}.{segment}")
        };
        match val {
            Value::Object(inner) if !inner.is_empty() => flatten_into(&key, inner, out),
            other => {
                out.insert(key, other.clone());
            }
        }
    }
}

fn target_columns(group: &[&Row]) -> Vec<String> {
    let mut keys: BTreeSet<String> = Default::default();
    for (_, p) in group {
        keys.extend(p.keys().cloned());
    }
    let mut cols: Vec<String> = META_COLS.iter().map(|s| s.to_string()).collect();
    cols.extend(keys);
    cols
}

fn cell(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Text that Excel/LibreOffice must not interpret as a formula. Numbers
/// remain numbers; canonical JSONL is unchanged for lossless interchange.
pub(crate) fn spreadsheet_text(s: &str) -> String {
    if s.starts_with(['\t', '\r', '\n']) || s.trim_start().starts_with(['=', '+', '-', '@']) {
        format!("'{s}")
    } else {
        s.to_string()
    }
}

pub(crate) fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn write_with_sha(dir: &Path, name: &str, body: &[u8]) -> Result<FileEntry> {
    let path = dir.join(name);
    let mut f =
        crate::export_io::create(&path).with_context(|| format!("create {}", path.display()))?;
    f.write_all(body)?;
    f.flush()?;
    let mut h = Sha256::new();
    h.update(body);
    let digest = h.finalize();
    let sha = digest.iter().map(|b| format!("{b:02x}")).collect();
    let lines = body.iter().filter(|b| **b == b'\n').count() as u64;
    Ok(FileEntry {
        name: name.to_string(),
        sha256: sha,
        bytes: body.len() as u64,
        lines,
    })
}

/// Newest `config_sha256` recorded for any of `runs` in the history log.
fn policy_for_runs(
    history: &Path,
    runs: &BTreeSet<String>,
) -> Result<std::collections::BTreeMap<String, String>> {
    const MAX_HISTORY_BYTES: u64 = 64 * 1024 * 1024;
    let bytes = crate::fsutil::read_bounded(history, MAX_HISTORY_BYTES)
        .context("read bounded no-follow history input")?;
    let txt = String::from_utf8(bytes).context("history is not valid UTF-8")?;
    let mut found: std::collections::BTreeMap<String, String> = Default::default();
    for line in txt.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(run) = v.get("run").and_then(|x| x.as_str()) else {
            continue;
        };
        if runs.contains(run) {
            if let Some(sha) = v.get("config_sha256").and_then(|x| x.as_str()) {
                found.insert(run.to_string(), sha.to_string()); // later lines win
            }
        }
    }
    Ok(found)
}

// ---------------------------------------------------------------- verify ----

/// One manifest file's verdict.
#[derive(Debug, Clone, Serialize)]
pub struct FileCheck {
    pub name: String,
    pub present: bool,
    pub bytes_ok: bool,
    pub sha_ok: bool,
    pub lines_ok: bool,
}

/// Structured outcome of `harvest verify`: every manifest promise checked
/// against the directory as it stands. `ok` means every payload file is
/// present with matching bytes, sha256, and line count, the record total
/// adds up, and — when a key is supplied — the detached manifest signature
/// verifies.
#[derive(Debug, Clone, Serialize)]
pub struct Verification {
    pub ok: bool,
    pub records: u64,
    pub records_ok: bool,
    /// How `records_ok` was established: "jsonl-lines" (re-counted from the
    /// JSONL payload) or "manifest-declared" (the delivery carries no JSONL, so
    /// the total cannot be re-derived from the folder and is carried as
    /// declared). Byte integrity is checked either way.
    pub records_basis: &'static str,
    pub files: Vec<FileCheck>,
    /// Files in the directory the manifest does not account for (hidden
    /// bookkeeping files and `manifest.json*` are not counted).
    pub extras: Vec<String>,
    /// "valid" | "invalid: …" | "not checked (no --key given)" | "unsigned".
    pub signature: String,
    /// Fatal problem that stops checking (missing/corrupt manifest).
    pub error: Option<String>,
}

/// Check an export directory against its manifest. Read-only; safe to run
/// on delivered data.
pub fn verify(dir: &Path, key: Option<&Path>) -> Result<Verification> {
    let manifest_path = dir.join("manifest.json");
    if std::fs::symlink_metadata(&manifest_path)
        .is_ok_and(|m| !m.file_type().is_file() || m.len() > 16 * 1024 * 1024)
    {
        return Ok(invalid_verification(
            "manifest.json must be a regular file no larger than 16 MiB",
        ));
    }
    let read_manifest = || -> std::io::Result<String> {
        use std::io::Read;
        let file = crate::fsutil::open_within(dir, &manifest_path, false)?;
        let mut text = String::new();
        file.take(16 * 1024 * 1024 + 1).read_to_string(&mut text)?;
        if text.len() > 16 * 1024 * 1024 {
            return Err(std::io::Error::other("manifest exceeds 16 MiB"));
        }
        Ok(text)
    };
    let txt = match read_manifest() {
        Ok(t) => t,
        Err(_) => {
            return Ok(Verification {
                ok: false,
                records: 0,
                records_ok: false,
                records_basis: "not-checked",
                files: Vec::new(),
                extras: Vec::new(),
                signature: "not checked".into(),
                error: Some(format!(
                    "no manifest.json in {} — export missing or never finalized",
                    dir.display()
                )),
            })
        }
    };
    let manifest: Manifest = match serde_json::from_str(&txt) {
        Ok(m) => m,
        Err(e) => {
            return Ok(Verification {
                ok: false,
                records: 0,
                records_ok: false,
                records_basis: "not-checked",
                files: Vec::new(),
                extras: Vec::new(),
                signature: "not checked".into(),
                error: Some(format!("manifest.json is unreadable: {e}")),
            })
        }
    };
    if manifest.v != 1
        || manifest.files.is_empty()
        || manifest.files.iter().all(|f| f.name == "checksums.txt")
    {
        return Ok(invalid_verification(
            "unsupported manifest version or empty payload file list",
        ));
    }
    let mut declared_names = std::collections::HashSet::new();
    for fe in &manifest.files {
        if !safe_payload_name(&fe.name) || !declared_names.insert(fe.name.as_str()) {
            return Ok(invalid_verification(
                "unsafe or duplicate filename in manifest",
            ));
        }
    }
    let mut files = Vec::new();
    let mut jsonl_records = 0u64;
    for fe in &manifest.files {
        let path = dir.join(&fe.name);
        let (present, bytes_ok, sha_ok, lines_ok) = match hash_file(&path) {
            Ok((sha, bytes, lines)) => {
                if fe.name.ends_with(".jsonl") {
                    jsonl_records = jsonl_records.saturating_add(lines);
                }
                (true, bytes == fe.bytes, sha == fe.sha256, lines == fe.lines)
            }
            Err(_) => (false, false, false, false),
        };
        files.push(FileCheck {
            name: fe.name.clone(),
            present,
            bytes_ok,
            sha_ok,
            lines_ok,
        });
    }
    let declared: std::collections::HashSet<&str> =
        manifest.files.iter().map(|f| f.name.as_str()).collect();
    let mut extras = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.')
                || name == "manifest.json"
                || name == "manifest.json.sig"
                || declared.contains(name.as_str())
            {
                continue;
            }
            extras.push(name);
        }
        extras.sort();
    }
    // Record totals are re-counted from the JSONL payload when the delivery
    // includes one. Format sets without JSONL (xlsx/parquet/xml only) cannot be
    // re-counted by reading the folder, so the declared total is carried — and
    // the basis is reported instead of silently claiming a recount happened.
    let has_jsonl = manifest.files.iter().any(|f| f.name.ends_with(".jsonl"));
    let (records_ok, records_basis) = if has_jsonl {
        (jsonl_records == manifest.records, "jsonl-lines")
    } else {
        (manifest.records > 0, "manifest-declared")
    };
    let signature = match key {
        Some(kp) => {
            let sig = dir.join("manifest.json.sig");
            if !sig.exists() {
                "invalid: no manifest.json.sig to check".to_string()
            } else {
                match crate::sign::Key::load(kp).and_then(|k| {
                    use std::io::Read;
                    let file = crate::fsutil::open_within(dir, &sig, false)?;
                    let mut signature = String::new();
                    file.take(1025).read_to_string(&mut signature)?;
                    if signature.len() > 1024 {
                        bail!("manifest signature too large");
                    }
                    k.verify_bytes(txt.as_bytes(), &signature)
                }) {
                    Ok(()) => "valid".to_string(),
                    Err(e) => format!("invalid: {e:#}"),
                }
            }
        }
        None if dir.join("manifest.json.sig").exists() => {
            "not checked (no --key given)".to_string()
        }
        None => "unsigned".to_string(),
    };
    let files_ok = files
        .iter()
        .all(|f| f.present && f.bytes_ok && f.sha_ok && f.lines_ok);
    let sig_ok = !signature.starts_with("invalid");
    Ok(Verification {
        ok: files_ok && records_ok && sig_ok,
        records: manifest.records,
        records_ok,
        records_basis,
        files,
        extras,
        signature,
        error: None,
    })
}

pub mod bundle;
pub mod parquet;
pub mod pdf;
pub mod xlsx;
pub mod xml;

#[cfg(test)]
mod tests;
