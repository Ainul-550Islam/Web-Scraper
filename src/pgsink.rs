//! Postgres sink (feature `postgres`). Same off-thread pattern as the HTTP
//! sink: a dedicated std thread owns a current-thread tokio runtime plus the
//! `tokio_postgres` client/connection pair (both must be built and dropped
//! outside the engine's runtime). `flush()` reports accumulated insert
//! errors so a broken DB fails the run loudly, per batch semantics elsewhere.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::sink::{Record, Sink};

enum Cmd {
    Push(Record),
    Flush(Sender<Result<()>>),
    Shutdown,
}

pub struct PgSink {
    tx: Sender<Cmd>,
    pending: Arc<AtomicUsize>,
    pub rows_written: Arc<AtomicU64>,
}

struct PgThread {
    dsn: String,
    rows_written: Arc<AtomicU64>,
    pending: Arc<AtomicUsize>,
}

const DDL: &str = r#"CREATE TABLE IF NOT EXISTS harvest_records (
    id BIGSERIAL PRIMARY KEY,
    run TEXT NOT NULL,
    target TEXT NOT NULL,
    url TEXT NOT NULL,
    depth INT NOT NULL,
    ts BIGINT NOT NULL,
    fingerprint TEXT NOT NULL,
    egress TEXT NOT NULL,
    payload JSONB NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS ux_harvest_records
    ON harvest_records (run, target, fingerprint);"#;

impl PgThread {
    fn run(self, rx: Receiver<Cmd>) {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                fail_all(&rx, &format!("pg sink runtime: {e}"));
                return;
            }
        };
        let state = rt.block_on(async {
            match tokio_postgres::connect(&self.dsn, tokio_postgres::NoTls).await {
                Ok((client, conn)) => {
                    rt.spawn(async move {
                        if let Err(e) = conn.await {
                            tracing::warn!(err = %e, "pg connection closed");
                        }
                    });
                    match client.batch_execute(DDL).await {
                        Ok(()) => Ok(client),
                        Err(e) => Err(format!("pg ddl: {e}")),
                    }
                }
                Err(e) => Err(format!("pg connect: {e}")),
            }
        });
        // Degraded state is STICKY: once the lane cannot deliver (connect or
        // insert died), every later record is counted as dropped and every
        // flush keeps failing. Clearing the error after one flush (take())
        // would silently swallow everything after the first report.
        let (mut client, mut degraded): (Option<tokio_postgres::Client>, Option<String>) =
            match state {
                Ok(c) => (Some(c), None),
                Err(msg) => (None, Some(msg)),
            };
        let mut rows = 0u64;
        let mut dropped = 0u64;

        loop {
            match rx.recv() {
                Ok(Cmd::Push(rec)) => {
                    if let Some(pg) = client.as_ref() {
                        let res = rt.block_on(pg.execute(
                            "INSERT INTO harvest_records \
                             (run, target, url, depth, ts, fingerprint, egress, payload) \
                             VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING",
                            &[
                                &rec.run,
                                &rec.target,
                                &rec.url,
                                &(rec.depth as i32),
                                &rec.ts,
                                &rec.fingerprint,
                                &rec.egress,
                                &tokio_postgres::types::Json(&rec.fields),
                            ],
                        ));
                        match res {
                            Ok(n) => rows += n,
                            Err(e) => {
                                degraded = Some(format!("pg insert: {e}"));
                                client = None; // stop pretending; count the loss
                            }
                        }
                    } else {
                        dropped += 1;
                        let base = degraded
                            .clone()
                            .unwrap_or_else(|| "pg sink degraded".to_string());
                        degraded = Some(format!("{base}: {dropped} records dropped"));
                    }
                    self.pending.fetch_sub(1, Ordering::SeqCst);
                }
                Ok(Cmd::Flush(ack)) => {
                    self.pending.store(0, Ordering::SeqCst);
                    let _ = ack.send(match &degraded {
                        Some(e) => Err(anyhow::anyhow!(e.clone())),
                        None => Ok(()),
                    });
                }
                Ok(Cmd::Shutdown) | Err(_) => break,
            }
        }
        if rows > 0 {
            self.rows_written.fetch_add(rows, Ordering::Relaxed);
        }
        // Client + runtime drop on this plain thread — never a tokio worker.
        drop(client);
    }
}

fn fail_all(rx: &Receiver<Cmd>, msg: &str) {
    while let Ok(cmd) = rx.try_recv() {
        if let Cmd::Flush(ack) = cmd {
            let _ = ack.send(Err(anyhow::anyhow!("{msg}")));
        }
    }
}

impl PgSink {
    pub fn open(dsn: String) -> Result<Self> {
        let (tx, rx) = mpsc_pair();
        let pending: Arc<AtomicUsize> = Default::default();
        let rows_written: Arc<AtomicU64> = Default::default();
        let st = PgThread {
            dsn,
            rows_written: rows_written.clone(),
            pending: pending.clone(),
        };
        std::thread::Builder::new()
            .name("harvest-pg-sink".into())
            .spawn(move || st.run(rx))
            .context("spawn pg sink thread")?;
        Ok(Self {
            tx,
            pending,
            rows_written,
        })
    }

    pub fn buffered(&self) -> usize {
        self.pending.load(Ordering::SeqCst)
    }
}

fn mpsc_pair() -> (Sender<Cmd>, Receiver<Cmd>) {
    std::sync::mpsc::channel()
}

impl Sink for PgSink {
    fn write(&self, rec: &Record) -> Result<()> {
        self.pending.fetch_add(1, Ordering::SeqCst);
        self.tx
            .send(Cmd::Push(rec.clone()))
            .map_err(|_| anyhow::anyhow!("pg sink thread gone"))
    }

    fn flush(&self) -> Result<()> {
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        self.tx
            .send(Cmd::Flush(ack_tx))
            .map_err(|_| anyhow::anyhow!("pg sink thread gone"))?;
        ack_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|e| anyhow::anyhow!("pg flush ack lost: {e}"))?
    }
}

impl Drop for PgSink {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

#[cfg(test)]
mod tests;
