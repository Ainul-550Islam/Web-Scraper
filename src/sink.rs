//! Pluggable result sinks: JSONL (default), CSV, SQLite (feature `sqlite`),
//! batched HTTP webhook, and null (for load tests). Sinks are append-only and
//! idempotent-safe: the engine dedups by fingerprint before writing.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Map;

use crate::config::SinkCfg;

fn ensure_output_directory(path: &Path) -> Result<()> {
    crate::fsutil::reject_symlink_components(path)?;
    std::fs::create_dir_all(path)
        .with_context(|| format!("create output directory {}", path.display()))?;
    crate::fsutil::reject_symlink_components(path)?;
    Ok(())
}

fn ensure_output_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    ensure_output_directory(parent)
}

#[derive(Debug, Clone, Serialize)]
pub struct Record {
    pub run: String,
    pub target: String,
    pub url: String,
    pub depth: usize,
    pub ts: i64,
    pub fingerprint: String,
    pub egress: String,
    pub fields: Map<String, serde_json::Value>,
}

pub trait Sink: Send + Sync {
    fn write(&self, rec: &Record) -> Result<()>;
    fn flush(&self) -> Result<()> {
        Ok(())
    }
    /// Final barrier: no owned delivery worker may outlive a completed run.
    fn finish(&self) -> Result<()> {
        self.flush()
    }
}

pub(crate) async fn finish_owned(sink: &std::sync::Arc<dyn Sink>) -> Result<()> {
    let sink = sink.clone();
    tokio::task::spawn_blocking(move || sink.finish())
        .await
        .map_err(|_| anyhow::anyhow!("sink finalizer panicked"))?
}

#[derive(Default)]
pub struct NullSink;
impl Sink for NullSink {
    fn write(&self, _rec: &Record) -> Result<()> {
        Ok(())
    }
}

