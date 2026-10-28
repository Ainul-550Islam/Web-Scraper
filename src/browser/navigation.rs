//! Real WebDriver navigation and explicit completion policies.
use super::error::{BrowserError, BrowserResult};
use super::session::BrowserSession;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WaitUntil {
    #[default]
    Load,
    DomContentLoaded,
    NetworkIdle,
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NavigationResult {
    pub requested_url: String,
    pub final_url: String,
    pub title: String,
    pub elapsed_ms: u64,
    pub wait_until: WaitUntil,
}

pub async fn navigate(
    session: &BrowserSession,
    url: &str,
    wait_until: WaitUntil,
    timeout_ms: u64,
) -> BrowserResult<NavigationResult> {
    if !(100..=300_000).contains(&timeout_ms) {
        return Err(BrowserError::InvalidInput("navigation timeout"));
    }
    let start = tokio::time::Instant::now();
    session.navigate(url, timeout_ms).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    match wait_until {
        WaitUntil::None => {}
        WaitUntil::Load => wait_ready_state(session, "complete", deadline).await?,
        WaitUntil::DomContentLoaded => wait_ready_state(session, "interactive", deadline).await?,
        WaitUntil::NetworkIdle => wait_network_idle(session, deadline).await?,
    }
    Ok(NavigationResult {
        requested_url: url.to_owned(),
        final_url: session.current_url().await?,
        title: session.title().await?,
        elapsed_ms: start.elapsed().as_millis().min(u64::MAX as u128) as u64,
        wait_until,
    })
}

async fn wait_ready_state(
    session: &BrowserSession,
    expected: &str,
    deadline: tokio::time::Instant,
) -> BrowserResult<()> {
    loop {
        let value = session
            .execute("return document.readyState;", Vec::new(), 128)
            .await?;
        let ready = value
            .as_str()
            .ok_or(BrowserError::Protocol { status: 502 })?;
        if ready == "complete" || (expected == "interactive" && ready == "interactive") {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(BrowserError::Timeout {
                operation: "navigation wait",
            });
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_network_idle(
    session: &BrowserSession,
    deadline: tokio::time::Instant,
) -> BrowserResult<()> {
    let mut quiet_since = None;
    loop {
        let value = session.execute("return document.readyState === 'complete' && (performance.getEntriesByType('resource').filter(e => e.responseEnd > performance.now()-500).length === 0);", Vec::new(), 128).await?;
        if value == Value::Bool(true) {
            let since = *quiet_since.get_or_insert_with(tokio::time::Instant::now);
            if since.elapsed() >= Duration::from_millis(500) {
                return Ok(());
            }
        } else {
            quiet_since = None;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(BrowserError::Timeout {
                operation: "network idle wait",
            });
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wait_policy_serde_is_stable() {
        assert_eq!(
            serde_json::to_string(&WaitUntil::DomContentLoaded).unwrap(),
            "\"dom_content_loaded\""
        );
    }
}
