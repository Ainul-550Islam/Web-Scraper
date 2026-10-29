//! Request validation for the managed customer API.
//!
//! Everything a caller can influence is bounded here, at the edge, before
//! any work is scheduled. The checks are deliberately explicit rather than
//! generic: a rejection reason has to be actionable by the caller, so each
//! one names the field it failed on.

use crate::config::ManagedApiCfg;
use crate::scrape_api::models::{ErrorBody, OutputFormat, ScrapeRequest};
use crate::usage::ledger::RenderMode;

/// Request-side error codes.
pub const ERR_INVALID_REQUEST: &str = "invalid_request";
pub const ERR_URL_REJECTED: &str = "url_rejected";
pub const ERR_WEBHOOK_INVALID: &str = "webhook_invalid";

/// Validate a request against the operator's configuration.
pub fn validate_request(cfg: &ManagedApiCfg, req: &ScrapeRequest) -> Result<(), ErrorBody> {
    let url = req.url.as_str();
    if url.trim().is_empty() {
        return Err(ErrorBody::new(ERR_INVALID_REQUEST, "url must not be empty"));
    }
    if url.len() > crate::config::V1_MAX_URL_LEN {
        return Err(ErrorBody::new(
            ERR_INVALID_REQUEST,
            "url exceeds the maximum length",
        ));
    }
    validate_url(url)?;

    match req.render {
        RenderMode::None | RenderMode::Browser => {}
    }

    if let Some(ms) = req.wait.ms {
        if ms > cfg.jobs.max_browser_ms {
            return Err(ErrorBody::new(
                ERR_INVALID_REQUEST,
                "wait.ms exceeds the configured maximum",
            ));
        }
    }
    if let Some(ms) = req.wait.settle_ms {
        if ms > cfg.jobs.max_browser_ms {
            return Err(ErrorBody::new(
                ERR_INVALID_REQUEST,
                "wait.settle_ms exceeds the configured maximum",
            ));
        }
    }
    if let Some(sel) = &req.wait.until_selector {
        if sel.len() > 256 || sel.contains(char::is_whitespace) {
            return Err(ErrorBody::new(
                ERR_INVALID_REQUEST,
                "wait.until_selector must be a single CSS selector",
            ));
        }
    }

    if req.format == OutputFormat::Screenshot && req.render != RenderMode::Browser {
        return Err(ErrorBody::new(
            ERR_INVALID_REQUEST,
            "format \"screenshot\" requires render: \"browser\"",
        ));
    }
    if req.screenshot && req.render != RenderMode::Browser {
        return Err(ErrorBody::new(
            ERR_INVALID_REQUEST,
            "screenshot requires render: \"browser\"",
        ));
    }

    if let Some(region) = &req.proxy_region {
        if region.len() > 64 {
            return Err(ErrorBody::new(
                ERR_INVALID_REQUEST,
                "proxy_region exceeds the maximum length",
            ));
        }
    }
    if let Some(session) = &req.session_key {
        if session.len() > 128
            || !session
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(ErrorBody::new(
                ERR_INVALID_REQUEST,
                "session_key must be up to 128 ASCII alphanumeric characters",
            ));
        }
    }

    if req.extract.fields.len() > crate::config::V1_EXTRACT_MAX_FIELDS {
        return Err(ErrorBody::new(
            ERR_INVALID_REQUEST,
            "too many extract fields in one request",
        ));
    }
    for (name, rule) in &req.extract.fields {
        if name.is_empty() {
            return Err(ErrorBody::new(
                ERR_INVALID_REQUEST,
                "extract field names must not be empty",
            ));
        }
        if rule.css.len() > 1024 {
            return Err(ErrorBody::new(
                ERR_INVALID_REQUEST,
                format!("extract field {name}: css selector is too long"),
            ));
        }
        if let Some(path) = &rule.path {
            if path.len() > 1024 {
                return Err(ErrorBody::new(
                    ERR_INVALID_REQUEST,
                    format!("extract field {name}: jsonpath is too long"),
                ));
            }
        }
    }
    if let Some(tree) = &req.extract.selector {
        if tree.to_string().len() > 64 * 1024 {
            return Err(ErrorBody::new(
                ERR_INVALID_REQUEST,
                "extract.selector is too large",
            ));
        }
    }
    if let Some(max) = req.extract.max_records {
        if max > crate::config::V1_MAX_RECORDS {
            return Err(ErrorBody::new(
                ERR_INVALID_REQUEST,
                "extract.max_records exceeds the configured maximum",
            ));
        }
    }

    if let Some(url) = &req.webhook_url {
        validate_webhook_url(cfg, url)?;
    }
    Ok(())
}