/// Batched webhook sink, fully off-thread: async callers only hand record
/// JSON to a reservation-bounded mpsc (1024 records / 32 MiB). A dedicated std thread owns everything
/// `reqwest::blocking` related (a blocking client builds and drops its own
/// runtime, so it must never touch a tokio worker), batches POSTs, and
/// reports failures back through the `flush()` ack. `flush()` blocks the
/// calling task only at end-of-run, when nothing else needs it.
pub struct HttpSink {
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    submit: Mutex<()>,
    closed: std::sync::atomic::AtomicBool,
    deadline: std::sync::Arc<crate::sink_lifecycle::DeliveryDeadline>,
    tx: std::sync::mpsc::Sender<Cmd>,
    pending: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pending_bytes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub bytes_sent: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub batches_sent: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

pub(crate) fn reserve_buffer(
    pending: &std::sync::atomic::AtomicUsize,
    bytes: &std::sync::atomic::AtomicUsize,
    n: usize,
) -> Result<()> {
    use std::sync::atomic::Ordering::SeqCst;
    bytes
        .fetch_update(SeqCst, SeqCst, |v| {
            v.checked_add(n).filter(|v| *v <= 32 * 1024 * 1024)
        })
        .map_err(|_| anyhow::anyhow!("sink buffer byte capacity exhausted (32 MiB)"))?;
    if pending
        .fetch_update(SeqCst, SeqCst, |v| (v < 1024).then_some(v + 1))
        .is_err()
    {
        bytes.fetch_sub(n, SeqCst);
        anyhow::bail!("sink buffer record capacity exhausted (1024 records)");
    }
    Ok(())
}

enum Cmd {
    Push(String),
    Flush(std::sync::mpsc::Sender<Result<()>>),
    Finish(std::sync::mpsc::Sender<Result<()>>),
    Shutdown,
}

struct SinkThread {
    deadline: std::sync::Arc<crate::sink_lifecycle::DeliveryDeadline>,
    timeout_secs: u64,
    url: String,
    auth: Option<String>,
    batch_max: usize,
    client: Option<reqwest::blocking::Client>,
    batch: Vec<String>,
    last_err: Option<String>,
    bytes_sent: std::sync::Arc<std::sync::atomic::AtomicU64>,
    batches_sent: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pending: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pending_bytes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl SinkThread {
    fn dec_pending(&self, n: usize) {
        let mut guard = true;
        while guard {
            let cur = self.pending.load(std::sync::atomic::Ordering::SeqCst);
            let next = cur.saturating_sub(n);
            guard = self
                .pending
                .compare_exchange(
                    cur,
                    next,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                )
                .is_err();
        }
    }

    fn post(&mut self) {
        if self.batch.is_empty() {
            return;
        }
        let count = self.batch.len();
        let queued_bytes: usize = self.batch.iter().map(String::len).sum();
        let mut body = String::with_capacity(queued_bytes + count + 1);
        body.push('[');
        for (i, line) in self.batch.drain(..).enumerate() {
            if i > 0 {
                body.push(',');
            }
            body.push_str(&line);
        }
        body.push(']');
        let remaining = self.deadline.remaining(
            std::time::Duration::from_secs(self.timeout_secs.max(1))
                .min(crate::sink_lifecycle::OP_LIMIT),
        );
        if remaining.is_zero() || self.last_err.is_some() {
            self.last_err.get_or_insert_with(|| "HTTP sink aggregate drain deadline exceeded; buffered records were not delivered".into());
            self.dec_pending(count);
            self.pending_bytes
                .fetch_sub(queued_bytes, std::sync::atomic::Ordering::SeqCst);
            return;
        }
        let Some(client) = &self.client else {
            self.last_err = Some("sink thread: blocking client unavailable".into());
            self.dec_pending(count);
            self.pending_bytes
                .fetch_sub(queued_bytes, std::sync::atomic::Ordering::SeqCst);
            return;
        };
        let len = body.len() as u64;
        let mut req = client
            .post(&self.url)
            .header("content-type", "application/json")
            .timeout(remaining)
            .body(body);
        if let Some(a) = &self.auth {
            req = req.header("authorization", a.clone());
        }
        let sent = match req.send() {
            Ok(r) if r.status().is_success() => true,
            Ok(r) => {
                self.last_err.get_or_insert_with(|| {
                    format!(
                        "sink POST {} returned {}",
                        crate::config::redact_url_origin(&self.url),
                        r.status()
                    )
                });
                false
            }
            Err(e) => {
                self.last_err.get_or_insert_with(|| {
                    format!(
                        "sink POST {} failed: {}",
                        crate::config::redact_url_origin(&self.url),
                        e.without_url()
                    )
                });
                false
            }
        };
        if sent {
            self.batches_sent
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.bytes_sent
                .fetch_add(len, std::sync::atomic::Ordering::Relaxed);
        }
        // Whether delivered or permanently failed, these are no longer buffered.
        self.dec_pending(count);
        self.pending_bytes
            .fetch_sub(queued_bytes, std::sync::atomic::Ordering::SeqCst);
    }

    fn run(mut self, rx: std::sync::mpsc::Receiver<Cmd>) {
        self.client = match reqwest::blocking::Client::builder()
            .timeout(
                std::time::Duration::from_secs(self.timeout_secs.max(1))
                    .min(crate::sink_lifecycle::OP_LIMIT),
            )
            .connect_timeout(crate::sink_lifecycle::OP_LIMIT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("harvest-sink/0.1")
            .build()
        {
            Ok(c) => Some(c),
            Err(e) => {
                self.last_err = Some(format!("sink client build failed: {e}"));
                None
            }
        };
        loop {
            match rx.recv() {
                Ok(Cmd::Push(line)) => {
                    self.batch.push(line);
                    if self.batch.len() >= self.batch_max {
                        self.post();
                    }
                }
                Ok(Cmd::Flush(ack)) => {
                    self.post();
                    let _ = ack.send(match &self.last_err {
                        Some(e) => Err(anyhow::anyhow!(e.clone())),
                        None => Ok(()),
                    });
                }
                Ok(Cmd::Finish(ack)) => {
                    self.post();
                    let _ = ack.send(match &self.last_err {
                        Some(e) => Err(anyhow::anyhow!(e.clone())),
                        None => Ok(()),
                    });
                    break;
                }
                Ok(Cmd::Shutdown) | Err(_) => {
                    self.post();
                    break;
                }
            }
        }
        // Dropping the blocking client here runs on this plain thread —
        // never on a tokio worker.
        drop(self.client.take());
    }
}

impl HttpSink {
    fn open(
        url: String,
        timeout_secs: u64,
        auth: Option<String>,
        batch_max: usize,
    ) -> Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
        let sink = Self {
            worker: Mutex::new(None),
            submit: Mutex::new(()),
            closed: Default::default(),
            deadline: Default::default(),
            tx,
            pending: Default::default(),
            pending_bytes: Default::default(),
            bytes_sent: Default::default(),
            batches_sent: Default::default(),
        };
        let st = SinkThread {
            deadline: sink.deadline.clone(),
            timeout_secs,
            url,
            auth,
            batch_max: batch_max.max(1),
            client: None,
            batch: Vec::new(),
            last_err: None,
            bytes_sent: sink.bytes_sent.clone(),
            batches_sent: sink.batches_sent.clone(),
            pending: sink.pending.clone(),
            pending_bytes: sink.pending_bytes.clone(),
        };
        let worker = std::thread::Builder::new()
            .name("harvest-http-sink".into())
            .spawn(move || st.run(rx))
            .context("spawn sink thread")?;
        *sink.worker.lock().unwrap_or_else(|p| p.into_inner()) = Some(worker);
        Ok(sink)
    }

    pub fn buffered(&self) -> usize {
        self.pending.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Sink for HttpSink {
    fn write(&self, rec: &Record) -> Result<()> {
        let _submit = self.submit.lock().unwrap_or_else(|p| p.into_inner());
        if self.closed.load(std::sync::atomic::Ordering::SeqCst) {
            anyhow::bail!("HTTP sink is closed");
        }
        let line = serde_json::to_string(rec)?;
        let n = line.len();
        reserve_buffer(&self.pending, &self.pending_bytes, n)?;
        if self.tx.send(Cmd::Push(line)).is_err() {
            self.pending
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            self.pending_bytes
                .fetch_sub(n, std::sync::atomic::Ordering::SeqCst);
            anyhow::bail!("sink thread gone");
        }
        Ok(())
    }

    fn flush(&self) -> Result<()> {
        let _submit = self.submit.lock().unwrap_or_else(|p| p.into_inner());
        self.deadline.begin();
        let result = (|| {
            let (ack_tx, ack_rx) = std::sync::mpsc::channel();
            self.tx
                .send(Cmd::Flush(ack_tx))
                .map_err(|_| anyhow::anyhow!("sink thread gone"))?;
            ack_rx
                .recv_timeout(
                    crate::sink_lifecycle::DRAIN_LIMIT + std::time::Duration::from_secs(1),
                )
                .map_err(|_| anyhow::anyhow!("sink flush acknowledgement deadline exceeded"))?
        })();
        self.deadline.end();
        result
    }
    fn finish(&self) -> Result<()> {
        let _submit = self.submit.lock().unwrap_or_else(|p| p.into_inner());
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        self.deadline.begin();
        let result = (|| {
            let (ack_tx, ack_rx) = std::sync::mpsc::channel();
            self.tx
                .send(Cmd::Finish(ack_tx))
                .map_err(|_| anyhow::anyhow!("sink thread gone"))?;
            ack_rx
                .recv_timeout(
                    crate::sink_lifecycle::DRAIN_LIMIT + std::time::Duration::from_secs(1),
                )
                .map_err(|_| anyhow::anyhow!("sink finish acknowledgement deadline exceeded"))?
        })();
        if let Some(worker) = self.worker.lock().unwrap_or_else(|p| p.into_inner()).take() {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("HTTP sink worker panicked"))?;
        }
        result
    }
}

impl Drop for HttpSink {
    fn drop(&mut self) {
        self.deadline.begin();
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(worker) = self
            .worker
            .get_mut()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            let _ = worker.join();
        }
    }
}

/// Directory-partitioned dataset delivery: `<dir>/dt=YYYY-MM-DD/part-<run>-<seq>.jsonl`
/// — an S3-friendly layout with zero cloud deps (sync the dir with any tool).
pub struct DatasetSink {
    dir: PathBuf,
    by_day: bool,
    files: Mutex<PartFiles>,
    seq: std::sync::atomic::AtomicUsize,
}

type PartFiles = std::collections::HashMap<String, (PathBuf, File)>;

impl DatasetSink {
    fn open(dir: PathBuf, partition: &str) -> Result<Self> {
        ensure_output_directory(&dir)?;
        Ok(Self {
            dir,
            by_day: partition == "day",
            files: Mutex::new(std::collections::HashMap::new()),
            seq: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    fn file_for(&self, rec: &Record) -> Result<(std::sync::MutexGuard<'_, PartFiles>, String)> {
        let key = if self.by_day {
            let (y, m, d) = civil_from_days(rec.ts.div_euclid(86_400));
            format!("dt={y:04}-{m:02}-{d:02}")
        } else {
            "parts".to_string()
        };
        let mut g = self.files.lock().unwrap_or_else(|p| p.into_inner());
        if !g.contains_key(&key) {
            let dir = self.dir.join(&key);
            ensure_output_directory(&dir)?;
            let n = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = dir.join(format!(
                "part-{}-{}-{}.jsonl",
                crate::document::safe_dataset_name(&rec.run),
                std::process::id(),
                n
            ));
            let f = crate::fsutil::open_append(&path)?;
            g.insert(key.clone(), (path, f));
        }
        Ok((g, key))
    }
}

impl Sink for DatasetSink {
    fn write(&self, rec: &Record) -> Result<()> {
        let line = serde_json::to_string(rec)?;
        let (mut g, key) = self.file_for(rec)?;
        let f = &mut g.get_mut(&key).unwrap().1;
        writeln!(f, "{line}")?;
        Ok(())
    }

    fn flush(&self) -> Result<()> {
        let mut g = self.files.lock().unwrap_or_else(|p| p.into_inner());
        for entry in g.values_mut() {
            entry.1.flush()?;
        }
        Ok(())
    }
}

impl DatasetSink {
    pub fn parts(&self) -> Vec<PathBuf> {
        self.files
            .lock()
            .unwrap()
            .values()
            .map(|(p, _)| p.clone())
            .collect()
    }
}

/// Calendar math consolidated in `crate::ids` (the dataset partition label
/// and the CLI `--dt` filter must agree exactly — one implementation).
pub(crate) use crate::ids::civil_from_days;

pub fn build(cfg: &SinkCfg) -> Result<(std::sync::Arc<dyn Sink>, PathBuf)> {
    match cfg {
        SinkCfg::Null {} => Ok((std::sync::Arc::new(NullSink), PathBuf::from("(null)"))),
        SinkCfg::Dataset { dir, partition } => Ok((
            std::sync::Arc::new(DatasetSink::open(dir.clone(), partition)?)
                as std::sync::Arc<dyn Sink>,
            dir.clone(),
        )),
        #[cfg(feature = "postgres")]
        SinkCfg::Postgres { dsn } => Ok((
            std::sync::Arc::new(crate::pgsink::PgSink::open(dsn.clone())?)
                as std::sync::Arc<dyn Sink>,
            PathBuf::from("(postgres)"),
        )),
        #[cfg(not(feature = "postgres"))]
        SinkCfg::Postgres { .. } => {
            anyhow::bail!(
                "binary built without Postgres support — rebuild with `--features postgres`"
            )
        }
        SinkCfg::Jsonl { path } => Ok((std::sync::Arc::new(JsonlSink::open(path)?), path.clone())),
        SinkCfg::Csv { path } => Ok((std::sync::Arc::new(CsvSink::open(path)?), path.clone())),
        SinkCfg::Http {
            url,
            batch_max,
            timeout_secs,
            auth,
        } => {
            let p = PathBuf::from(url);
            Ok((
                std::sync::Arc::new(HttpSink::open(
                    url.clone(),
                    *timeout_secs,
                    auth.clone(),
                    *batch_max,
                )?) as std::sync::Arc<dyn Sink>,
                p,
            ))
        }
        #[cfg(feature = "sqlite")]
        SinkCfg::Sqlite { path } => Ok((
            std::sync::Arc::new(SqliteSink::open(path)?) as std::sync::Arc<dyn Sink>,
            path.clone(),
        )),
        #[cfg(not(feature = "sqlite"))]
        SinkCfg::Sqlite { .. } => {
            anyhow::bail!("binary built without SQLite support — rebuild with `--features sqlite`")
        }
    }
}

pub struct JsonlSink {
    file: Mutex<File>,
}

impl JsonlSink {
    fn open(path: &Path) -> Result<Self> {
        ensure_output_parent(path)?;
        let file = crate::fsutil::open_append(path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }
}

impl Sink for JsonlSink {
    fn write(&self, rec: &Record) -> Result<()> {
        let line = serde_json::to_string(rec)?;
        let mut f = self.file.lock().unwrap_or_else(|p| p.into_inner());
        writeln!(f, "{line}")?;
        Ok(())
    }
    fn flush(&self) -> Result<()> {
        let mut f = self.file.lock().unwrap_or_else(|p| p.into_inner());
        f.flush()?;
        Ok(())
    }
}

pub struct CsvSink {
    file: Mutex<CsvState>,
}

struct CsvState {
    writer: csv::Writer<File>,
    keys: Option<Vec<String>>,
    header_done: bool,
}

const CSV_META: [&str; 7] = [
    "run",
    "target",
    "url",
    "depth",
    "ts",
    "fingerprint",
    "egress",
];

fn validate_csv_keys(keys: &[String]) -> Result<()> {
    if keys.len() > 1024 {
        anyhow::bail!("CSV supports at most 1024 data columns; use JSONL or split the schema");
    }
    let mut encoded = std::collections::BTreeSet::new();
    for key in keys {
        if key.is_empty()
            || key.len() > 256
            || !encoded.insert(crate::export::spreadsheet_text(key))
        {
            anyhow::bail!("CSV field headings are empty, oversized or collide after formula protection; rename fields or use JSONL");
        }
    }
    Ok(())
}

impl CsvSink {
    fn open(path: &Path) -> Result<Self> {
        Self::open_with_fields(path, None)
    }

    fn open_with_fields(path: &Path, configured: Option<Vec<String>>) -> Result<Self> {
        if let Some(keys) = &configured {
            validate_csv_keys(keys)?;
        }
        ensure_output_parent(path)?;
        let exists = path.exists() && std::fs::metadata(path)?.len() > 0;
        let keys = if exists {
            let file = crate::fsutil::open_append(path)?;
            let mut reader = csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(crate::source_io::CappedReader::new(file, 1024 * 1024));
            let header = reader.records().next().context("CSV has no header")??;
            if header.len() < CSV_META.len() || !header.iter().take(7).eq(CSV_META) {
                anyhow::bail!("existing CSV header is not a harvest schema");
            }
            let keys: Vec<String> = header.iter().skip(7).map(String::from).collect();
            validate_csv_keys(&keys)?;
            if configured.as_ref().is_some_and(|want| {
                want.iter()
                    .any(|k| !keys.contains(&crate::export::spreadsheet_text(k)))
            }) {
                anyhow::bail!(
                    "CSV schema changed: use a new output file or JSONL followed by export"
                );
            }
            // Formula-neutralized field names are recovered from the known
            // configuration, not guessed. Ordinary field names are unchanged.
            if let Some(want) = configured {
                Some(
                    keys.iter()
                        .map(|k| {
                            want.iter()
                                .find(|w| crate::export::spreadsheet_text(w) == *k)
                                .cloned()
                                .unwrap_or_else(|| k.clone())
                        })
                        .collect(),
                )
            } else {
                Some(keys)
            }
        } else {
            configured
        };
        let file = crate::fsutil::open_append(path)?;
        Ok(Self {
            file: Mutex::new(CsvState {
                writer: csv::WriterBuilder::new().from_writer(file),
                keys,
                header_done: exists,
            }),
        })
    }
}

fn json_flat(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => crate::export::spreadsheet_text(s),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
fn csv_escape(s: &str) -> String {
    crate::export::csv_escape(s)
}

impl Sink for CsvSink {
    fn write(&self, rec: &Record) -> Result<()> {
        let mut g = self.file.lock().unwrap_or_else(|p| p.into_inner());
        let keys = g
            .keys
            .get_or_insert_with(|| rec.fields.keys().cloned().collect())
            .clone();
        if rec.fields.keys().any(|k| !keys.contains(k)) {
            anyhow::bail!("CSV schema changed: refusing an unaligned row; use JSONL + export or a new CSV file");
        }
        validate_csv_keys(&keys)?;
        if !g.header_done {
            let mut header: Vec<String> = CSV_META.iter().map(|s| s.to_string()).collect();
            header.extend(keys.iter().map(|k| crate::export::spreadsheet_text(k)));
            g.writer.write_record(header)?;
            g.header_done = true;
        }
        let text = crate::export::spreadsheet_text;
        let mut row = vec![
            text(&rec.run),
            text(&rec.target),
            text(&rec.url),
            rec.depth.to_string(),
            rec.ts.to_string(),
            text(&rec.fingerprint),
            text(&rec.egress),
        ];
        row.extend(
            keys.iter()
                .map(|k| rec.fields.get(k).map(json_flat).unwrap_or_default()),
        );
        g.writer.write_record(row)?;
        Ok(())
    }
    fn flush(&self) -> Result<()> {
        self.file
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .writer
            .flush()?;
        Ok(())
    }
}

/// Config-aware CSV schema: declare the union once, before any worker writes.
pub(crate) fn build_for_targets(
    cfg: &SinkCfg,
    targets: &[crate::config::TargetCfg],
) -> Result<(std::sync::Arc<dyn Sink>, PathBuf)> {
    if let SinkCfg::Csv { path } = cfg {
        let fields: std::collections::BTreeSet<String> = targets
            .iter()
            .flat_map(|t| t.extract.keys().cloned())
            .collect();
        Ok((
            std::sync::Arc::new(CsvSink::open_with_fields(
                path,
                Some(fields.into_iter().collect()),
            )?),
            path.clone(),
        ))
    } else {
        build(cfg)
    }
}

#[cfg(feature = "sqlite")]
pub struct SqliteSink {
    conn: Mutex<rusqlite::Connection>,
}

#[cfg(feature = "sqlite")]
impl SqliteSink {
    fn open(path: &Path) -> Result<Self> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        ensure_output_directory(parent)?;
        // Pin/check the final database entry with the same no-follow and
        // hard-link policy as the file sinks before SQLite resolves it by
        // pathname. SQLite's own NOFOLLOW flag below closes the final-link
        // race for the database file itself.
        crate::fsutil::reject_sqlite_sidecars(path)?;
        let database_guard = crate::fsutil::open_append(path)?;
        drop(database_guard);
        let conn = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        crate::fsutil::reject_sqlite_sidecars(path)?;
        conn.execute_batch(
            r#"CREATE TABLE IF NOT EXISTS records (
                 id INTEGER PRIMARY KEY,
                 run TEXT NOT NULL,
                 target TEXT NOT NULL,
                 url TEXT NOT NULL,
                 depth INTEGER NOT NULL,
                 ts INTEGER NOT NULL,
                 fingerprint TEXT NOT NULL,
                 egress TEXT NOT NULL,
                 payload TEXT NOT NULL
               );
               CREATE UNIQUE INDEX IF NOT EXISTS ux_records_fp ON records(run, target, fingerprint);"#,
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

#[cfg(feature = "sqlite")]
impl Sink for SqliteSink {
    fn flush(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let (busy, _, _): (i64, i64, i64) =
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        if busy != 0 {
            anyhow::bail!("SQLite checkpoint busy; a complete snapshot cannot be promised");
        }
        Ok(())
    }
    fn write(&self, rec: &Record) -> Result<()> {
        let payload = serde_json::to_string(&rec.fields)?;
        let c = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        c.execute(
            "INSERT OR IGNORE INTO records (run, target, url, depth, ts, fingerprint, egress, payload) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![rec.run, rec.target, rec.url, rec.depth as i64, rec.ts, rec.fingerprint, rec.egress, payload],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
