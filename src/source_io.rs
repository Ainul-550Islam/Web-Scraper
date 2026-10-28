//! Bounded readers for the configured local output, normalized to records.
//! CSV is inherently lossy: field values remain strings (including formula
//! neutralization); empty cells become null. JSONL/SQLite preserve JSON types.
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::config::{Config, SinkCfg};
const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_TOTAL: u64 = 256 * 1024 * 1024;
const MAX_JSONL_RECORD_BYTES: usize = 8 * 1024 * 1024;
const META: [&str; 7] = [
    "run",
    "target",
    "url",
    "depth",
    "ts",
    "fingerprint",
    "egress",
];

/// A cap must report excess bytes, never fake EOF in the middle of a
/// syntactically valid CSV field. `Read::take` can silently accept that tail.
pub(crate) struct CappedReader<R> {
    inner: R,
    left: u64,
}
impl<R> CappedReader<R> {
    pub(crate) fn new(inner: R, limit: u64) -> Self {
        Self { inner, left: limit }
    }
}
impl<R: Read> Read for CappedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        crate::work_cancel::check()?;
        if buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            let mut extra = [0u8; 1];
            if self.inner.read(&mut extra)? != 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "source exceeds its read byte cap or grew during reading",
                ));
            }
            return Ok(0);
        }
        let cap = (self.left.min(buf.len() as u64)) as usize;
        let n = self.inner.read(&mut buf[..cap])?;
        self.left -= n as u64;
        Ok(n)
    }
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    line: &mut Vec<u8>,
    cap: usize,
) -> std::io::Result<usize> {
    line.clear();
    loop {
        crate::work_cancel::check()?;
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(line.len());
        }
        let end = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |i| i + 1);
        if line.len().saturating_add(end) > cap {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "JSONL record exceeds the 8 MiB record cap",
            ));
        }
        let complete = available.get(end.saturating_sub(1)) == Some(&b'\n');
        line.extend_from_slice(&available[..end]);
        reader.consume(end);
        if complete {
            return Ok(line.len());
        }
    }
}

fn open(path: &Path) -> Result<std::fs::File> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path.file_name().context("source needs a filename")?;
    let file = crate::fsutil::open_within(parent, &parent.join(name), false)?;
    if file.metadata()?.len() > MAX_FILE {
        bail!("source exceeds the 64 MiB per-file cap; rotate/merge a smaller range");
    }
    Ok(file)
}

fn source_bytes(cfg: &Config, path: &Path) -> Result<u64> {
    let mut total = open(path)?.metadata()?.len();
    if matches!(&cfg.output, SinkCfg::Sqlite { .. }) {
        crate::fsutil::reject_sqlite_sidecars(path)?;
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            let sidecar = PathBuf::from(name);
            match std::fs::symlink_metadata(&sidecar) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
                Ok(meta) => {
                    if !meta.is_file() || meta.len() > MAX_FILE {
                        bail!("SQLite sidecar exceeds the 64 MiB per-file source cap");
                    }
                    total = total.saturating_add(meta.len());
                }
            }
        }
    }
    Ok(total)
}

