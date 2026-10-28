//! In-memory cookie jar abstraction. Cookie values are excluded from Debug
//! and ordinary serialization; durable plaintext cookie persistence is not
//! provided by this module.
use super::error::{BrowserError, BrowserResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;

#[derive(Clone, Serialize, Deserialize)]
pub struct BrowserCookie {
    pub name: String,
    #[serde(skip_serializing)]
    pub value: String,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<String>,
    pub expires_unix_secs: Option<u64>,
}
impl fmt::Debug for BrowserCookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserCookie")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("secure", &self.secure)
            .field("http_only", &self.http_only)
            .finish()
    }
}

#[derive(Default)]
pub struct CookieJar {
    cookies: Vec<BrowserCookie>,
}
impl CookieJar {
    pub fn set(&mut self, cookie: BrowserCookie, origin_host: &str) -> BrowserResult<()> {
        validate_for_session(&cookie, origin_host)?;
        if let Some(old) = self
            .cookies
            .iter_mut()
            .find(|c| c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
        {
            *old = cookie;
        } else if self.cookies.len() < 256 {
            self.cookies.push(cookie);
        } else {
            return Err(BrowserError::ContentLimit {
                kind: "cookie count",
                limit: 256,
            });
        }
        Ok(())
    }
    pub fn get(&self, name: &str) -> Option<&BrowserCookie> {
        self.cookies.iter().find(|c| c.name == name)
    }
    pub fn list(&self) -> Vec<CookieMetadata> {
        self.cookies.iter().map(CookieMetadata::from).collect()
    }
    pub fn remove(&mut self, name: &str, domain: Option<&str>, path: Option<&str>) -> bool {
        let n = self.cookies.len();
        self.cookies.retain(|c| {
            !(c.name == name
                && domain.is_none_or(|d| c.domain.as_deref() == Some(d))
                && path.is_none_or(|p| c.path.as_deref() == Some(p)))
        });
        n != self.cookies.len()
    }
    pub fn clear(&mut self) {
        self.cookies.clear();
    }
    /// Explicit export for an authorized caller. The caller is responsible for
    /// encrypting the returned JSON before storing it.
    pub fn export_authorized(&self, authorized: bool) -> BrowserResult<Vec<u8>> {
        if !authorized {
            return Err(BrowserError::Policy(
                "cookie export requires explicit authorization".into(),
            ));
        }
        if self.cookies.len() > 256 {
            return Err(BrowserError::ContentLimit {
                kind: "cookie count",
                limit: 256,
            });
        }
        serde_json::to_vec(&self.cookies.iter().map(|c|json!({"name":c.name,"value":c.value,"domain":c.domain,"path":c.path,"secure":c.secure,"http_only":c.http_only,"same_site":c.same_site,"expires_unix_secs":c.expires_unix_secs})).collect::<Vec<_>>()).map_err(|_|BrowserError::InvalidInput("cookie export"))
    }
    pub fn import_authorized(
        &mut self,
        bytes: &[u8],
        origin_host: &str,
        authorized: bool,
    ) -> BrowserResult<usize> {
        if !authorized {
            return Err(BrowserError::Policy(
                "cookie import requires explicit authorization".into(),
            ));
        }
        if bytes.len() > 256 * 1024 {
            return Err(BrowserError::ContentLimit {
                kind: "cookie import",
                limit: 256 * 1024,
            });
        }
        let list: Vec<BrowserCookie> = serde_json::from_slice(bytes)
            .map_err(|_| BrowserError::InvalidInput("cookie import"))?;
        if list.len() > 256 {
            return Err(BrowserError::ContentLimit {
                kind: "cookie count",
                limit: 256,
            });
        }
        for cookie in list {
            self.set(cookie, origin_host)?;
        }
        Ok(self.cookies.len())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CookieMetadata {
    pub name: String,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub secure: bool,
    pub http_only: bool,
    pub expires_unix_secs: Option<u64>,
    pub value_redacted: bool,
}
impl From<&BrowserCookie> for CookieMetadata {
    fn from(c: &BrowserCookie) -> Self {
        Self {
            name: c.name.clone(),
            domain: c.domain.clone(),
            path: c.path.clone(),
            secure: c.secure,
            http_only: c.http_only,
            expires_unix_secs: c.expires_unix_secs,
            value_redacted: true,
        }
    }
}

pub fn validate_for_session(cookie: &BrowserCookie, origin_host: &str) -> BrowserResult<()> {
    if cookie.name.is_empty()
        || cookie.name.len() > 256
        || !cookie.name.bytes().all(is_cookie_token)
    {
        return Err(BrowserError::InvalidInput("cookie name"));
    }
    if cookie.value.len() > 4096
        || cookie
            .value
            .chars()
            .any(|c| c == '\r' || c == '\n' || c == '\0')
    {
        return Err(BrowserError::InvalidInput("cookie value"));
    }
    if let Some(domain) = &cookie.domain {
        let d = domain.trim_start_matches('.').to_ascii_lowercase();
        if d.is_empty()
            || !(origin_host.eq_ignore_ascii_case(&d)
                || origin_host.to_ascii_lowercase().ends_with(&format!(".{d}")))
        {
            return Err(BrowserError::Policy(
                "cookie domain is outside the current host".into(),
            ));
        }
    }
    if let Some(path) = &cookie.path {
        if !path.starts_with('/') || path.len() > 2048 || path.chars().any(char::is_control) {
            return Err(BrowserError::InvalidInput("cookie path"));
        }
    }
    if cookie
        .same_site
        .as_deref()
        .is_some_and(|s| !matches!(s, "Strict" | "Lax" | "None"))
    {
        return Err(BrowserError::InvalidInput("cookie SameSite"));
    }
    Ok(())
}

pub(crate) fn to_webdriver_value(cookie: &BrowserCookie) -> Value {
    let mut value = json!({"name":cookie.name,"value":cookie.value,"secure":cookie.secure,"httpOnly":cookie.http_only});
    if let Some(v) = &cookie.domain {
        value["domain"] = json!(v);
    }
    if let Some(v) = &cookie.path {
        value["path"] = json!(v);
    }
    if let Some(v) = &cookie.same_site {
        value["sameSite"] = json!(v);
    }
    if let Some(v) = cookie.expires_unix_secs {
        value["expiry"] = json!(v);
    }
    value
}
pub(crate) fn from_webdriver_value(v: &Value) -> BrowserResult<BrowserCookie> {
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .ok_or(BrowserError::Protocol { status: 502 })?
        .to_owned();
    let value = v
        .get("value")
        .and_then(Value::as_str)
        .ok_or(BrowserError::Protocol { status: 502 })?
        .to_owned();
    Ok(BrowserCookie {
        name,
        value,
        domain: v
            .get("domain")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        path: v.get("path").and_then(Value::as_str).map(ToOwned::to_owned),
        secure: v.get("secure").and_then(Value::as_bool).unwrap_or(false),
        http_only: v.get("httpOnly").and_then(Value::as_bool).unwrap_or(false),
        same_site: v
            .get("sameSite")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        expires_unix_secs: v.get("expiry").and_then(Value::as_u64),
    })
}
fn is_cookie_token(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cookie(domain: &str) -> BrowserCookie {
        BrowserCookie {
            name: "sid".into(),
            value: "private".into(),
            domain: Some(domain.into()),
            path: Some("/".into()),
            secure: true,
            http_only: true,
            same_site: Some("Lax".into()),
            expires_unix_secs: None,
        }
    }
    #[test]
    fn rejects_cross_site_cookie_and_redacts_debug() {
        let c = cookie("evil.test");
        let mut jar = CookieJar::default();
        assert!(jar.set(c, "example.test").is_err());
        let c = cookie("example.test");
        assert!(!format!("{c:?}").contains("private"));
    }
    #[test]
    fn export_requires_explicit_authorization_and_includes_authorized_value() {
        let mut jar = CookieJar::default();
        jar.set(cookie("example.test"), "example.test").unwrap();
        assert!(jar.export_authorized(false).is_err());
        let bytes = jar.export_authorized(true).unwrap();
        assert!(String::from_utf8(bytes).unwrap().contains("private"));
    }
}
