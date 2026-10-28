//! Bounded localStorage/sessionStorage capture and restoration. Snapshots keep
//! values in memory only and intentionally lack Debug/Serialize implementations.
use super::error::{BrowserError, BrowserResult};
use super::session::BrowserSession;
use serde_json::{json, Value};

pub struct StorageSnapshot {
    values: Vec<(String, String)>,
}
impl StorageSnapshot {
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    pub fn redacted_metadata(&self) -> Vec<StorageEntryMetadata> {
        self.values
            .iter()
            .map(|(key, value)| StorageEntryMetadata {
                key: key.clone(),
                value_bytes: value.len(),
                redacted: is_sensitive_key(key),
            })
            .collect()
    }
    pub fn serialized_bytes(&self) -> usize {
        self.values.iter().map(|(k, v)| k.len() + v.len()).sum()
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct StorageEntryMetadata {
    pub key: String,
    pub value_bytes: usize,
    pub redacted: bool,
}

pub async fn capture(
    session: &BrowserSession,
    storage: &str,
    max_entries: usize,
    max_bytes: usize,
) -> BrowserResult<StorageSnapshot> {
    let storage = storage_name(storage)?;
    let value=session.execute("const s=window[arguments[0]]; const a=[]; for(let i=0;i<s.length;i++){const k=s.key(i); if(k!==null)a.push([k,s.getItem(k)||'']);} return a;",vec![json!(storage)],max_bytes.saturating_add(64*1024)).await?;
    let entries = value
        .as_array()
        .ok_or(BrowserError::Protocol { status: 502 })?;
    if entries.len() > max_entries {
        return Err(BrowserError::ContentLimit {
            kind: "storage entries",
            limit: max_entries,
        });
    }
    let mut values = Vec::with_capacity(entries.len());
    let mut total = 0usize;
    for entry in entries {
        let pair = entry
            .as_array()
            .filter(|p| p.len() == 2)
            .ok_or(BrowserError::Protocol { status: 502 })?;
        let key = pair[0]
            .as_str()
            .ok_or(BrowserError::Protocol { status: 502 })?
            .to_owned();
        let val = pair[1]
            .as_str()
            .ok_or(BrowserError::Protocol { status: 502 })?
            .to_owned();
        total = total.saturating_add(key.len()).saturating_add(val.len());
        if total > max_bytes {
            return Err(BrowserError::ContentLimit {
                kind: "storage snapshot",
                limit: max_bytes,
            });
        }
        values.push((key, val));
    }
    Ok(StorageSnapshot { values })
}

pub async fn restore(
    session: &BrowserSession,
    storage: &str,
    snapshot: &StorageSnapshot,
    max_bytes: usize,
) -> BrowserResult<()> {
    if snapshot.serialized_bytes() > max_bytes {
        return Err(BrowserError::ContentLimit {
            kind: "storage restore",
            limit: max_bytes,
        });
    }
    let name = storage_name(storage)?;
    let pairs: Vec<Value> = snapshot.values.iter().map(|(k, v)| json!([k, v])).collect();
    session.execute("const s=window[arguments[0]]; const a=arguments[1]; s.clear(); for(const p of a){s.setItem(p[0],p[1]);} return s.length;",vec![json!(name),json!(pairs)],4096).await?;
    Ok(())
}

pub fn redacted_json(snapshot: &StorageSnapshot) -> Value {
    json!({"count":snapshot.values.len(),"entries":snapshot.redacted_metadata()})
}
fn storage_name(name: &str) -> BrowserResult<&'static str> {
    match name {
        "localStorage" | "local" => Ok("localStorage"),
        "sessionStorage" | "session" => Ok("sessionStorage"),
        _ => Err(BrowserError::InvalidInput("storage kind")),
    }
}
fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "pass",
        "token",
        "auth",
        "cookie",
        "secret",
        "session",
        "credential",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_redacts_sensitive_key() {
        let s = StorageSnapshot {
            values: vec![
                ("access_token".into(), "s3cr3t".into()),
                ("theme".into(), "dark".into()),
            ],
        };
        let json = redacted_json(&s).to_string();
        assert!(!json.contains("s3cr3t"));
        assert!(json.contains("access_token"));
        assert!(s.redacted_metadata()[0].redacted);
    }
}
