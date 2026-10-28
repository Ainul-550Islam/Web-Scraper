//! Login workflow abstraction. Credentials are resolved only at execution;
//! MFA and challenge pages are returned to the operator/provider boundary.
use super::actions;
use super::credentials::{resolve_secret, CredentialResolver};
use super::error::{BrowserError, BrowserResult};
use super::session::BrowserSession;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginFlow {
    pub username_selector: String,
    pub password_selector: String,
    pub submit_selector: String,
    pub username_secret_ref: String,
    pub password_secret_ref: String,
    pub success_selector: Option<String>,
    pub failure_selector: Option<String>,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginResult {
    Authenticated,
    Rejected,
    ManualChallengeRequired,
}

impl LoginFlow {
    pub fn validate(&self) -> BrowserResult<()> {
        for (name, v) in [
            ("username selector", &self.username_selector),
            ("password selector", &self.password_selector),
            ("submit selector", &self.submit_selector),
            ("username secret", &self.username_secret_ref),
            ("password secret", &self.password_secret_ref),
        ] {
            if v.is_empty() || v.len() > 4096 || v.chars().any(char::is_control) {
                return Err(BrowserError::InvalidInput(name));
            }
        }
        if !(1000..=300_000).contains(&self.timeout_ms) {
            return Err(BrowserError::InvalidInput("login timeout"));
        }
        Ok(())
    }
    pub async fn execute(
        &self,
        session: &BrowserSession,
        resolver: &dyn CredentialResolver,
    ) -> BrowserResult<LoginResult> {
        self.validate()?;
        let username = resolve_secret(resolver, &self.username_secret_ref)?;
        let password = resolve_secret(resolver, &self.password_secret_ref)?;
        actions::input(session, &self.username_selector, &username, true).await?;
        actions::input(session, &self.password_selector, &password, true).await?;
        actions::click(session, &self.submit_selector).await?;
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(self.timeout_ms);
        loop {
            if let Some(sel) = &self.success_selector {
                let ok = session
                    .execute(
                        "return document.querySelector(arguments[0])!==null;",
                        vec![json!(sel)],
                        128,
                    )
                    .await?;
                if ok == serde_json::Value::Bool(true) {
                    return Ok(LoginResult::Authenticated);
                }
            }
            if let Some(sel) = &self.failure_selector {
                let fail = session
                    .execute(
                        "return document.querySelector(arguments[0])!==null;",
                        vec![json!(sel)],
                        128,
                    )
                    .await?;
                if fail == serde_json::Value::Bool(true) {
                    return Ok(LoginResult::Rejected);
                }
            }
            let text = session
                .execute(
                    "return (document.body?.innerText||'').toLowerCase();",
                    Vec::new(),
                    32 * 1024,
                )
                .await?
                .as_str()
                .unwrap_or("")
                .to_ascii_lowercase();
            if [
                "captcha",
                "two-factor",
                "two factor",
                "verify you are human",
                "security challenge",
            ]
            .iter()
            .any(|m| text.contains(m))
            {
                return Ok(LoginResult::ManualChallengeRequired);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(BrowserError::Timeout {
                    operation: "login condition",
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unbounded_login_timeout() {
        let f = LoginFlow {
            username_selector: "#u".into(),
            password_selector: "#p".into(),
            submit_selector: "button".into(),
            username_secret_ref: "env:HARVEST_USER".into(),
            password_secret_ref: "env:HARVEST_PASS".into(),
            success_selector: None,
            failure_selector: None,
            timeout_ms: 0,
        };
        assert!(f.validate().is_err());
    }
}
