//! Secret-reference resolution. The browser subsystem never logs resolved
//! values; environment variables are the built-in provider boundary.
use super::error::{BrowserError, BrowserResult};
use std::fmt;

pub trait CredentialResolver: Send + Sync {
    fn resolve(&self, reference: &str) -> BrowserResult<SecretValue>;
}

#[derive(Default)]
pub struct EnvironmentCredentialResolver;

impl CredentialResolver for EnvironmentCredentialResolver {
    fn resolve(&self, reference: &str) -> BrowserResult<SecretValue> {
        let name = reference
            .strip_prefix("env:")
            .ok_or(BrowserError::InvalidInput("secret reference"))?;
        if name.len() > 128
            || !name.starts_with("HARVEST_")
            || !name
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(BrowserError::InvalidInput("secret environment reference"));
        }
        let value = std::env::var(name).map_err(|_| {
            BrowserError::Policy(format!("secret reference {name} is not configured"))
        })?;
        if value.len() > 16_384 {
            return Err(BrowserError::ContentLimit {
                kind: "secret",
                limit: 16_384,
            });
        }
        Ok(SecretValue(value))
    }
}

/// Secret container with redacted formatting and best-effort memory clearing.
pub struct SecretValue(String);

impl SecretValue {
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn into_inner(mut self) -> String {
        std::mem::take(&mut self.0)
    }
}
impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue([REDACTED])")
    }
}
impl fmt::Display for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl Drop for SecretValue {
    fn drop(&mut self) {
        unsafe {
            self.0.as_bytes_mut().fill(0);
        }
    }
}

pub fn resolve_secret(resolver: &dyn CredentialResolver, reference: &str) -> BrowserResult<String> {
    resolver.resolve(reference).map(SecretValue::into_inner)
}

pub fn redact(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    for marker in [
        "password=",
        "token=",
        "cookie=",
        "authorization=",
        "secret=",
    ] {
        if let Some(pos) = lower.find(marker) {
            let start = pos + marker.len();
            let end = input[start..]
                .find(['&', ' ', '\n', '\r', ';'])
                .map(|n| start + n)
                .unwrap_or(input.len());
            return format!("{}[REDACTED]{}", &input[..start], &input[end..]);
        }
    }
    input.chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secret_debug_and_display_are_redacted() {
        let secret = SecretValue("not-for-logs".into());
        assert_eq!(format!("{secret:?}"), "SecretValue([REDACTED])");
        assert_eq!(format!("{secret}"), "[REDACTED]");
    }
    #[test]
    fn resolver_rejects_non_harvest_environment_name() {
        let r = EnvironmentCredentialResolver;
        assert!(r.resolve("env:PATH").is_err());
        assert!(r.resolve("plaintext").is_err());
    }
}
