//! Secret-safe typed browser errors.
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserError {
    Config(String),
    Launch(String),
    MissingExecutable,
    Timeout { operation: &'static str },
    Connection,
    Crashed,
    ContextCreation,
    PageCreation,
    Navigation { status: Option<u16> },
    Javascript(String),
    ContentLimit { kind: &'static str, limit: usize },
    Shutdown,
    Unsupported(&'static str),
    Policy(String),
    Protocol { status: u16 },
    NotFound(&'static str),
    InvalidInput(&'static str),
}

pub type BrowserResult<T> = Result<T, BrowserError>;

impl fmt::Display for BrowserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(s) => write!(f, "browser configuration error: {}", safe_message(s)),
            Self::Launch(s) => write!(f, "browser launch failed: {}", safe_message(s)),
            Self::MissingExecutable => f.write_str("browser driver executable is missing"),
            Self::Timeout { operation } => write!(f, "browser {operation} timed out"),
            Self::Connection => f.write_str("browser driver connection failed"),
            Self::Crashed => f.write_str("browser process exited unexpectedly"),
            Self::ContextCreation => f.write_str("browser context creation failed"),
            Self::PageCreation => f.write_str("browser page creation failed"),
            Self::Navigation { status } => match status {
                Some(s) => write!(f, "browser navigation failed (driver status {s})"),
                None => f.write_str("browser navigation failed"),
            },
            Self::Javascript(s) => write!(f, "browser JavaScript failed: {}", safe_message(s)),
            Self::ContentLimit { kind, limit } => write!(
                f,
                "browser {kind} exceeded configured limit ({limit} bytes)"
            ),
            Self::Shutdown => f.write_str("browser shutdown failed"),
            Self::Unsupported(cap) => write!(f, "browser capability is unsupported: {cap}"),
            Self::Policy(s) => write!(
                f,
                "browser policy denied the operation: {}",
                safe_message(s)
            ),
            Self::Protocol { status } => write!(f, "browser driver returned HTTP status {status}"),
            Self::NotFound(what) => write!(f, "browser {what} was not found"),
            Self::InvalidInput(what) => write!(f, "invalid browser {what}"),
        }
    }
}

impl std::error::Error for BrowserError {}

/// Redact common credential-bearing values before they reach error surfaces.
pub fn safe_message(input: &str) -> String {
    let mut out = input.chars().take(256).collect::<String>();
    for key in [
        "password=",
        "token=",
        "authorization=",
        "cookie=",
        "secret=",
    ] {
        let lower = out.to_ascii_lowercase();
        if let Some(start) = lower.find(key) {
            let value_start = start + key.len();
            let end = out[value_start..]
                .find(['&', ' ', '\n', '\r', ';'])
                .map(|n| value_start + n)
                .unwrap_or(out.len());
            out.replace_range(value_start..end, "[REDACTED]");
        }
    }
    if let Some(scheme) = out.find("://") {
        let auth_start = scheme + 3;
        if let Some(at_rel) = out[auth_start..].find('@') {
            if out[auth_start..auth_start + at_rel].contains(':') {
                out.replace_range(auth_start..auth_start + at_rel, "[REDACTED]");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redacts_url_auth_and_common_secret_parameters() {
        let msg = safe_message("https://alice:secret@host.test/x?token=abc password=xyz");
        assert!(!msg.contains("secret"));
        assert!(!msg.contains("abc"));
        assert!(!msg.contains("xyz"));
    }
}
