//! Browser launch and target-workflow configuration with compatibility-safe defaults.
use std::path::PathBuf;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use url::Url;

use super::workflow::Workflow;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserEngine {
    #[default]
    Chrome,
    Firefox,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionIsolation {
    PerJob,
    PerTargetWorker,
}

impl Default for SessionIsolation {
    fn default() -> Self {
        Self::PerJob
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub device_scale_factor: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 1365,
            height: 768,
            device_scale_factor: 1.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DownloadPolicy {
    pub enabled: bool,
    pub max_file_bytes: usize,
    pub allowed_mime_types: Vec<String>,
}

impl Default for DownloadPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_file_bytes: 10 * 1024 * 1024,
            allowed_mime_types: vec![
                "application/pdf".into(),
                "image/png".into(),
                "image/jpeg".into(),
                "image/webp".into(),
                "text/plain".into(),
            ],
        }
    }
}

/// Process-wide browser runtime settings. Paths and WebDriver URLs may contain
/// no embedded credentials; secrets are resolved through `credentials.rs`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserConfig {
    pub enabled: bool,
    pub engine: BrowserEngine,
    pub webdriver_url: Option<String>,
    pub driver_executable: Option<PathBuf>,
    pub browser_executable: Option<PathBuf>,
    pub headless: bool,
    pub launch_args: Vec<String>,
    pub viewport: Viewport,
    pub locale: String,
    pub timezone: Option<String>,
    pub user_agent_override: Option<String>,
    pub navigation_timeout_ms: u64,
    pub command_timeout_ms: u64,
    pub max_sessions: usize,
    pub max_html_bytes: usize,
    pub max_text_bytes: usize,
    pub max_screenshot_bytes: usize,
    pub capture_dom: bool,
    pub capture_screenshot_on_failure: bool,
    pub allow_javascript: bool,
    pub session_isolation: SessionIsolation,
    /// Additional explicitly authorized hosts. Target start URLs remain the
    /// minimum scope; these entries only permit deliberate workflow navigation.
    pub authorized_hosts: Vec<String>,
    pub download: DownloadPolicy,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            engine: BrowserEngine::Chrome,
            webdriver_url: None,
            driver_executable: None,
            browser_executable: None,
            headless: true,
            launch_args: vec!["--disable-dev-shm-usage".into()],
            viewport: Viewport::default(),
            locale: "en-US".into(),
            timezone: None,
            user_agent_override: None,
            navigation_timeout_ms: 30_000,
            command_timeout_ms: 10_000,
            max_sessions: 4,
            max_html_bytes: 8 * 1024 * 1024,
            max_text_bytes: 2 * 1024 * 1024,
            max_screenshot_bytes: 8 * 1024 * 1024,
            capture_dom: true,
            capture_screenshot_on_failure: false,
            allow_javascript: false,
            session_isolation: SessionIsolation::PerJob,
            authorized_hosts: Vec::new(),
            download: DownloadPolicy::default(),
        }
    }
}

