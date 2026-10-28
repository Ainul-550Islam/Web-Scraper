//! Proxy credential references and safe URL handling. Providers should use
//! references instead of persisting usernames/passwords in configs.
use super::provider::redacted_url;
use crate::browser::credentials::{CredentialResolver, SecretValue};
use crate::browser::error::BrowserError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyCredentialRefs {
    pub username_secret_ref: Option<String>,
    pub password_secret_ref: Option<String>,
}
impl ProxyCredentialRefs {
    pub fn validate(&self) -> Result<(), BrowserError> {
        for r in self
            .username_secret_ref
            .iter()
            .chain(self.password_secret_ref.iter())
        {
            if !r.starts_with("env:HARVEST_") || r.len() > 160 || r.chars().any(char::is_control) {
                return Err(BrowserError::InvalidInput("proxy credential reference"));
            }
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        resolver: &dyn CredentialResolver,
    ) -> Result<ResolvedProxyCredentials, BrowserError> {
        self.validate()?;
        Ok(ResolvedProxyCredentials {
            username: match &self.username_secret_ref {
                Some(r) => Some(resolver.resolve(r)?),
                None => None,
            },
            password: match &self.password_secret_ref {
                Some(r) => Some(resolver.resolve(r)?),
                None => None,
            },
        })
    }
}
pub struct ResolvedProxyCredentials {
    pub username: Option<SecretValue>,
    pub password: Option<SecretValue>,
}
impl std::fmt::Debug for ResolvedProxyCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedProxyCredentials")
            .field("username", &self.username.as_ref().map(|_| "[REDACTED]"))
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

pub fn safe_connection_string(url: &str) -> String {
    redacted_url(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn safe_url_never_shows_password() {
        assert!(!safe_connection_string("http://u:p@host.test:3").contains(":p@"));
    }
}
