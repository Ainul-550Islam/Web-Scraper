//! A disposable browser context boundary. WebDriver implementations generally
//! represent each isolated context as a new session; this wrapper owns that
//! boundary and makes cleanup explicit.
use std::sync::Arc;

use super::error::{BrowserError, BrowserResult};
use super::session::BrowserSession;

pub struct BrowserContext {
    key: String,
    session: Arc<BrowserSession>,
    disposed: bool,
}

impl BrowserContext {
    pub fn new(key: impl Into<String>, session: Arc<BrowserSession>) -> BrowserResult<Self> {
        let key = key.into();
        if key.is_empty() || key.len() > 128 || key.chars().any(char::is_control) {
            return Err(BrowserError::InvalidInput("context key"));
        }
        Ok(Self {
            key,
            session,
            disposed: false,
        })
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn session(&self) -> &Arc<BrowserSession> {
        &self.session
    }
    pub fn is_disposed(&self) -> bool {
        self.disposed
    }
    pub async fn dispose(&mut self) -> BrowserResult<()> {
        if !self.disposed {
            self.session.close().await?;
            self.disposed = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::config::BrowserConfig;
    use crate::browser::session::BrowserSession;

    /// A session that is never driven: the context key is validated before the
    /// session is touched, so the key rules can be tested without a live
    /// WebDriver endpoint.
    fn idle_session() -> Arc<BrowserSession> {
        Arc::new(
            BrowserSession::new(
                "ctx-test".to_string(),
                "http://127.0.0.1:1".to_string(),
                "rt-test".to_string(),
                reqwest::Client::new(),
                &BrowserConfig::default(),
            )
            .expect("a test session"),
        )
    }

    #[test]
    fn context_keys_do_not_accept_controls() {
        let session = idle_session();
        // Control characters and an empty key are refused; a valid key is
        // accepted and carries the session through unchanged.
        assert!(super::BrowserContext::new("a\nb", session.clone()).is_err());
        assert!(super::BrowserContext::new("", session.clone()).is_err());
        assert!(super::BrowserContext::new("x", session.clone()).is_ok());
        // Overlong keys are refused too.
        assert!(super::BrowserContext::new("x".repeat(200), session).is_err());
    }
}
