//! Observable website-state predicates reusable by login and crawl flows.
use super::error::{BrowserError, BrowserResult};
use super::session::BrowserSession;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StatePredicates {
    pub signed_in_selector: Option<String>,
    pub signed_out_selector: Option<String>,
    pub consent_selector: Option<String>,
    pub content_ready_selector: Option<String>,
    pub session_expired_selector: Option<String>,
}
impl Default for StatePredicates {
    fn default() -> Self {
        Self {
            signed_in_selector: None,
            signed_out_selector: None,
            consent_selector: None,
            content_ready_selector: None,
            session_expired_selector: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebsiteState {
    SignedIn,
    SignedOut,
    ConsentRequired,
    ChallengeDetected,
    ContentReady,
    SessionExpired,
    Unknown,
}

pub async fn detect(
    session: &BrowserSession,
    predicates: &StatePredicates,
) -> BrowserResult<WebsiteState> {
    for (selector, state) in [
        (
            &predicates.session_expired_selector,
            WebsiteState::SessionExpired,
        ),
        (&predicates.consent_selector, WebsiteState::ConsentRequired),
        (&predicates.signed_in_selector, WebsiteState::SignedIn),
        (&predicates.signed_out_selector, WebsiteState::SignedOut),
        (
            &predicates.content_ready_selector,
            WebsiteState::ContentReady,
        ),
    ] {
        if let Some(css) = selector {
            scraper::Selector::parse(css)
                .map_err(|_| BrowserError::InvalidInput("state CSS selector"))?;
            let v = session
                .execute(
                    "return document.querySelector(arguments[0])!==null;",
                    vec![json!(css)],
                    128,
                )
                .await?;
            if v == serde_json::Value::Bool(true) {
                return Ok(state);
            }
        }
    }
    let text = session
        .execute(
            "return ((document.title||'')+' '+(document.body?.innerText||'')).toLowerCase();",
            Vec::new(),
            64 * 1024,
        )
        .await?
        .as_str()
        .unwrap_or("")
        .to_owned();
    let lower = text.to_ascii_lowercase();
    if [
        "captcha",
        "verify you are human",
        "checking your browser",
        "access denied",
        "unusual traffic",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return Ok(WebsiteState::ChallengeDetected);
    }
    Ok(WebsiteState::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn predicates_default_empty() {
        let p = StatePredicates::default();
        assert!(p.signed_in_selector.is_none());
    }
}
