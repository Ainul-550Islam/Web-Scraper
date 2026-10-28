//! Reproducible browser profile model. This is configuration consistency,
//! not fingerprint spoofing or randomized identity generation.
use super::config::{BrowserEngine, Viewport};
use super::error::{BrowserError, BrowserResult};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserProfile {
    pub id: String,
    pub version: u32,
    pub engine: BrowserEngine,
    pub viewport: Viewport,
    pub locale: String,
    pub timezone: Option<String>,
    pub user_agent: Option<String>,
    pub created_at_ms: u64,
    pub expires_at_ms: Option<u64>,
}
impl BrowserProfile {
    pub fn validate(&self) -> BrowserResult<()> {
        if self.id.is_empty()
            || self.id.len() > 128
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(BrowserError::InvalidInput("profile id"));
        }
        if self.version == 0 || self.locale.is_empty() || self.locale.len() > 64 {
            return Err(BrowserError::InvalidInput("profile version or locale"));
        }
        if !(320..=7680).contains(&self.viewport.width)
            || !(240..=4320).contains(&self.viewport.height)
        {
            return Err(BrowserError::InvalidInput("profile viewport"));
        }
        if self
            .user_agent
            .as_ref()
            .is_some_and(|s| s.len() > 512 || s.chars().any(char::is_control))
        {
            return Err(BrowserError::InvalidInput("profile user agent"));
        }
        Ok(())
    }
    pub fn expired(&self, now_ms: u64) -> bool {
        self.expires_at_ms.is_some_and(|e| e <= now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_validates_and_expires() {
        let p = BrowserProfile {
            id: "prod".into(),
            version: 1,
            engine: BrowserEngine::Chrome,
            viewport: Viewport::default(),
            locale: "en-US".into(),
            timezone: None,
            user_agent: None,
            created_at_ms: 1,
            expires_at_ms: Some(5),
        };
        p.validate().unwrap();
        assert!(p.expired(5));
    }
}
