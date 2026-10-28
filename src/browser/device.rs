//! Explicit device profiles for consistent viewport/locale configuration.
use super::config::{BrowserEngine, Viewport};
use super::error::{BrowserError, BrowserResult};
use super::profile::BrowserProfile;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceClass {
    Desktop,
    Tablet,
    Mobile,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub id: String,
    pub class: DeviceClass,
    pub engine: BrowserEngine,
    pub viewport: Viewport,
    pub locale: String,
    pub timezone: Option<String>,
    pub user_agent: Option<String>,
}
impl DeviceProfile {
    pub fn validate(&self) -> BrowserResult<()> {
        if self.id.is_empty() || self.id.len() > 128 {
            return Err(BrowserError::InvalidInput("device profile id"));
        }
        if !(240..=7680).contains(&self.viewport.width)
            || !(240..=4320).contains(&self.viewport.height)
        {
            return Err(BrowserError::InvalidInput("device viewport"));
        }
        if self.locale.is_empty() || self.locale.len() > 64 {
            return Err(BrowserError::InvalidInput("device locale"));
        }
        Ok(())
    }
    pub fn into_profile(self, created_at_ms: u64) -> BrowserProfile {
        BrowserProfile {
            id: self.id,
            version: 1,
            engine: self.engine,
            viewport: self.viewport,
            locale: self.locale,
            timezone: self.timezone,
            user_agent: self.user_agent,
            created_at_ms,
            expires_at_ms: None,
        }
    }
}
pub fn standard_profiles() -> Vec<DeviceProfile> {
    vec![
        DeviceProfile {
            id: "desktop-1365x768".into(),
            class: DeviceClass::Desktop,
            engine: BrowserEngine::Chrome,
            viewport: Viewport::default(),
            locale: "en-US".into(),
            timezone: None,
            user_agent: None,
        },
        DeviceProfile {
            id: "tablet-1024x768".into(),
            class: DeviceClass::Tablet,
            engine: BrowserEngine::Chrome,
            viewport: Viewport {
                width: 1024,
                height: 768,
                device_scale_factor: 1.0,
            },
            locale: "en-US".into(),
            timezone: None,
            user_agent: None,
        },
        DeviceProfile {
            id: "mobile-390x844".into(),
            class: DeviceClass::Mobile,
            engine: BrowserEngine::Chrome,
            viewport: Viewport {
                width: 390,
                height: 844,
                device_scale_factor: 1.0,
            },
            locale: "en-US".into(),
            timezone: None,
            user_agent: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standard_profiles_are_consistent() {
        for p in standard_profiles() {
            p.validate().unwrap();
        }
    }
}
