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
    Push(Record, usize),
    Flush(Sender<Result<()>>),
    Finish(Sender<Result<()>>),
    Shutdown,
}

pub struct PgSink {
    worker: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    submit: std::sync::Mutex<()>,
    closed: std::sync::atomic::AtomicBool,
    deadline: Arc<crate::sink_lifecycle::DeliveryDeadline>,
    tx: Sender<Cmd>,
    pending: Arc<AtomicUsize>,
    pending_bytes: Arc<AtomicUsize>,
    pub rows_written: Arc<AtomicU64>,
}

struct PgThread {
    deadline: Arc<crate::sink_lifecycle::DeliveryDeadline>,
    config: tokio_postgres::Config,
    rows_written: Arc<AtomicU64>,
    pending: Arc<AtomicUsize>,
    pending_bytes: Arc<AtomicUsize>,
}

const DDL: &str = r#"SET statement_timeout = '5000ms';
SET lock_timeout = '5000ms';
CREATE TABLE IF NOT EXISTS harvest_records (
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
            let tls = match tokio_postgres_rustls::MakeRustlsConnect::with_native_certs() {
                Ok((tls, _)) => tls,
                Err(_) => return Err("pg connect: no usable platform CA certificates".to_string()),
            };
            match tokio::time::timeout(
                self.deadline.remaining(crate::sink_lifecycle::OP_LIMIT),
                self.config.connect(tls),
            )
            .await
            {
                Ok(Ok((client, conn))) => {
                    rt.spawn(async move {
                        if let Err(e) = conn.await {
                            tracing::warn!(err = %e, "pg connection closed");
                        }
                    });
                    match tokio::time::timeout(
                        self.deadline.remaining(crate::sink_lifecycle::OP_LIMIT),
                        client.batch_execute(DDL),
                    )
                    .await
                    {
                        Ok(Ok(())) => Ok(client),
                        Ok(Err(e)) => Err(format!("pg ddl: {e}")),
                        Err(_) => Err("pg ddl: timeout".to_string()),
                    }
                }
                Ok(Err(e)) => Err(format!("pg connect: {e}")),
                Err(_) => Err("pg connect: timeout".to_string()),
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
        let mut dropped = 0u64;

        loop {
            match rx.recv() {
                Ok(Cmd::Push(rec, queued_bytes)) => {
                    if self
                        .deadline
                        .remaining(crate::sink_lifecycle::OP_LIMIT)
                        .is_zero()
                        && degraded.is_none()
                    {
                        degraded = Some("PG aggregate drain deadline exceeded".into());
                        client = None;
                    }
                    if let Some(pg) = client.as_ref() {
                        let res = rt.block_on(async {
                            tokio::time::timeout(
                                self.deadline.remaining(crate::sink_lifecycle::OP_LIMIT),
                                pg.execute(
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
                                ),
                            )
                            .await
                        });
                        match res {
                            Ok(Ok(n)) => {
                                self.rows_written.fetch_add(n, Ordering::Relaxed);
                            }
                            other => {
                                dropped += 1;
                                degraded = Some(match other {
                                    Ok(Err(e)) => format!("pg insert: {e}"),
                                    _ => "pg insert: timeout".to_string(),
                                });
                                client = None; // stop pretending; count the loss
                            }
                        }
                    } else {
                        dropped += 1;
                        // Keep one stable reason. Appending every loss to the
                        // previous string grows quadratically under a DB outage.
                    }
                    self.pending.fetch_sub(1, Ordering::SeqCst);
                    self.pending_bytes.fetch_sub(queued_bytes, Ordering::SeqCst);
                }
                Ok(Cmd::Flush(ack)) => {
                    let _ = ack.send(match &degraded {
                        Some(e) => Err(anyhow::anyhow!("{e}: {dropped} records dropped")),
                        None => Ok(()),
                    });
                }
                Ok(Cmd::Finish(ack)) => {
                    let _ = ack.send(match &degraded {
                        Some(e) => Err(anyhow::anyhow!("{e}: {dropped} records dropped")),
                        None => Ok(()),
                    });
                    break;
                }
                Ok(Cmd::Shutdown) | Err(_) => break,
            }
        }
        // Client + runtime drop on this plain thread — never a tokio worker.
        drop(client);
    }
}

