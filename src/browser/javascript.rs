//! Explicitly gated, size-limited JavaScript evaluation.
use super::config::BrowserConfig;
use super::error::{BrowserError, BrowserResult};
use super::session::BrowserSession;
use serde_json::Value;

pub async fn evaluate(
    session: &BrowserSession,
    cfg: &BrowserConfig,
    script: &str,
    args: Vec<Value>,
) -> BrowserResult<Value> {
    if !cfg.allow_javascript {
        return Err(BrowserError::Policy(
            "JavaScript evaluation is disabled by browser.allow_javascript".into(),
        ));
    }
    if script.is_empty() || script.len() > 16_384 || args.len() > 32 {
        return Err(BrowserError::InvalidInput("JavaScript payload"));
    }
    let arg_bytes = serde_json::to_vec(&args)
        .map_err(|_| BrowserError::InvalidInput("JavaScript arguments"))?;
    if arg_bytes.len() > 32 * 1024 {
        return Err(BrowserError::ContentLimit {
            kind: "JavaScript arguments",
            limit: 32 * 1024,
        });
    }
    // Script bodies and argument values are intentionally omitted from logs.
    session
        .execute(script, args, cfg.max_text_bytes.min(256 * 1024))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn javaScript_requires_explicit_opt_in() {
        let cfg = BrowserConfig::default();
        assert!(!cfg.allow_javascript);
        let error = BrowserError::Policy("JavaScript evaluation is disabled".into());
        assert!(error.to_string().contains("policy"));
    }
}
