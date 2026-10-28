//! Private, bounded browser artifact storage. Large payloads are represented
//! by metadata references; paths are never returned to callers.
use super::error::{BrowserError, BrowserResult};
use super::page::PageArtifactRef;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_ARTIFACTS: usize = 2048;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserArtifactMetadata {
    pub id: String,
    pub kind: String,
    pub content_type: String,
    pub sha256: String,
    pub bytes: u64,
    pub created_at_ms: u64,
    pub session_id: Option<String>,
}
#[derive(Clone)]
struct Stored {
    metadata: BrowserArtifactMetadata,
    path: PathBuf,
}
pub struct BrowserArtifactStore {
    root: PathBuf,
    entries: Mutex<BTreeMap<String, Stored>>,
}
impl BrowserArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            entries: Mutex::new(BTreeMap::new()),
        }
    }
    pub fn store(
        &self,
        kind: &str,
        content_type: &str,
        bytes: &[u8],
        max_bytes: usize,
        session_id: Option<&str>,
    ) -> BrowserResult<PageArtifactRef> {
        if bytes.len() > max_bytes {
            return Err(BrowserError::ContentLimit {
                kind: "artifact",
                limit: max_bytes,
            });
        }
        let (extension, expected) =
            kind_info(kind).ok_or(BrowserError::InvalidInput("artifact kind"))?;
        if !valid_content_type(content_type, expected) {
            return Err(BrowserError::InvalidInput("artifact content type"));
        }
        if bytes.is_empty() {
            return Err(BrowserError::InvalidInput("empty artifact"));
        }
        let digest = Sha256::digest(bytes);
        let hex = digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let id = format!("br-{}-{}", &hex[..24], super::page::now_ms());
        let dir = self.root.join(".browser-artifacts");
        let path = dir.join(format!("{id}.{extension}"));
        crate::fsutil::write_private_atomic(&path, bytes).map_err(|_| {
            BrowserError::Policy("browser artifact could not be safely stored".into())
        })?;
        let metadata = BrowserArtifactMetadata {
            id: id.clone(),
            kind: kind.into(),
            content_type: content_type.into(),
            sha256: hex.clone(),
            bytes: bytes.len() as u64,
            created_at_ms: super::page::now_ms(),
            session_id: session_id.map(ToOwned::to_owned),
        };
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        if entries.len() >= MAX_ARTIFACTS {
            if let Some(oldest) = entries
                .values()
                .min_by_key(|v| v.metadata.created_at_ms)
                .map(|v| v.metadata.id.clone())
            {
                entries.remove(&oldest);
            }
        }
        entries.insert(id.clone(), Stored { metadata, path });
        Ok(PageArtifactRef {
            id,
            kind: kind.into(),
            sha256: hex,
            bytes: bytes.len() as u64,
        })
    }
    pub fn metadata(&self, id: &str) -> Option<BrowserArtifactMetadata> {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .map(|s| s.metadata.clone())
    }
    pub fn read(&self, id: &str, max_bytes: u64) -> BrowserResult<Vec<u8>> {
        let entry = self
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
            .ok_or(BrowserError::NotFound("browser artifact"))?;
        if entry.metadata.bytes > max_bytes {
            return Err(BrowserError::ContentLimit {
                kind: "artifact download",
                limit: max_bytes as usize,
            });
        }
        crate::fsutil::read_bounded(&entry.path, max_bytes)
            .map_err(|_| BrowserError::Policy("browser artifact could not be safely read".into()))
    }
    pub fn list(&self) -> Vec<BrowserArtifactMetadata> {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .map(|s| s.metadata.clone())
            .collect()
    }
    pub fn root_is_configured(&self) -> bool {
        !self.root.as_os_str().is_empty()
    }
}
fn kind_info(kind: &str) -> Option<(&'static str, &'static str)> {
    match kind {
        "screenshot" => Some(("png", "image/png")),
        "dom" => Some(("html", "text/html")),
        "html" => Some(("html", "text/html")),
        "trace" => Some(("json", "application/json")),
        "download" => Some(("bin", "application/octet-stream")),
        "challenge" => Some(("png", "image/png")),
        _ => None,
    }
}
fn valid_content_type(actual: &str, expected: &str) -> bool {
    actual
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case(expected)
        || (expected == "application/octet-stream" && actual.starts_with("application/"))
}

#[cfg(feature = "browser")]
pub async fn capture_screenshot(
    session: &super::session::BrowserSession,
    store: &BrowserArtifactStore,
    max_bytes: usize,
) -> BrowserResult<PageArtifactRef> {
    use base64::Engine;
    use reqwest::Method;
    use serde_json::Value;
    let encoded = session.command(Method::GET, "screenshot", None).await?;
    let text = encoded
        .as_str()
        .ok_or(BrowserError::Protocol { status: 502 })?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|_| BrowserError::Protocol { status: 502 })?;
    if bytes.len() > max_bytes {
        return Err(BrowserError::ContentLimit {
            kind: "screenshot",
            limit: max_bytes,
        });
    }
    if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return Err(BrowserError::InvalidInput("screenshot image bytes"));
    }
    let _ = Value::Null;
    store.store(
        "screenshot",
        "image/png",
        &bytes,
        max_bytes,
        Some(session.id()),
    )
}

#[cfg(feature = "browser")]
pub fn store_dom(
    store: &BrowserArtifactStore,
    session_id: &str,
    html: &str,
    max_bytes: usize,
) -> BrowserResult<PageArtifactRef> {
    store.store(
        "dom",
        "text/html",
        html.as_bytes(),
        max_bytes,
        Some(session_id),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_artifact_kind() {
        let s = BrowserArtifactStore::new(std::env::temp_dir());
        assert!(s.store("arbitrary", "text/plain", b"x", 10, None).is_err());
    }
    #[test]
    fn content_types_are_pinned() {
        assert!(!valid_content_type("text/html", "image/png"));
        assert!(valid_content_type("text/html; charset=utf-8", "text/html"));
    }
}