/// A target must be an absolute `http`/`https` URL with a host and no
/// whitespace or control characters. Only the `http` and `https` schemes are
/// ever followed; everything else is refused here, before the URL is handed to
/// the fetcher or the SSRF guard.
pub fn validate_url(url: &str) -> Result<(), ErrorBody> {
    if url.trim().is_empty() {
        return Err(ErrorBody::new(ERR_INVALID_REQUEST, "url must not be empty"));
    }
    if url.len() > crate::config::V1_MAX_URL_LEN {
        return Err(ErrorBody::new(
            ERR_INVALID_REQUEST,
            "url exceeds the maximum length",
        ));
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(ErrorBody::new(
            ERR_INVALID_REQUEST,
            "url must not contain whitespace or control characters",
        ));
    }
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => {
            return Err(ErrorBody::new(
                ERR_INVALID_REQUEST,
                "url is not a valid URL",
            ))
        }
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(ErrorBody::new(
            ERR_INVALID_REQUEST,
            "only http and https URLs are accepted",
        ));
    }
    if parsed.host_str().is_none() {
        return Err(ErrorBody::new(ERR_INVALID_REQUEST, "url must name a host"));
    }
    Ok(())
}

/// A webhook endpoint must be a public `https` URL of bounded length.
pub fn validate_webhook_url(cfg: &ManagedApiCfg, url: &str) -> Result<(), ErrorBody> {
    if url.trim().is_empty() {
        return Err(ErrorBody::new(
            ERR_WEBHOOK_INVALID,
            "webhook_url must not be empty",
        ));
    }
    if url.len() > crate::config::V1_WEBHOOK_URL_MAX_LEN {
        return Err(ErrorBody::new(
            ERR_WEBHOOK_INVALID,
            "webhook_url exceeds the maximum length",
        ));
    }
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => {
            return Err(ErrorBody::new(
                ERR_WEBHOOK_INVALID,
                "webhook_url is not a valid URL",
            ))
        }
    };
    // Webhooks are https in production. The documented demo/test opt-in
    // relaxes that along with the SSRF guard, so a local receiver can be
    // exercised end to end; nothing else changes.
    if parsed.scheme() != "https" && !(cfg.allow_private_targets && parsed.scheme() == "http") {
        return Err(ErrorBody::new(
            ERR_WEBHOOK_INVALID,
            "webhook_url must use https",
        ));
    }
    let host = parsed.host_str().unwrap_or("");
    if host.is_empty() {
        return Err(ErrorBody::new(
            ERR_WEBHOOK_INVALID,
            "webhook_url must have a host",
        ));
    }
    if !cfg.webhook.allowed_hosts.is_empty()
        && !cfg
            .webhook
            .allowed_hosts
            .iter()
            .any(|allowed| host == allowed || host.ends_with(&format!(".{allowed}")))
    {
        return Err(ErrorBody::new(
            ERR_WEBHOOK_INVALID,
            format!("webhook_url host {host} is not in the allowlist"),
        ));
    }
    Ok(())
}

/// The SSRF check is shared with the fetcher's own guard, but the error is
/// translated into the managed API's vocabulary here so a caller sees a
/// stable code rather than an internal one.
pub fn ssrf_error(err: &crate::ssrf::SsrfError) -> ErrorBody {
    ErrorBody::new(ERR_URL_REJECTED, err.message())
}
