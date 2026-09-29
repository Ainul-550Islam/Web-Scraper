//! Developer-facing test harness: deterministic time, fixture builders, a
//! configurable mock HTTP origin, an in-memory sink, and small collectors.
//!
//! Compiled only for `cfg(test)` unit tests and when the `devtools` feature
//! is enabled (the crate declares itself as a dev-dependency with that
//! feature, so `tests/*.rs` see it too) — it never ships in a normal build.
//!
//! The mock origin deliberately models the *nasty* parts of the real
//! internet (delays, truncation, garbage, redirects) so security-adjacent
//! tests can exercise fetcher/sink code paths without any security bypass.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::sink::{Record, Sink};

// ------------------------------------------------------------------ clock

/// Deterministic clock helper: the ledger APIs take an explicit `now`
/// (`AlertLedger::note_at`), so tests pin time instead of sleeping.
#[derive(Debug)]
pub struct TestClock {
    now: AtomicU64,
}

impl TestClock {
    pub fn new(start: u64) -> Self {
        Self {
            now: AtomicU64::new(start),
        }
    }
    pub fn now(&self) -> u64 {
        self.now.load(Ordering::Relaxed)
    }
    pub fn advance(&self, secs: u64) -> u64 {
        self.now.fetch_add(secs, Ordering::Relaxed) + secs
    }
    pub fn set(&self, secs: u64) {
        self.now.store(secs, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------- fixture

/// A temp directory holding a written config plus its usual lanes
/// (out/, history). Drop removes everything.
pub struct TempFixture {
    pub dir: PathBuf,
    pub cfg_path: PathBuf,
}

impl TempFixture {
    /// `name` namespaces the dir; pid keeps parallel test runs apart.
    pub fn new(name: &str, base_url: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("harvest-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("out")).expect("create fixture dir");
        let cfg_path = dir.join("cfg.json");
        let cfg = fixture_config(name, base_url, &dir);
        std::fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap())
            .expect("write fixture config");
        Self { dir, cfg_path }
    }

    pub fn config(&self) -> crate::Config {
        crate::Config::load(&self.cfg_path).expect("fixture config must load")
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The canonical fixture config: one product-like target, generous limits,
/// robots off (the mock serves no robots), everything under `dir`.
pub fn fixture_config(name: &str, base_url: &str, dir: &Path) -> Value {
    json!({
        "name": name,
        "respect_robots": false,
        "concurrency": 2,
        "max_pages": 50,
        "request_timeout_secs": 3,
        "connect_timeout_secs": 2,
        "metrics_addr": "",
        "schedule": {
            "every_secs": 60,
            "runs": 1,
            "history_path": dir.join("out/runs.jsonl").to_string_lossy()
        },
        "output": {
            "kind": "jsonl",
            "path": dir.join("out/records.jsonl").to_string_lossy()
        },
        "targets": [{
            "id": "products",
            "start_url": format!("{base_url}/index.html"),
            "follow_links": true,
            "max_depth": 2,
            "extract": {
                "title": { "css": "h1" },
                "price": { "css": "p.price" }
            }
        }]
    })
}

// ------------------------------------------------------------- mock origin

/// What a mock route does when matched.
#[derive(Debug, Clone)]
pub enum Route {
    /// Full response with fixed body.
    Respond {
        status: u16,
        body: String,
        content_type: &'static str,
        headers: Vec<(String, String)>,
    },
    /// 301/302-style redirect (status, Location).
    Redirect(u16, String),
    /// Headers flushed, then the connection dies mid-body — the client must
    /// treat a truncated stream as a transport failure, not data.
    Cut {
        body_prefix: String,
        content_length_lie: bool,
    },
    /// Bytes that are not HTTP at all.
    Garbage,
    /// Fixed delay, then the inner route answers (timeout-path testing).
    Delay(Duration, Box<Route>),
    /// Transient fault injection, deterministic per route: the first
    /// `fail_first` requests to this route answer `code` (a retryable
    /// status), every later one answers `then`. Models flaky origins
    /// without a random seed that could make CI flakier than the product.
    Flaky {
        fail_first: usize,
        code: u16,
        then: Box<Route>,
    },
    /// Connection closed with no response bytes — the pure transport failure.
    Reset,
}

impl Route {
    pub fn ok(body: impl Into<String>) -> Route {
        Route::Respond {
            status: 200,
            body: body.into(),
            content_type: "text/html; charset=utf-8",
            headers: Vec::new(),
        }
    }
    pub fn status(code: u16, body: impl Into<String>) -> Route {
        Route::Respond {
            status: code,
            body: body.into(),
            content_type: "text/html; charset=utf-8",
            headers: Vec::new(),
        }
    }
    /// Fail the first `n` requests with `code`, then delegate to `then`.
    pub fn flaky(n: usize, code: u16, then: Route) -> Route {
        Route::Flaky {
            fail_first: n,
            code,
            then: Box::new(then),
        }
    }
    /// Never answer; hang for `d` first, then reset (client-timeout path).
    pub fn hang(d: Duration) -> Route {
        Route::Delay(d, Box::new(Route::Reset))
    }
}

/// Single-threaded mock HTTP/1.1 origin for tests. Longest-prefix route
/// match; unmatched paths answer a plain 404. Every request path is recorded
/// (`hits()`), so assertions observe what the engine actually did.
pub struct MockServer {
    pub addr: String,
    inner: Arc<MockInner>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

struct MockInner {
    routes: Vec<(String, Route)>,
    hits: Mutex<Vec<String>>,
    /// Requests served *per route slot* (post-sort index): what Flaky
    /// counts against. Single accept-loop ⇒ exact, no race.
    route_hits: Mutex<Vec<usize>>,
}

impl MockServer {
    /// `routes`: (path prefix, action) pairs, matched longest-first.
    pub fn start(routes: Vec<(&str, Route)>) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("mock addr").to_string();
        let mut routes: Vec<(String, Route)> = routes
            .into_iter()
            .map(|(p, r)| (p.to_string(), r))
            .collect();
        routes.sort_by_key(|(p, _)| std::cmp::Reverse(p.len()));
        let n = routes.len();
        let inner = Arc::new(MockInner {
            routes,
            hits: Mutex::new(Vec::new()),
            route_hits: Mutex::new(vec![0; n + 1]),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (i2, s2) = (inner.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            for conn in listener.incoming() {
                if s2.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(mut sock) = conn else { continue };
                let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
                let _ = sock.set_write_timeout(Some(Duration::from_secs(5)));
                let Some(path) = read_request_line(&mut sock) else {
                    continue;
                };
                i2.hits.lock().unwrap().push(path.clone());
                let matched = i2
                    .routes
                    .iter()
                    .enumerate()
                    .find(|(_, (p, _))| path.starts_with(p.as_str()))
                    .map(|(i, (_, r))| (i, r.clone()));
                let (slot, route) =
                    matched.unwrap_or((i2.routes.len(), Route::status(404, "not found")));
                let hit_n = {
                    let mut rh = i2.route_hits.lock().unwrap();
                    let lim = rh.len() - 1;
                    let n = &mut rh[slot.min(lim)];
                    *n += 1;
                    *n - 1
                };
                let _ = serve_route(&mut sock, &route, hit_n);
            }
        });
        MockServer {
            addr,
            inner,
            stop,
            handle: Some(handle),
        }
    }

    pub fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Request paths seen so far, in order.
    pub fn hits(&self) -> Vec<String> {
        self.inner.hits.lock().unwrap().clone()
    }

    pub fn hit_counts(&self) -> BTreeMap<String, usize> {
        let mut m = BTreeMap::new();
        for h in self.hits() {
            *m.entry(h).or_insert(0usize) += 1;
        }
        m
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // one self-connect to wake the accept loop, then join
        if let Ok(mut s) = TcpStream::connect(&self.addr) {
            let _ = s.write_all(b"GET /__shutdown HTTP/1.1\r\nhost: m\r\n\r\n");
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn read_request_line(sock: &mut TcpStream) -> Option<String> {
    let mut buf = [0u8; 4096];
    let mut filled = 0usize;
    loop {
        match sock.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => {
                filled += n;
                if buf[..filled].windows(4).any(|w| w == b"\r\n\r\n") || filled >= buf.len() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let head = String::from_utf8_lossy(&buf[..filled]).into_owned();
    head.lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .map(str::to_string)
}

fn serve_route(sock: &mut TcpStream, route: &Route, hit_n: usize) -> std::io::Result<()> {
    match route {
        Route::Respond {
            status,
            body,
            content_type,
            headers,
        } => {
            let mut extra = String::new();
            for (k, v) in headers {
                extra.push_str(&format!("{k}: {v}\r\n"));
            }
            write!(
                sock,
                "HTTP/1.1 {status} mock\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n{extra}\r\n",
                body.len()
            )?;
            sock.write_all(body.as_bytes())
        }
        Route::Redirect(code, to) => {
            write!(
                sock,
                "HTTP/1.1 {code} redirect\r\nlocation: {to}\r\ncontent-length: 0\r\n\r\n"
            )
        }
        Route::Cut {
            body_prefix,
            content_length_lie,
        } => {
            let declared = if *content_length_lie {
                body_prefix.len() + 1000
            } else {
                body_prefix.len()
            };
            write!(
                sock,
                "HTTP/1.1 200 mock\r\ncontent-type: text/html\r\ncontent-length: {declared}\r\n\r\n"
            )?;
            sock.write_all(body_prefix.as_bytes())?;
            // die mid-stream: client must see EOF before the declared length
            let _ = sock.shutdown(std::net::Shutdown::Both);
            Ok(())
        }
        Route::Garbage => sock.write_all(b"NOT-HTTP \x01\x02 garbage garbage"),
        Route::Reset => {
            // Close without answering: the client sees a connection that
            // died with no response — the canonical transport failure.
            let _ = sock.shutdown(std::net::Shutdown::Both);
            Ok(())
        }
        Route::Flaky {
            fail_first,
            code,
            then,
        } => {
            if hit_n < *fail_first {
                write!(
                    sock,
                    "HTTP/1.1 {code} mock\r\ncontent-type: text/plain\r\ncontent-length: 8\r\n\r\nflaky-err"
                )
            } else {
                serve_route(sock, then, hit_n - fail_first)
            }
        }
        Route::Delay(d, inner) => {
            std::thread::sleep(*d);
            serve_route(sock, inner, hit_n)
        }
    }
}

/// A page that fans out into N `/product/<i>` links — the shape every
/// engine-layer test has always crawled.
pub fn fanout_page(n: usize) -> String {
    let links: Vec<String> = (0..n)
        .map(|i| format!("<a href=\"/product/{i}\">p{i}</a>"))
        .collect();
    format!(
        "<html><body><h1>Index</h1><p class=price>$1.00</p>{}</body></html>",
        links.join("")
    )
}

pub fn product_page(i: usize) -> String {
    format!("<html><body><h1>P{i}</h1><p class=price>${i}.50</p></body></html>")
}

// ------------------------------------------------------------------ sink

/// In-memory sink: captures every record for assertions; also counts flushes.
#[derive(Default, Debug)]
pub struct InMemorySink {
    pub records: Mutex<Vec<Record>>,
    pub flushes: AtomicU64,
}

impl InMemorySink {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn snapshot(&self) -> Vec<Value> {
        let g = self.records.lock().unwrap();
        g.iter()
            .map(|r| {
                let mut m = Map::new();
                m.insert("url".into(), r.url.clone().into());
                m.insert("target".into(), r.target.clone().into());
                for (k, v) in &r.fields {
                    m.insert(k.clone(), v.clone());
                }
                Value::Object(m)
            })
            .collect()
    }
}

impl Sink for InMemorySink {
    fn write(&self, rec: &Record) -> anyhow::Result<()> {
        self.records.lock().unwrap().push(Record {
            run: rec.run.clone(),
            target: rec.target.clone(),
            url: rec.url.clone(),
            depth: rec.depth,
            ts: rec.ts,
            fingerprint: rec.fingerprint.clone(),
            egress: rec.egress.clone(),
            fields: rec.fields.clone(),
        });
        Ok(())
    }
    fn flush(&self) -> anyhow::Result<()> {
        self.flushes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

// ------------------------------------------------------------- collectors

/// Read an events JSONL file into values (missing file => empty).
pub fn collect_events(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Wait until a predicate holds or the deadline passes — the only allowed
/// sleep in tests (no fixed `sleep` races).
pub fn wait_for(deadline: Duration, mut pred: impl FnMut() -> bool) -> bool {
    let t0 = Instant::now();
    loop {
        if pred() {
            return true;
        }
        if t0.elapsed() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Snapshot metrics as JSON with derived means attached (the arithmetic the
/// ops dashboards do; kept here so tests assert semantics, not formulas).
pub fn metrics_view(m: &crate::metrics::Metrics) -> Value {
    let mut j = m.display_json();
    if let Some(obj) = j.as_object_mut() {
        let req = obj["requests_total"].as_u64().unwrap_or(0).max(1);
        let w = obj["storage_writes"].as_u64().unwrap_or(0).max(1);
        let api = obj["api_requests_total"].as_u64().unwrap_or(0).max(1);
        obj.insert(
            "avg_request_ms".into(),
            json!(obj["request_ms_total"].as_u64().unwrap_or(0) as f64 / req as f64),
        );
        obj.insert(
            "avg_storage_write_ms".into(),
            json!(obj["storage_write_ms_total"].as_u64().unwrap_or(0) as f64 / w as f64),
        );
        obj.insert(
            "avg_api_ms".into(),
            json!(obj["api_request_ms_total"].as_u64().unwrap_or(0) as f64 / api as f64),
        );
    }
    j
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests;
