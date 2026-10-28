//! Reusable WebDriver session handle with serialized, bounded commands.
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use reqwest::{Client, Method, StatusCode};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use super::config::BrowserConfig;
use super::error::{BrowserError, BrowserResult};
use super::page::now_ms;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLifecycle {
    Active,
    Closing,
    Closed,
}

/// A W3C WebDriver session. Its command lock prevents concurrent navigation
/// or extraction from corrupting one another when a target reuses a session.
pub struct BrowserSession {
    id: String,
    endpoint: String,
    runtime_id: String,
    client: Client,
    command_timeout: Duration,
    max_response_bytes: usize,
    created_at_ms: u64,
    closed: AtomicBool,
    command_lock: Mutex<()>,
}

impl BrowserSession {
    pub(crate) fn new(
        id: String,
        endpoint: String,
        runtime_id: String,
        client: Client,
        cfg: &BrowserConfig,
    ) -> BrowserResult<Self> {
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(BrowserError::Protocol { status: 502 });
        }
        Ok(Self {
            id,
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            runtime_id,
            client,
            command_timeout: Duration::from_millis(cfg.command_timeout_ms),
            max_response_bytes: cfg
                .max_html_bytes
                .max(cfg.max_screenshot_bytes)
                .saturating_add(2 * 1024 * 1024),
            created_at_ms: now_ms(),
            closed: AtomicBool::new(false),
            command_lock: Mutex::new(()),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn runtime_id(&self) -> &str {
        &self.runtime_id
    }
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }
    pub fn lifecycle(&self) -> SessionLifecycle {
        if self.closed.load(Ordering::Acquire) {
            SessionLifecycle::Closed
        } else {
            SessionLifecycle::Active
        }
    }

    pub(crate) fn base_session_url(&self) -> String {
        format!("{}/session/{}", self.endpoint, self.id)
    }

    pub(crate) async fn command(
        &self,
        method: Method,
        suffix: &str,
        payload: Option<Value>,
    ) -> BrowserResult<Value> {
        if self.closed.load(Ordering::Acquire) {
            return Err(BrowserError::Shutdown);
        }
        if suffix.starts_with('/') || suffix.contains("..") || suffix.chars().any(char::is_control)
        {
            return Err(BrowserError::InvalidInput("command path"));
        }
        let _guard = self.command_lock.lock().await;
        if self.closed.load(Ordering::Acquire) {
            return Err(BrowserError::Shutdown);
        }
        let url = if suffix.is_empty() {
            self.base_session_url()
        } else {
            format!("{}/{}", self.base_session_url(), suffix)
        };
        let mut req = self
            .client
            .request(method, url)
            .timeout(self.command_timeout);
        if let Some(value) = payload {
            req = req.json(&value);
        }
        let response = req.send().await.map_err(|e| {
            if e.is_timeout() {
                BrowserError::Timeout {
                    operation: "command",
                }
            } else {
                BrowserError::Connection
            }
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(map_protocol_error(status));
        }
        if response
            .content_length()
            .is_some_and(|n| n as usize > self.max_response_bytes)
        {
            return Err(BrowserError::ContentLimit {
                kind: "driver response",
                limit: self.max_response_bytes,
            });
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|_| BrowserError::Connection)?;
        if bytes.len() > self.max_response_bytes {
            return Err(BrowserError::ContentLimit {
                kind: "driver response",
                limit: self.max_response_bytes,
            });
        }
        let body: Value = serde_json::from_slice(&bytes).map_err(|_| BrowserError::Protocol {
            status: status.as_u16(),
        })?;
        if let Some(err) = body.pointer("/value/error").and_then(Value::as_str) {
            return Err(match err {
                "no such element" => BrowserError::NotFound("element"),
                "invalid session id" => BrowserError::Shutdown,
                "timeout" => BrowserError::Timeout {
                    operation: "command",
                },
                "javascript error" => {
                    BrowserError::Javascript("driver reported a script error".into())
                }
                _ => BrowserError::Protocol { status: 500 },
            });
        }
        Ok(body.get("value").cloned().unwrap_or(body))
    }

    pub async fn navigate(&self, url: &str, timeout_ms: u64) -> BrowserResult<()> {
        let parsed = url::Url::parse(url).map_err(|_| BrowserError::InvalidInput("URL"))?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(BrowserError::Policy(
                "only credential-free http(s) URLs are allowed".into(),
            ));
        }
        let old = self.command_timeout;
        let _ = old;
        let result = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            self.command(Method::POST, "url", Some(json!({"url": url}))),
        )
        .await;
        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(BrowserError::Timeout {
                operation: "navigation",
            }),
        }
    }

    pub async fn current_url(&self) -> BrowserResult<String> {
        self.command(Method::GET, "url", None)
            .await?
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or(BrowserError::Protocol { status: 502 })
    }

    pub async fn title(&self) -> BrowserResult<String> {
        self.command(Method::GET, "title", None)
            .await?
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or(BrowserError::Protocol { status: 502 })
    }

    pub async fn page_source(&self, limit: usize) -> BrowserResult<String> {
        let value = self.command(Method::GET, "source", None).await?;
        let source = value
            .as_str()
            .ok_or(BrowserError::Protocol { status: 502 })?;
        if source.len() > limit {
            return Err(BrowserError::ContentLimit {
                kind: "HTML snapshot",
                limit,
            });
        }
        Ok(source.to_owned())
    }

    pub async fn cookies(&self) -> BrowserResult<Vec<super::cookies::BrowserCookie>> {
        let value = self.command(Method::GET, "cookie", None).await?;
        let list = value
            .as_array()
            .ok_or(BrowserError::Protocol { status: 502 })?;
        if list.len() > 256 {
            return Err(BrowserError::ContentLimit {
                kind: "browser cookies",
                limit: 256,
            });
        }
        list.iter()
            .map(super::cookies::from_webdriver_value)
            .collect()
    }

    pub async fn set_cookie(&self, cookie: &super::cookies::BrowserCookie) -> BrowserResult<()> {
        let current = self.current_url().await?;
        let host = url::Url::parse(&current)
            .map_err(|_| BrowserError::Protocol { status: 502 })?
            .host_str()
            .ok_or(BrowserError::Protocol { status: 502 })?
            .to_owned();
        super::cookies::validate_for_session(cookie, &host)?;
        let payload = super::cookies::to_webdriver_value(cookie);
        self.command(
            Method::POST,
            "cookie",
            Some(serde_json::json!({"cookie":payload})),
        )
        .await?;
        Ok(())
    }

    pub async fn delete_cookie(&self, name: &str) -> BrowserResult<()> {
        if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
            return Err(BrowserError::InvalidInput("cookie name"));
        }
        self.command(
            Method::DELETE,
            &format!("cookie/{}", urlencoding_safe(name)),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn execute(
        &self,
        script: &str,
        args: Vec<Value>,
        max_bytes: usize,
    ) -> BrowserResult<Value> {
        if script.is_empty() || script.len() > 16_384 || args.len() > 64 {
            return Err(BrowserError::InvalidInput("JavaScript payload"));
        }
        let value = self
            .command(
                Method::POST,
                "execute/sync",
                Some(json!({"script": script, "args": args})),
            )
            .await?;
        if serde_json::to_vec(&value)
            .map(|b| b.len() > max_bytes)
            .unwrap_or(true)
        {
            return Err(BrowserError::ContentLimit {
                kind: "JavaScript result",
                limit: max_bytes,
            });
        }
        Ok(value)
    }

    pub async fn close(&self) -> BrowserResult<()> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let url = self.base_session_url();
        let response =
            tokio::time::timeout(self.command_timeout, self.client.delete(url).send()).await;
        match response {
            Ok(Ok(r)) if r.status().is_success() || r.status() == StatusCode::NOT_FOUND => Ok(()),
            Ok(Ok(_)) => Err(BrowserError::Shutdown),
            Ok(Err(_)) => Err(BrowserError::Shutdown),
            Err(_) => Err(BrowserError::Timeout {
                operation: "shutdown",
            }),
        }
    }
}

fn urlencoding_safe(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn map_protocol_error(status: StatusCode) -> BrowserError {
    if status == StatusCode::NOT_FOUND {
        BrowserError::NotFound("WebDriver endpoint")
    } else if status.is_server_error() {
        BrowserError::Crashed
    } else {
        BrowserError::Protocol {
            status: status.as_u16(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsafe_session_id() {
        let cfg = BrowserConfig::default();
        let client = Client::new();
        assert!(BrowserSession::new(
            "../x".into(),
            "http://127.0.0.1:4444".into(),
            "r".into(),
            client,
            &cfg
        )
        .is_err());
    }
}
