//! Bounded immutable observation of a browser page.
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageArtifactRef {
    pub id: String,
    pub kind: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageObservation {
    pub page_id: String,
    pub session_id: String,
    pub runtime_id: String,
    pub requested_url: String,
    pub final_url: Option<String>,
    pub status: Option<u16>,
    pub title: Option<String>,
    pub content_type: Option<String>,
    pub html: Option<String>,
    pub visible_text: Option<String>,
    pub started_at_unix_ms: u64,
    pub finished_at_unix_ms: u64,
    pub error: Option<String>,
    pub artifacts: Vec<PageArtifactRef>,
}

impl PageObservation {
    pub fn new(
        session_id: impl Into<String>,
        runtime_id: impl Into<String>,
        requested_url: impl Into<String>,
    ) -> Self {
        let now = now_ms();
        Self {
            page_id: format!("page-{now}"),
            session_id: session_id.into(),
            runtime_id: runtime_id.into(),
            requested_url: requested_url.into(),
            final_url: None,
            status: None,
            title: None,
            content_type: None,
            html: None,
            visible_text: None,
            started_at_unix_ms: now,
            finished_at_unix_ms: now,
            error: None,
            artifacts: Vec::new(),
        }
    }

    pub fn finish(&mut self) {
        self.finished_at_unix_ms = now_ms();
    }

    pub fn set_html_bounded(&mut self, html: String, limit: usize) -> Result<(), usize> {
        if html.len() > limit {
            return Err(html.len());
        }
        self.html = Some(html);
        Ok(())
    }

    pub fn set_text_bounded(&mut self, text: String, limit: usize) -> Result<(), usize> {
        if text.len() > limit {
            return Err(text.len());
        }
        self.visible_text = Some(text);
        Ok(())
    }

    pub fn add_artifact(&mut self, artifact: PageArtifactRef, max: usize) -> bool {
        if self.artifacts.len() >= max {
            return false;
        }
        self.artifacts.push(artifact);
        true
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observation_rejects_oversized_snapshots() {
        let mut p = PageObservation::new("s", "r", "https://example.test");
        assert!(p.set_html_bounded("abcd".into(), 3).is_err());
        assert!(p.set_html_bounded("abc".into(), 3).is_ok());
        assert_eq!(p.html.as_deref(), Some("abc"));
    }
}