/// The reader never clones the entire input; callers decide whether to stage
/// rows on disk or retain a small, bounded analysis sample in memory.
pub(crate) fn visit(
    cfg: &Config,
    paths: &[PathBuf],
    maximum: u64,
    mut emit: impl FnMut(Value) -> Result<()>,
) -> Result<u64> {
    if paths.len() > crate::export_job::MAX_SOURCE_FILES {
        bail!("too many source files; select a smaller range");
    }
    let mut total = 0u64;
    for p in paths {
        total = total.saturating_add(source_bytes(cfg, p)?);
    }
    if total > MAX_TOTAL {
        bail!("sources exceed the 256 MiB aggregate cap");
    }
    let mut left = maximum.min(100_000);
    let mut count = 0u64;
    for path in paths {
        if left == 0 {
            break;
        }
        match &cfg.output {
            SinkCfg::Csv { .. } => {
                let file = open(path)?;
                let mut csv = csv::Reader::from_reader(CappedReader::new(file, MAX_FILE));
                let header = csv.headers()?.clone();
                if header.len() < 7 || !header.iter().take(7).eq(META) {
                    bail!("configured CSV is not a harvest schema");
                }
                let declared: BTreeSet<String> = cfg
                    .targets
                    .iter()
                    .flat_map(|t| t.extract.keys().cloned())
                    .collect();
                let names: Vec<String> = header
                    .iter()
                    .skip(7)
                    .map(|name| {
                        declared
                            .iter()
                            .find(|k| crate::export::spreadsheet_text(k) == name)
                            .cloned()
                            .unwrap_or_else(|| name.to_string())
                    })
                    .collect();
                if names.len() > 1024
                    || names.iter().any(|n| n.is_empty() || n.len() > 256)
                    || names.iter().collect::<BTreeSet<_>>().len() != names.len()
                {
                    bail!("CSV schema exceeds export limits");
                }
                for row in csv.records() {
                    crate::work_cancel::check()?;
                    let row = row?;
                    let depth = row
                        .get(3)
                        .unwrap_or("")
                        .parse::<u64>()
                        .context("CSV depth is invalid")?;
                    let ts = row
                        .get(4)
                        .unwrap_or("")
                        .parse::<i64>()
                        .context("CSV timestamp is invalid")?;
                    let mut fields = Map::new();
                    for (i, key) in names.iter().enumerate() {
                        let text = row.get(i + 7).unwrap_or("");
                        fields.insert(
                            key.clone(),
                            if text.is_empty() {
                                Value::Null
                            } else {
                                Value::String(text.to_string())
                            },
                        );
                    }
                    let recover = |text: &str| {
                        if crate::export::spreadsheet_text(&cfg.name) == text {
                            cfg.name.clone()
                        } else {
                            cfg.targets
                                .iter()
                                .find(|t| crate::export::spreadsheet_text(&t.id) == text)
                                .map_or_else(|| text.to_string(), |t| t.id.clone())
                        }
                    };
                    emit(
                        json!({"run":recover(&row[0]),"target":recover(&row[1]),"url":row[2],"depth":depth,"ts":ts,"fingerprint":row[5],"egress":row[6],"fields":fields}),
                    )?;
                    count += 1;
                    left -= 1;
                    if left == 0 {
                        break;
                    }
                }
            }
            SinkCfg::Sqlite { .. } => {
                #[cfg(feature = "sqlite")]
                {
                    crate::fsutil::reject_sqlite_sidecars(path)?;
                    let conn = rusqlite::Connection::open_with_flags(
                        path,
                        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                            | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
                    )?;
                    conn.busy_timeout(std::time::Duration::from_secs(5))?;
                    let mut stmt = conn.prepare("SELECT run,target,url,depth,ts,fingerprint,egress,payload FROM records ORDER BY id LIMIT ?1")?;
                    let mut rows = stmt.query([left as i64])?;
                    while let Some(row) = rows.next()? {
                        crate::work_cancel::check()?;
                        let payload: String = row.get(7)?;
                        if payload.len() as u64 > MAX_FILE {
                            bail!("SQLite record exceeds the source byte cap");
                        }
                        let fields: Value = serde_json::from_str(&payload)?;
                        if !fields.is_object() {
                            bail!("SQLite payload must be a field object");
                        }
                        let depth: i64 = row.get(3)?;
                        if depth < 0 {
                            bail!("SQLite record has a negative depth");
                        }
                        emit(
                            json!({"run":row.get::<_,String>(0)?,"target":row.get::<_,String>(1)?,"url":row.get::<_,String>(2)?,"depth":depth,"ts":row.get::<_,i64>(4)?,"fingerprint":row.get::<_,String>(5)?,"egress":row.get::<_,String>(6)?,"fields":fields}),
                        )?;
                        count += 1;
                        left -= 1;
                    }
                }
                #[cfg(not(feature = "sqlite"))]
                bail!("SQLite source requires the sqlite build feature");
            }
            _ => {
                let mut reader = BufReader::new(CappedReader::new(open(path)?, MAX_FILE));
                let mut consumed = 0u64;
                let mut line = Vec::new();
                loop {
                    crate::work_cancel::check()?;
                    let n = read_bounded_line(&mut reader, &mut line, MAX_JSONL_RECORD_BYTES)?;
                    if n == 0 {
                        break;
                    }
                    consumed = consumed.saturating_add(n as u64);
                    if consumed > MAX_FILE {
                        bail!("source grew beyond the 64 MiB read cap");
                    }
                    let text = std::str::from_utf8(&line).context("JSONL source is not UTF-8")?;
                    if text.trim().is_empty() {
                        continue;
                    }
                    let record: Value = serde_json::from_str(text).context(
                        "source has malformed JSONL; repair it instead of silently dropping rows",
                    )?;
                    if !record.is_object() {
                        bail!("source record must be a JSON object");
                    }
                    emit(record)?;
                    count += 1;
                    left -= 1;
                    if left == 0 {
                        break;
                    }
                }
            }
        }
    }
    Ok(count)
}

