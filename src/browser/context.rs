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
    #[test]
    fn context_keys_do_not_accept_controls() {
        assert!(super::BrowserContext::new("a\nb", panic_session()).is_err());
    }
    fn panic_session() -> std::sync::Arc<super::BrowserSession> {
        panic!("key validation precedes session use")
    }
}
