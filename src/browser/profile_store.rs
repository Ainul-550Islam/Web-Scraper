//! Atomic, private profile persistence with schema versioning and TTL cleanup.
use super::error::{BrowserError, BrowserResult};
use super::page::now_ms;
use super::profile::BrowserProfile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

const VERSION: u32 = 1;
const MAX_BYTES: u64 = 2 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
struct ProfileFile {
    version: u32,
    profiles: BTreeMap<String, BrowserProfile>,
}
pub struct ProfileStore {
    path: PathBuf,
}
impl ProfileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn list(&self) -> BrowserResult<Vec<BrowserProfile>> {
        let data = match crate::fsutil::read_bounded(&self.path, MAX_BYTES) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(_) => {
                return Err(BrowserError::Policy(
                    "profile store could not be safely read".into(),
                ))
            }
        };
        let f: ProfileFile = serde_json::from_slice(&data)
            .map_err(|_| BrowserError::InvalidInput("profile store"))?;
        if f.version != VERSION {
            return Err(BrowserError::Unsupported("profile store version"));
        }
        Ok(f.profiles.into_values().collect())
    }
    pub fn get(&self, id: &str) -> BrowserResult<Option<BrowserProfile>> {
        Ok(self
            .list()?
            .into_iter()
            .find(|p| p.id == id && !p.expired(now_ms())))
    }
    pub fn save(&self, profile: BrowserProfile) -> BrowserResult<()> {
        profile.validate()?;
        let mut all: BTreeMap<_, _> = self
            .list()?
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect();
        if !all.contains_key(&profile.id) && all.len() >= 256 {
            return Err(BrowserError::ContentLimit {
                kind: "browser profiles",
                limit: 256,
            });
        }
        all.insert(profile.id.clone(), profile);
        self.write(all)
    }
    pub fn remove(&self, id: &str) -> BrowserResult<bool> {
        let mut all: BTreeMap<_, _> = self
            .list()?
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect();
        let removed = all.remove(id).is_some();
        if removed {
            self.write(all)?;
        }
        Ok(removed)
    }
    pub fn purge_expired(&self) -> BrowserResult<usize> {
        let mut all: BTreeMap<_, _> = self
            .list()?
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect();
        let n = all.len();
        all.retain(|_, p| !p.expired(now_ms()));
        let removed = n - all.len();
        if removed > 0 {
            self.write(all)?;
        }
        Ok(removed)
    }
    fn write(&self, profiles: BTreeMap<String, BrowserProfile>) -> BrowserResult<()> {
        let bytes = serde_json::to_vec(&ProfileFile {
            version: VERSION,
            profiles,
        })
        .map_err(|_| BrowserError::InvalidInput("profile serialization"))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(BrowserError::ContentLimit {
                kind: "profile store",
                limit: MAX_BYTES as usize,
            });
        }
        crate::fsutil::write_private_atomic(&self.path, &bytes)
            .map_err(|_| BrowserError::Policy("profile store could not be safely written".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_store_is_empty() {
        let path =
            std::env::temp_dir().join(format!("harvest-profile-test-{}", std::process::id()));
        let s = ProfileStore::new(path);
        assert!(s.list().unwrap().is_empty());
    }
}