pub(crate) fn stage(
    cfg: &Config,
    paths: &[PathBuf],
    dir: &Path,
    maximum: u64,
) -> Result<Vec<PathBuf>> {
    if paths.len() > crate::export_job::MAX_SOURCE_FILES {
        bail!("too many source files");
    }
    let mut total = 0u64;
    for p in paths {
        total = total.saturating_add(source_bytes(cfg, p)?);
    }
    if total > MAX_TOTAL {
        bail!("sources exceed the 256 MiB aggregate cap");
    }
    let mut files = Vec::new();
    let mut left = maximum.min(100_000);
    let mut bytes = 0u64;
    for (i, source) in paths.iter().enumerate() {
        if left == 0 {
            break;
        }
        let path = dir.join(format!("source-{i}.jsonl"));
        let mut file = crate::fsutil::open_within(dir, &path, true)?;
        file.set_len(0)?;
        let n = visit(cfg, std::slice::from_ref(source), left, |record| {
            let body = serde_json::to_vec(&record)?;
            bytes = bytes.saturating_add(body.len() as u64 + 1);
            if bytes > MAX_TOTAL {
                bail!("normalized sources exceed the 256 MiB stage cap");
            }
            file.write_all(&body)?;
            file.write_all(b"\n")?;
            Ok(())
        })?;
        file.sync_all()?;
        left -= n;
        if n > 0 {
            files.push(path);
        }
    }
    if files.is_empty() {
        bail!("the configured output holds no records to export");
    }
    Ok(files)
}

/// Discover partitioned dataset parts without following descendant links or
/// silently truncating the source list. Bounds cover directories too.
pub(crate) fn dataset_files(root: &Path) -> Result<Vec<PathBuf>> {
    crate::fsutil::reject_symlink_components(root)?;
    let mut pending = vec![root.to_path_buf()];
    let mut visited = 0usize;
    let mut files = Vec::new();
    while let Some(dir) = pending.pop() {
        visited += 1;
        if visited > 256 {
            bail!("dataset exceeds the 256-directory discovery cap; select/merge a smaller range");
        }
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                bail!("dataset must not contain symlink entries");
            }
            if kind.is_dir() {
                // The control plane stores private run snapshots and prior
                // deliveries beneath this directory when a dataset sink is
                // configured. Those generated JSONL files are not source
                // partitions and must never be re-ingested as crawl output.
                if dir == root && entry.file_name() == std::ffi::OsStr::new(".harvest-control") {
                    continue;
                }
                pending.push(entry.path());
            } else if kind.is_file() && entry.path().extension().is_some_and(|e| e == "jsonl") {
                files.push(entry.path());
                if files.len() > crate::export_job::MAX_SOURCE_FILES {
                    bail!("dataset exceeds the 64-part cap; merge a smaller range (no parts were silently skipped)");
                }
            }
        }
        if pending.len() > 256 {
            bail!("dataset directory fanout exceeds its cap");
        }
    }
    files.sort();
    Ok(files)
}

pub(crate) struct StageGuard(pub PathBuf);
impl Drop for StageGuard {
    fn drop(&mut self) {
        if std::fs::remove_dir_all(&self.0).is_err() {
            tracing::warn!("could not remove private source stage; operator cleanup required");
        }
    }
}

#[cfg(test)]
mod phase2_tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn phase2_csv_budget_is_an_error_not_a_synthetic_end_of_field() {
        let body = b"name,value\nrow,a-long-payload\n";
        let cap = body.len() as u64 - 6;
        let mut reader = csv::Reader::from_reader(CappedReader::new(Cursor::new(body), cap));
        assert!(
            reader.records().next().unwrap().is_err(),
            "a CSV parser must not accept a silently shortened last column"
        );
    }

    #[test]
    fn phase2_reader_allows_exact_limit_and_rejects_one_extra_byte() {
        let mut exact = CappedReader::new(Cursor::new(b"hello"), 5);
        let mut text = String::new();
        exact.read_to_string(&mut text).unwrap();
        assert_eq!(text, "hello");
        let mut excess = CappedReader::new(Cursor::new(b"hello!"), 5);
        let mut text = String::new();
        assert!(excess.read_to_string(&mut text).is_err());
    }

    #[test]
    fn phase2_jsonl_line_reader_enforces_a_hard_row_limit() {
        let mut exact = BufReader::new(Cursor::new(b"12345\n"));
        let mut line = Vec::new();
        assert_eq!(read_bounded_line(&mut exact, &mut line, 6).unwrap(), 6);
        assert_eq!(line, b"12345\n");

        let mut oversized = BufReader::new(Cursor::new(b"123456\n"));
        assert!(read_bounded_line(&mut oversized, &mut line, 6).is_err());
    }
}