fn fail_all(rx: &Receiver<Cmd>, msg: &str) {
    while let Ok(cmd) = rx.try_recv() {
        if let Cmd::Flush(ack) | Cmd::Finish(ack) = cmd {
            let _ = ack.send(Err(anyhow::anyhow!("{msg}")));
        }
    }
}

fn connection_config(dsn: &str) -> Result<tokio_postgres::Config> {
    use tokio_postgres::config::{Host, SslMode};
    let mut cfg: tokio_postgres::Config = dsn
        .parse()
        .map_err(|_| anyhow::anyhow!("postgres sink: invalid DSN"))?;
    let remote = cfg.get_hosts().iter().any(|h| match h {
        Host::Tcp(h) => {
            !h.eq_ignore_ascii_case("localhost")
                && !h
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        }
        #[cfg(unix)]
        Host::Unix(_) => false,
    });
    if remote && cfg.get_ssl_mode() != SslMode::Require {
        anyhow::bail!(
            "remote postgres sink requires sslmode=require (verified TLS; no plaintext fallback)"
        );
    }
    cfg.connect_timeout(
        cfg.get_connect_timeout()
            .copied()
            .unwrap_or(crate::sink_lifecycle::OP_LIMIT)
            .min(crate::sink_lifecycle::OP_LIMIT),
    );
    Ok(cfg)
}

pub(crate) fn validate_dsn(dsn: &str) -> Result<()> {
    connection_config(dsn).map(|_| ())
}

impl PgSink {
    pub fn open(dsn: String) -> Result<Self> {
        let config = connection_config(&dsn)?;
        let (tx, rx) = mpsc_pair();
        let pending: Arc<AtomicUsize> = Default::default();
        let rows_written: Arc<AtomicU64> = Default::default();
        let pending_bytes: Arc<AtomicUsize> = Default::default();
        let deadline: Arc<crate::sink_lifecycle::DeliveryDeadline> = Default::default();
        let st = PgThread {
            deadline: deadline.clone(),
            config,
            pending_bytes: pending_bytes.clone(),
            rows_written: rows_written.clone(),
            pending: pending.clone(),
        };
        let worker = std::thread::Builder::new()
            .name("harvest-pg-sink".into())
            .spawn(move || st.run(rx))
            .context("spawn pg sink thread")?;
        Ok(Self {
            worker: std::sync::Mutex::new(Some(worker)),
            submit: Default::default(),
            closed: Default::default(),
            deadline,
            tx,
            pending,
            pending_bytes,
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
        let _submit = self.submit.lock().unwrap_or_else(|p| p.into_inner());
        if self.closed.load(Ordering::SeqCst) {
            anyhow::bail!("PG sink is closed");
        }
        let n = serde_json::to_vec(rec)?.len();
        crate::sink::reserve_buffer(&self.pending, &self.pending_bytes, n)?;
        if self.tx.send(Cmd::Push(rec.clone(), n)).is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            self.pending_bytes.fetch_sub(n, Ordering::SeqCst);
            anyhow::bail!("pg sink thread gone");
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
                .map_err(|_| anyhow::anyhow!("pg sink thread gone"))?;
            ack_rx
                .recv_timeout(
                    crate::sink_lifecycle::DRAIN_LIMIT + std::time::Duration::from_secs(1),
                )
                .map_err(|_| anyhow::anyhow!("PG flush acknowledgement deadline exceeded"))?
        })();
        self.deadline.end();
        result
    }
    fn finish(&self) -> Result<()> {
        let _submit = self.submit.lock().unwrap_or_else(|p| p.into_inner());
        self.closed.store(true, Ordering::SeqCst);
        self.deadline.begin();
        let result = (|| {
            let (ack_tx, ack_rx) = std::sync::mpsc::channel();
            self.tx
                .send(Cmd::Finish(ack_tx))
                .map_err(|_| anyhow::anyhow!("pg sink thread gone"))?;
            ack_rx
                .recv_timeout(
                    crate::sink_lifecycle::DRAIN_LIMIT + std::time::Duration::from_secs(1),
                )
                .map_err(|_| anyhow::anyhow!("PG finish acknowledgement deadline exceeded"))?
        })();
        if let Some(worker) = self.worker.lock().unwrap_or_else(|p| p.into_inner()).take() {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("PG sink worker panicked"))?;
        }
        result
    }
}

impl Drop for PgSink {
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

#[cfg(test)]
mod tests;
