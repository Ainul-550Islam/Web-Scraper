//! Typed page actions. Input values are never included in errors or tracing.
use super::error::{BrowserError, BrowserResult};
use super::session::BrowserSession;
use reqwest::Method;
use serde_json::{json, Value};
use std::time::Duration;

#[cfg(test)]
const ACTION_TIMEOUT_MS: u64 = 15_000;

pub async fn click(session: &BrowserSession, selector: &str) -> BrowserResult<()> {
    let id = find_element(session, selector).await?;
    session
        .command(
            Method::POST,
            &format!("element/{id}/click"),
            Some(json!({})),
        )
        .await?;
    Ok(())
}

pub async fn input(
    session: &BrowserSession,
    selector: &str,
    value: &str,
    clear_first: bool,
) -> BrowserResult<()> {
    if value.len() > 16_384 || value.chars().any(|c| c == '\0') {
        return Err(BrowserError::InvalidInput("input value"));
    }
    let id = find_element(session, selector).await?;
    if clear_first {
        session
            .command(
                Method::POST,
                &format!("element/{id}/clear"),
                Some(json!({})),
            )
            .await?;
    }
    session.command(Method::POST, &format!("element/{id}/value"), Some(json!({"text":value,"value":value.chars().map(|c| c.to_string()).collect::<Vec<_>>()}))).await?;
    Ok(())
}

pub async fn keypress(
    session: &BrowserSession,
    selector: Option<&str>,
    key: &str,
) -> BrowserResult<()> {
    let key_value = key_code(key).ok_or(BrowserError::InvalidInput("keypress key"))?;
    let id = match selector {
        Some(s) => Some(find_element(session, s).await?),
        None => None,
    };
    let path = id
        .map(|id| format!("element/{id}/value"))
        .unwrap_or_else(|| "actions".into());
    let payload = if path == "actions" {
        json!({"actions":[{"type":"key","id":"keyboard","actions":[{"type":"keyDown","value":key_value},{"type":"keyUp","value":key_value}]}]})
    } else {
        json!({"text":key_value,"value":[key_value]})
    };
    session.command(Method::POST, &path, Some(payload)).await?;
    Ok(())
}

pub async fn scroll(session: &BrowserSession, direction: &str, pixels: u32) -> BrowserResult<()> {
    if !(1..=10_000).contains(&pixels) {
        return Err(BrowserError::InvalidInput("scroll distance"));
    }
    let dy = match direction {
        "down" => pixels as i64,
        "up" => -(pixels as i64),
        _ => return Err(BrowserError::InvalidInput("scroll direction")),
    };
    session.execute("window.scrollBy({left:0, top:arguments[0], behavior:'instant'}); return {x:window.scrollX,y:window.scrollY};", vec![json!(dy)], 1024).await?;
    Ok(())
}

pub async fn hover(session: &BrowserSession, selector: &str) -> BrowserResult<()> {
    let id = find_element(session, selector).await?;
    session.command(Method::POST, "actions", Some(json!({"actions":[{"type":"pointer","id":"mouse","parameters":{"pointerType":"mouse"},"actions":[{"type":"pointerMove","origin":{"element-6066-11e4-a52e-4f735466cecf":id},"x":0,"y":0}]}]}))).await?;
    Ok(())
}

pub async fn focus(session: &BrowserSession, selector: &str) -> BrowserResult<()> {
    let value = session.execute("const e=document.querySelector(arguments[0]); if(!e) return false; e.focus(); return document.activeElement===e;", vec![json!(selector)], 1024).await?;
    if value == Value::Bool(true) {
        Ok(())
    } else {
        Err(BrowserError::NotFound("focusable element"))
    }
}

pub async fn wait_for(
    session: &BrowserSession,
    selector: &str,
    timeout_ms: u64,
) -> BrowserResult<()> {
    if !(100..=300_000).contains(&timeout_ms) {
        return Err(BrowserError::InvalidInput("action timeout"));
    }
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        match find_element(session, selector).await {
            Ok(_) => return Ok(()),
            Err(BrowserError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(BrowserError::Timeout {
                operation: "selector wait",
            });
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) async fn find_element(
    session: &BrowserSession,
    selector: &str,
) -> BrowserResult<String> {
    if selector.is_empty() || selector.len() > 4096 || selector.chars().any(char::is_control) {
        return Err(BrowserError::InvalidInput("CSS selector"));
    }
    let parsed = scraper::Selector::parse(selector)
        .map_err(|_| BrowserError::InvalidInput("CSS selector"))?;
    drop(parsed);
    let value = session
        .command(
            Method::POST,
            "element",
            Some(json!({"using":"css selector","value":selector})),
        )
        .await?;
    value
        .get("element-6066-11e4-a52e-4f735466cecf")
        .and_then(Value::as_str)
        .or_else(|| value.get("ELEMENT").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .ok_or(BrowserError::Protocol { status: 502 })
}

fn key_code(key: &str) -> Option<String> {
    let code = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => "\u{e007}",
        "tab" => "\u{e004}",
        "escape" | "esc" => "\u{e00c}",
        "backspace" => "\u{e003}",
        "delete" => "\u{e017}",
        "arrowup" => "\u{e013}",
        "arrowdown" => "\u{e015}",
        "arrowleft" => "\u{e012}",
        "arrowright" => "\u{e014}",
        "space" => "\u{e00d}",
        "home" => "\u{e011}",
        "end" => "\u{e010}",
        "pageup" => "\u{e00e}",
        "pagedown" => "\u{e00f}",
        _ if key.chars().count() == 1 => return Some(key.to_owned()),
        _ => return None,
    };
    Some(code.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_common_w3c_keys_and_rejects_unknown_names() {
        assert_eq!(key_code("enter").as_deref(), Some("\u{e007}"));
        assert_eq!(key_code("a").as_deref(), Some("a"));
        assert!(key_code("launch-shell").is_none());
    }
    #[test]
    fn action_timeout_is_bounded() {
        assert!((100..=300_000).contains(&ACTION_TIMEOUT_MS));
    }
}
