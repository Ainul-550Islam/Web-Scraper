//! Pluggable result sinks: JSONL (default), CSV, SQLite (feature `sqlite`),
//! batched HTTP webhook, and null (for load tests). Sinks are append-only and
//! idempotent-safe: the engine dedups by fingerprint before writing.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Map;

use crate::config::SinkCfg;

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
}

#[derive(Default)]
pub struct NullSink;
impl Sink for NullSink {
    fn write(&self, _rec: &Record) -> Result<()> {
        Ok(())
    }
}

/// Batched webhook sink, fully off-thread: async callers only hand record
/// JSON to an unbounded mpsc. A dedicated std thread owns everything
/// `reqwest::blocking` related (a blocking client builds and drops its own
/// runtime, so it must never touch a tokio worker), batches POSTs, and
/// reports failures back through the `flush()` ack. `flush()` blocks the
/// calling task only at end-of-run, when nothing else needs it.
pub struct HttpSink {
    tx: std::sync::mpsc::Sender<Cmd>,
    pending: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub bytes_sent: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub batches_sent: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

enum Cmd {
    Push(String),
    Flush(std::sync::mpsc::Sender<Result<()>>),
    Shutdown,
}

struct SinkThread {
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
        let body = format!("[{}]", self.batch.join(","));
        self.batch.clear();
        let Some(client) = &self.client else {
            self.last_err = Some("sink thread: blocking client unavailable".into());
            self.dec_pending(count);
            return;
        };
        let len = body.len() as u64;
        let mut sent = false;
        for _ in 0..2 {
            let mut req = client
                .post(&self.url)
                .header("content-type", "application/json")
                .body(body.clone());
            if let Some(a) = &self.auth {
                req = req.header("authorization", a.clone());
            }
            match req.send() {
                Ok(r) if r.status().is_success() => {
                    sent = true;
                    break;
                }
                Ok(r) => {
                    self.last_err = Some(format!("sink POST {} returned {}", self.url, r.status()))
                }
                Err(e) => self.last_err = Some(format!("sink POST {} failed: {e}", self.url)),
            }
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        if sent {
            self.batches_sent
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.bytes_sent
                .fetch_add(len, std::sync::atomic::Ordering::Relaxed);
        }
        // Whether delivered or permanently failed, these are no longer buffered.
        self.dec_pending(count);
    }

    fn run(mut self, rx: std::sync::mpsc::Receiver<Cmd>) {
        self.client = match reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(self.timeout_secs.max(1)))
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
                    self.pending.store(0, std::sync::atomic::Ordering::SeqCst);
                    let _ = ack.send(match self.last_err.take() {
                        Some(e) => Err(anyhow::anyhow!(e)),
                        None => Ok(()),
                    });
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
            tx,
            pending: Default::default(),
            bytes_sent: Default::default(),
            batches_sent: Default::default(),
        };
        let st = SinkThread {
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
        };
        std::thread::Builder::new()
            .name("harvest-http-sink".into())
            .spawn(move || st.run(rx))
            .context("spawn sink thread")?;
        Ok(sink)
    }

    pub fn buffered(&self) -> usize {
        self.pending.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Sink for HttpSink {
    fn write(&self, rec: &Record) -> Result<()> {
        let line = serde_json::to_string(rec)?;
        self.pending
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.tx
            .send(Cmd::Push(line))
            .map_err(|_| anyhow::anyhow!("sink thread gone"))
    }

    fn flush(&self) -> Result<()> {
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        self.tx
            .send(Cmd::Flush(ack_tx))
            .map_err(|_| anyhow::anyhow!("sink thread gone"))?;
        ack_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|e| anyhow::anyhow!("sink flush ack lost: {e}"))?
    }
}

impl Drop for HttpSink {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
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
        std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
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
            std::fs::create_dir_all(&dir)?;
            let n = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = dir.join(format!(
                "part-{}-{}-{}.jsonl",
                rec.run,
                std::process::id(),
                n
            ));
            let f = OpenOptions::new().create(true).append(true).open(&path)?;
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
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).with_context(|| format!("mkdir {}", p.display()))?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
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
    file: Mutex<(File, bool)>,
}

impl CsvSink {
    fn open(path: &Path) -> Result<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).with_context(|| format!("mkdir {}", p.display()))?;
        }
        let exists = path.exists()
            && std::fs::metadata(path)
                .map(|m| m.len() > 0)
                .unwrap_or(false);
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Mutex::new((file, exists)),
        })
    }
}

fn json_flat(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

impl Sink for CsvSink {
    fn write(&self, rec: &Record) -> Result<()> {
        let mut g = self.file.lock().unwrap_or_else(|p| p.into_inner());
        let (f, header_done) = &mut *g;
        let keys: Vec<String> = rec.fields.keys().cloned().collect();
        if !*header_done {
            let mut header = vec![
                "run",
                "target",
                "url",
                "depth",
                "ts",
                "fingerprint",
                "egress",
            ];
            header.extend(keys.iter().map(String::as_str));
            let line = header
                .iter()
                .map(|h| csv_escape(h))
                .collect::<Vec<_>>()
                .join(",");
            writeln!(f, "{line}")?;
            *header_done = true;
        }
        let mut row = vec![
            rec.run.clone(),
            rec.target.clone(),
            rec.url.clone(),
            rec.depth.to_string(),
            rec.ts.to_string(),
            rec.fingerprint.clone(),
            rec.egress.clone(),
        ];
        for k in &keys {
            row.push(json_flat(&rec.fields[k]));
        }
        let line = row
            .iter()
            .map(|s| csv_escape(s))
            .collect::<Vec<_>>()
            .join(",");
        writeln!(f, "{line}")?;
        Ok(())
    }
    fn flush(&self) -> Result<()> {
        let mut g = self.file.lock().unwrap_or_else(|p| p.into_inner());
        g.0.flush()?;
        Ok(())
    }
}

#[cfg(feature = "sqlite")]
pub struct SqliteSink {
    conn: Mutex<rusqlite::Connection>,
}

#[cfg(feature = "sqlite")]
impl SqliteSink {
    fn open(path: &Path) -> Result<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let conn = rusqlite::Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
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
