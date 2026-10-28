//! Durable, versioned session metadata store. Cookie/storage secrets are not
//! persisted here; live browser state is owned by WebDriver sessions.
use super::error::{BrowserError, BrowserResult};
use super::page::now_ms;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const VERSION: u32 = 1;
const MAX_STORE_BYTES: u64 = 4 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub id: String,
    pub runtime_id: String,
    pub profile_id: Option<String>,
    pub egress_id: Option<String>,
    pub created_at_ms: u64,
    pub expires_at_ms: Option<u64>,
    pub invalidated_at_ms: Option<u64>,
    pub artifact_ids: Vec<String>,
}
#[derive(Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    sessions: Vec<SessionMetadata>,
}

pub struct SessionStore {
    path: PathBuf,
}
impl SessionStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn list(&self) -> BrowserResult<Vec<SessionMetadata>> {
        let bytes = match crate::fsutil::read_bounded(&self.path, MAX_STORE_BYTES) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => {
                return Err(BrowserError::Policy(
                    "session metadata store cannot be safely read".into(),
                ))
            }
        };
        let doc: StoreFile = serde_json::from_slice(&bytes)
            .map_err(|_| BrowserError::InvalidInput("session metadata store"))?;
        if doc.version != VERSION {
            return Err(BrowserError::Unsupported("session store version"));
        }
        Ok(doc.sessions)
    }
    pub fn save(&self, metadata: SessionMetadata) -> BrowserResult<()> {
        validate_id(&metadata.id)?;
        let mut sessions = self.list()?;
        if let Some(existing) = sessions.iter_mut().find(|s| s.id == metadata.id) {
            *existing = metadata;
        } else {
            if sessions.len() >= 10_000 {
                return Err(BrowserError::ContentLimit {
                    kind: "session metadata entries",
                    limit: 10_000,
                });
            }
            sessions.push(metadata);
        }
        self.write(sessions)
    }
    pub fn get(&self, id: &str) -> BrowserResult<Option<SessionMetadata>> {
        Ok(self.list()?.into_iter().find(|s| s.id == id && !expired(s)))
    }
    pub fn invalidate(&self, id: &str) -> BrowserResult<bool> {
        let mut sessions = self.list()?;
        let Some(s) = sessions.iter_mut().find(|s| s.id == id) else {
            return Ok(false);
        };
        s.invalidated_at_ms = Some(now_ms());
        self.write(sessions)?;
        Ok(true)
    }
    pub fn purge_expired(&self) -> BrowserResult<usize> {
        let mut sessions = self.list()?;
        let before = sessions.len();
        sessions.retain(|s| !expired(s) && s.invalidated_at_ms.is_none());
        let removed = before - sessions.len();
        self.write(sessions)?;
        Ok(removed)
    }
    fn write(&self, sessions: Vec<SessionMetadata>) -> BrowserResult<()> {
        let bytes = serde_json::to_vec(&StoreFile {
            version: VERSION,
            sessions,
        })
        .map_err(|_| BrowserError::InvalidInput("session metadata"))?;
        if bytes.len() as u64 > MAX_STORE_BYTES {
            return Err(BrowserError::ContentLimit {
                kind: "session metadata store",
                limit: MAX_STORE_BYTES as usize,
            });
        }
        crate::fsutil::write_private_atomic(&self.path, &bytes).map_err(|_| {
            BrowserError::Policy("session metadata store cannot be safely written".into())
        })
    }
}
fn expired(s: &SessionMetadata) -> bool {
    s.expires_at_ms.is_some_and(|t| t <= now_ms())
}
fn validate_id(id: &str) -> BrowserResult<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        Err(BrowserError::InvalidInput("session metadata id"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_path_like_session_ids() {
        assert!(validate_id("../cookies").is_err());
        assert!(validate_id("session-1").is_ok());
    }
}