impl BrowserConfig {
    pub fn validate(&self) -> Result<()> {
        if !(320..=7680).contains(&self.viewport.width)
            || !(240..=4320).contains(&self.viewport.height)
        {
            bail!("browser.viewport dimensions are outside supported bounds");
        }
        if !self.viewport.device_scale_factor.is_finite()
            || !(0.25..=4.0).contains(&self.viewport.device_scale_factor)
        {
            bail!("browser.viewport.device_scale_factor must be finite and within 0.25..=4");
        }
        if !(1_000..=300_000).contains(&self.navigation_timeout_ms)
            || !(250..=120_000).contains(&self.command_timeout_ms)
        {
            bail!("browser timeouts are outside supported bounds");
        }
        if !(1..=64).contains(&self.max_sessions) {
            bail!("browser.max_sessions must be 1..=64");
        }
        if !(16 * 1024..=64 * 1024 * 1024).contains(&self.max_html_bytes) {
            bail!("browser.max_html_bytes must be 16 KiB..64 MiB");
        }
        if !(1024..=16 * 1024 * 1024).contains(&self.max_text_bytes) {
            bail!("browser.max_text_bytes must be 1 KiB..16 MiB");
        }
        if !(1024..=32 * 1024 * 1024).contains(&self.max_screenshot_bytes) {
            bail!("browser.max_screenshot_bytes must be 1 KiB..32 MiB");
        }
        if self.locale.is_empty()
            || self.locale.len() > 64
            || self.locale.chars().any(char::is_control)
        {
            bail!("browser.locale must be a short printable locale tag");
        }
        if let Some(tz) = &self.timezone {
            if tz.is_empty() || tz.len() > 64 || tz.chars().any(char::is_control) {
                bail!("browser.timezone must be a short printable IANA zone name");
            }
        }
        if let Some(ua) = &self.user_agent_override {
            if ua.is_empty() || ua.len() > 512 || ua.chars().any(char::is_control) {
                bail!("browser.user_agent_override must be a printable value of at most 512 bytes");
            }
        }
        if let Some(endpoint) = &self.webdriver_url {
            let url = Url::parse(endpoint)
                .map_err(|_| anyhow::anyhow!("browser.webdriver_url is invalid"))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                bail!("browser.webdriver_url must be an http(s) endpoint without credentials, query, or fragment");
            }
        }
        if self.enabled && self.webdriver_url.is_none() && self.driver_executable.is_none() {
            bail!("browser.enabled requires webdriver_url or driver_executable");
        }
        if self.launch_args.len() > 64
            || self
                .launch_args
                .iter()
                .any(|a| a.len() > 512 || a.chars().any(char::is_control))
        {
            bail!("browser.launch_args exceeds a safe bound");
        }
        for arg in &self.launch_args {
            let lower = arg.to_ascii_lowercase();
            if lower.starts_with("--user-data-dir")
                || lower.starts_with("--remote-debugging")
                || lower == "--no-sandbox"
            {
                bail!("browser.launch_args contains a forbidden profile/debug/sandbox override");
            }
        }
        if self.authorized_hosts.len() > 256 {
            bail!("browser.authorized_hosts supports at most 256 entries");
        }
        for host in &self.authorized_hosts {
            let h = host.trim().trim_end_matches('.');
            if h.is_empty()
                || h.contains('/')
                || h.contains(':')
                || h.contains('@')
                || h.chars().any(char::is_control)
            {
                bail!("browser.authorized_hosts contains an invalid host entry");
            }
        }
        if self.download.max_file_bytes == 0
            || self.download.max_file_bytes > 256 * 1024 * 1024
            || self.download.allowed_mime_types.len() > 64
        {
            bail!("browser.download limits are outside supported bounds");
        }
        for mime in &self.download.allowed_mime_types {
            if mime.len() > 128 || mime.chars().any(char::is_control) {
                bail!("browser.download has an invalid MIME type");
            }
        }
        Ok(())
    }
}

/// Per-target browser settings. `workflow` is run in order after the initial
/// navigation. Session ids are scoped to the current process and are never
/// interpreted as filesystem paths.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TargetBrowserConfig {
    pub enabled: bool,
    pub session_id: Option<String>,
    pub profile_id: Option<String>,
    pub workflow: Workflow,
    pub allowed_hosts: Vec<String>,
}

impl TargetBrowserConfig {
    pub fn validate(&self) -> Result<()> {
        if let Some(id) = &self.session_id {
            if id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            {
                bail!("browser session_id must contain only letters, digits, dot, underscore, or hyphen");
            }
        }
        if let Some(id) = &self.profile_id {
            if id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            {
                bail!("browser profile_id has an invalid shape");
            }
        }
        if self.allowed_hosts.len() > 128 {
            bail!("target browser.allowed_hosts supports at most 128 hosts");
        }
        for host in &self.allowed_hosts {
            if host.is_empty()
                || host.contains('/')
                || host.contains(':')
                || host.contains('@')
                || host.chars().any(char::is_control)
            {
                bail!("target browser.allowed_hosts contains an invalid host");
            }
        }
        self.workflow.validate()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_http_only_and_backward_compatible() {
        let cfg: BrowserConfig = serde_json::from_str("{}").unwrap();
        assert!(!cfg.enabled);
        assert_eq!(cfg.engine, BrowserEngine::Chrome);
        cfg.validate().unwrap();
    }

    #[test]
    fn webdriver_credentials_are_rejected() {
        let mut cfg = BrowserConfig::default();
        cfg.webdriver_url = Some("https://user:secret@example.test/wd/hub".into());
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn unsafe_launch_args_are_rejected() {
        let mut cfg = BrowserConfig::default();
        cfg.launch_args.push("--no-sandbox".into());
        assert!(cfg.validate().is_err());
    }
}
