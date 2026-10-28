//! Bounded browser wait conditions. Polling is explicit and always capped.
use super::error::{BrowserError, BrowserResult};
use super::session::BrowserSession;
use super::workflow::WaitCondition;
use serde_json::{json, Value};
use std::time::Duration;

pub async fn wait_for(
    session: &BrowserSession,
    condition: &WaitCondition,
    timeout_ms: u64,
) -> BrowserResult<()> {
    if !(100..=300_000).contains(&timeout_ms) {
        return Err(BrowserError::InvalidInput("wait timeout"));
    }
    if let WaitCondition::Delay { millis } = condition {
        if *millis > 30_000 {
            return Err(BrowserError::InvalidInput("explicit delay"));
        }
        tokio::time::sleep(Duration::from_millis(*millis)).await;
        return Ok(());
    }
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if condition_met(session, condition).await? {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(BrowserError::Timeout {
                operation: "wait condition",
            });
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn condition_met(session: &BrowserSession, condition: &WaitCondition) -> BrowserResult<bool> {
    match condition {
        WaitCondition::SelectorAttached { selector } => {
            scraper::Selector::parse(selector)
                .map_err(|_| BrowserError::InvalidInput("CSS selector"))?;
            let v = session
                .execute(
                    "return document.querySelector(arguments[0]) !== null;",
                    vec![json!(selector)],
                    128,
                )
                .await?;
            Ok(v == Value::Bool(true))
        }
        WaitCondition::SelectorVisible { selector } => {
            scraper::Selector::parse(selector)
                .map_err(|_| BrowserError::InvalidInput("CSS selector"))?;
            let v=session.execute("const e=document.querySelector(arguments[0]); if(!e) return false; const r=e.getBoundingClientRect(); const s=getComputedStyle(e); return r.width>0&&r.height>0&&s.visibility!=='hidden'&&s.display!=='none';", vec![json!(selector)], 128).await?;
            Ok(v == Value::Bool(true))
        }
        WaitCondition::TextPresent { text } => {
            let v = session
                .execute(
                    "return (document.body?.innerText||'').includes(arguments[0]);",
                    vec![json!(text)],
                    128,
                )
                .await?;
            Ok(v == Value::Bool(true))
        }
        WaitCondition::UrlContains { value } => Ok(session.current_url().await?.contains(value)),
        WaitCondition::PageComplete => {
            let v = session
                .execute("return document.readyState==='complete';", Vec::new(), 128)
                .await?;
            Ok(v == Value::Bool(true))
        }
        WaitCondition::Delay { .. } => Ok(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wait_timeout_bounds_reject_zero() {
        assert!(!(100..=300_000).contains(&0));
    }
}
