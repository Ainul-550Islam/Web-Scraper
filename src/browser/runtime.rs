//! Optional WebDriver runtime lifecycle. A single instance owns its driver,
//! bounded session semaphore and all contexts; callers do not share globals.
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

use super::config::{BrowserConfig, BrowserEngine};
use super::error::{BrowserError, BrowserResult};
use super::page::{now_ms, PageObservation};
use super::session::BrowserSession;

struct ManagedSession {
    session: Arc<BrowserSession>,
    egress_id: Option<String>,
    _permit: OwnedSemaphorePermit,
}

/// Explicitly-owned browser runtime. Local mode starts a driver process on
/// loopback; remote mode connects to an operator-provided WebDriver endpoint.
#[derive(Clone)]
pub struct BrowserRuntime {
    inner: Arc<RuntimeInner>,
}

struct RuntimeInner {
    id: String,
    config: BrowserConfig,
    endpoint: String,
    client: Client,
    sessions: Mutex<HashMap<String, ManagedSession>>,
    capacity: Arc<Semaphore>,
    child: Mutex<Option<Child>>,
    active: AtomicBool,
}

impl BrowserRuntime {
    pub async fn launch(config: BrowserConfig) -> BrowserResult<Self> {
        config
            .validate()
            .map_err(|e| BrowserError::Config(e.to_string()))?;
        if !config.enabled {
            return Err(BrowserError::Policy("browser runtime is disabled".into()));
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(config.command_timeout_ms))
            .build()
            .map_err(|_| BrowserError::Connection)?;
        let (endpoint, child) = if let Some(url) = &config.webdriver_url {
            (url.trim_end_matches('/').to_owned(), None)
        } else {
            let executable = config
                .driver_executable
                .as_ref()
                .ok_or(BrowserError::MissingExecutable)?;
            if !executable.is_file() {
                return Err(BrowserError::MissingExecutable);
            }
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
                .map_err(|_| BrowserError::Launch("could not reserve a loopback port".into()))?;
            let port = listener
                .local_addr()
                .map_err(|_| BrowserError::Launch("could not read loopback port".into()))?
                .port();
            drop(listener);
            let mut cmd = Command::new(executable);
            match config.engine {
                BrowserEngine::Chrome => {
                    cmd.arg(format!("--port={port}"));
                }
                BrowserEngine::Firefox => {
                    cmd.arg(format!("--port={port}"));
                }
            }
            let child = cmd
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| BrowserError::Launch(super::error::safe_message(&e.to_string())))?;
            (format!("http://127.0.0.1:{port}"), Some(child))
        };
        let runtime_id = format!("browser-{}-{}", std::process::id(), now_ms());
        let inner = Arc::new(RuntimeInner {
            id: runtime_id,
            config: config.clone(),
            endpoint,
            client,
            sessions: Mutex::new(HashMap::new()),
            capacity: Arc::new(Semaphore::new(config.max_sessions)),
            child: Mutex::new(child),
            active: AtomicBool::new(true),
        });
        let runtime = Self { inner };
        runtime.wait_ready().await?;
        Ok(runtime)
    }

    pub fn id(&self) -> &str {
        &self.inner.id
    }
    pub fn config(&self) -> &BrowserConfig {
        &self.inner.config
    }
    pub fn active_sessions(&self) -> usize {
        self.inner.sessions.try_lock().map(|m| m.len()).unwrap_or(0)
    }
    pub async fn contains_session(&self, key: &str) -> bool {
        self.inner.sessions.lock().await.contains_key(key)
    }

    async fn wait_ready(&self) -> BrowserResult<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            if !self.inner.active.load(Ordering::Acquire) {
                return Err(BrowserError::Crashed);
            }
            if let Ok(response) = self
                .inner
                .client
                .get(format!("{}/status", self.inner.endpoint))
                .timeout(Duration::from_millis(750))
                .send()
                .await
            {
                if response.status().is_success() {
                    return Ok(());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(BrowserError::Timeout {
                    operation: "driver readiness",
                });
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Return an existing named session or create one. `egress_id` is recorded
    /// at creation and cannot be changed later, preventing silent mid-session
    /// route changes that would invalidate cookies or audit attribution.
    pub async fn session(
        &self,
        key: &str,
        egress_id: Option<&str>,
        proxy_capability: Option<Value>,
    ) -> BrowserResult<Arc<BrowserSession>> {
        validate_key(key)?;
        if !self.inner.active.load(Ordering::Acquire) {
            return Err(BrowserError::Shutdown);
        }
        {
            let sessions = self.inner.sessions.lock().await;
            if let Some(entry) = sessions.get(key) {
                if entry.egress_id.as_deref() != egress_id {
                    return Err(BrowserError::Policy(
                        "session egress is immutable; create a new isolated session to change it"
                            .into(),
                    ));
                }
                return Ok(entry.session.clone());
            }
        }
        let permit = self
            .inner
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| BrowserError::Policy("browser session capacity is full".into()))?;
        let mut sessions = self.inner.sessions.lock().await;
        if let Some(entry) = sessions.get(key) {
            if entry.egress_id.as_deref() != egress_id {
                return Err(BrowserError::Policy(
                    "session egress is immutable; create a new isolated session to change it"
                        .into(),
                ));
            }
            return Ok(entry.session.clone());
        }
        let caps = self.capabilities(proxy_capability);
        let response = tokio::time::timeout(
            Duration::from_millis(self.inner.config.command_timeout_ms),
            self.inner
                .client
                .post(format!("{}/session", self.inner.endpoint))
                .json(&caps)
                .send(),
        )
        .await
        .map_err(|_| BrowserError::Timeout {
            operation: "session creation",
        })?
        .map_err(|_| BrowserError::Connection)?;
        let status = response.status();
        if !status.is_success() {
            return Err(if status.is_server_error() {
                BrowserError::Crashed
            } else {
                BrowserError::ContextCreation
            });
        }
        let body: Value = response.json().await.map_err(|_| BrowserError::Protocol {
            status: status.as_u16(),
        })?;
        let session_id = body
            .pointer("/value/sessionId")
            .or_else(|| body.get("sessionId"))
            .and_then(Value::as_str)
            .ok_or(BrowserError::Protocol {
                status: StatusCode::BAD_GATEWAY.as_u16(),
            })?
            .to_owned();
        let session = Arc::new(BrowserSession::new(
            session_id,
            self.inner.endpoint.clone(),
            self.inner.id.clone(),
            self.inner.client.clone(),
            &self.inner.config,
        )?);
        sessions.insert(
            key.to_owned(),
            ManagedSession {
                session: session.clone(),
                egress_id: egress_id.map(ToOwned::to_owned),
                _permit: permit,
            },
        );
        Ok(session)
    }

    fn capabilities(&self, proxy: Option<Value>) -> Value {
        let mut args = self.inner.config.launch_args.clone();
        if self.inner.config.headless {
            let flag = match self.inner.config.engine {
                BrowserEngine::Chrome => "--headless=new",
                BrowserEngine::Firefox => "-headless",
            };
            if !args.iter().any(|arg| arg == flag) {
                args.push(flag.into());
            }
        }
        args.push(format!(
            "--window-size={},{}",
            self.inner.config.viewport.width, self.inner.config.viewport.height
        ));
        args.push(format!("--lang={}", self.inner.config.locale));
        if let Some(tz) = &self.inner.config.timezone {
            args.push(format!("--timezone={tz}"));
        }
        if let Some(ua) = &self.inner.config.user_agent_override {
            args.push(format!("--user-agent={ua}"));
        }
        let caps = match self.inner.config.engine {
            BrowserEngine::Chrome => {
                let mut options = json!({"args": args});
                if let Some(path) = &self.inner.config.browser_executable {
                    options["binary"] = json!(path);
                }
                json!({"browserName":"chrome", "goog:chromeOptions": options})
            }
            BrowserEngine::Firefox => {
                let mut options =
                    json!({"args": args.iter().map(|a| json!({"value":a})).collect::<Vec<_>>()});
                if let Some(path) = &self.inner.config.browser_executable {
                    options["binary"] = json!(path);
                }
                json!({"browserName":"firefox", "moz:firefoxOptions": options})
            }
        };
        let mut always = caps;
        if let Some(proxy) = proxy {
            always["proxy"] = proxy;
        }
        json!({"capabilities":{"alwaysMatch":always,"firstMatch":[{}]}})
    }

    pub async fn close_session(&self, key: &str) -> BrowserResult<bool> {
        let entry = self.inner.sessions.lock().await.remove(key);
        if let Some(entry) = entry {
            entry.session.close().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn shutdown(&self) -> BrowserResult<()> {
        if !self.inner.active.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        let entries = {
            let mut map = self.inner.sessions.lock().await;
            map.drain().map(|(_, v)| v).collect::<Vec<_>>()
        };
        let mut failed = false;
        for entry in entries {
            if entry.session.close().await.is_err() {
                failed = true;
            }
        }
        if let Some(mut child) = self.inner.child.lock().await.take() {
            if child.try_wait().ok().flatten().is_none() && child.kill().await.is_err() {
                failed = true;
            }
            let _ = child.wait().await;
        }
        if failed {
            Err(BrowserError::Shutdown)
        } else {
            Ok(())
        }
    }

    /// Capture a bounded observation after a page has been navigated. The
    /// optional body status is queried from the page's Performance API and is
    /// best-effort because cross-browser engines may report zero.
    pub async fn observe(
        &self,
        session: &BrowserSession,
        requested_url: &str,
    ) -> BrowserResult<PageObservation> {
        let mut page = PageObservation::new(session.id(), self.id(), requested_url);
        page.final_url = Some(session.current_url().await?);
        page.title = Some(session.title().await?);
        let html = session
            .page_source(self.inner.config.max_html_bytes)
            .await?;
        page.set_html_bounded(html, self.inner.config.max_html_bytes)
            .map_err(|_| BrowserError::ContentLimit {
                kind: "HTML snapshot",
                limit: self.inner.config.max_html_bytes,
            })?;
        let text = session
            .execute(
                "return document.body ? document.body.innerText : '';",
                Vec::new(),
                self.inner.config.max_text_bytes,
            )
            .await?;
        let text = text
            .as_str()
            .ok_or(BrowserError::Protocol { status: 502 })?
            .to_owned();
        page.set_text_bounded(text, self.inner.config.max_text_bytes)
            .map_err(|_| BrowserError::ContentLimit {
                kind: "visible text",
                limit: self.inner.config.max_text_bytes,
            })?;
        if let Ok(value) = session.execute("const e=performance.getEntriesByType('navigation')[0]; return e ? {status:e.responseStatus||0,type:document.contentType||null} : {status:0,type:document.contentType||null};", Vec::new(), 1024).await {
            page.status = value.get("status").and_then(Value::as_u64).filter(|n| *n > 0 && *n <= 599).map(|n| n as u16);
            page.content_type = value.get("type").and_then(Value::as_str).map(ToOwned::to_owned);
        }
        page.finish();
        Ok(page)
    }
}

fn validate_key(key: &str) -> BrowserResult<()> {
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
    {
        return Err(BrowserError::InvalidInput("session key"));
    }
    Ok(())
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        // `tokio::sync::Mutex::get_mut` hands back `&mut Option<Child>`
        // directly (it cannot fail: the guard is only held from `&mut self`).
        if let Some(child) = self.child.get_mut().as_mut() {
            let _ = child.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_keys_are_restricted() {
        assert!(validate_key("target-1.worker-0").is_ok());
        assert!(validate_key("../secret").is_err());
        assert!(validate_key("").is_err());
    }
}
