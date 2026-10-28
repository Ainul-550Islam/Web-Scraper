//! Deterministic egress selection policy; rotation is reliability/geography
//! driven and is never triggered to evade a block or challenge.
use super::provider::{ProxyEndpoint, ProxyProvider, ProxyRequest};
use crate::browser::error::{BrowserError, BrowserResult};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum EgressPolicy {
    #[default]
    Direct,
    Fixed {
        provider: String,
        endpoint_id: String,
    },
    Pool {
        provider: String,
        region: Option<String>,
        session_affinity: bool,
    },
    PerTarget {
        provider: String,
        region: Option<String>,
    },
    PerSession {
        provider: String,
        region: Option<String>,
    },
}

pub fn select(
    policy: &EgressPolicy,
    request: &ProxyRequest,
    providers: &[&dyn ProxyProvider],
) -> BrowserResult<Option<ProxyEndpoint>> {
    match policy {
        EgressPolicy::Direct => Ok(None),
        EgressPolicy::Fixed {
            provider,
            endpoint_id,
        } => {
            let p = providers
                .iter()
                .find(|p| p.id() == provider)
                .ok_or(BrowserError::NotFound("proxy provider"))?;
            let mut request = request.clone();
            request.preferred_region = None;
            let e = p
                .acquire(&request)
                .map_err(|e| BrowserError::Policy(e.safe_message))?;
            if e.id != *endpoint_id {
                return Err(BrowserError::Policy(
                    "provider did not return the configured fixed endpoint".into(),
                ));
            }
            Ok(Some(e))
        }
        EgressPolicy::Pool {
            provider,
            region,
            session_affinity,
        } => select_from_provider(provider, region, *session_affinity, request, providers),
        EgressPolicy::PerTarget { provider, region }
        | EgressPolicy::PerSession { provider, region } => {
            select_from_provider(provider, region, false, request, providers)
        }
    }
}

fn select_from_provider(
    provider: &str,
    region: &Option<String>,
    require_session_affinity: bool,
    request: &ProxyRequest,
    providers: &[&dyn ProxyProvider],
) -> BrowserResult<Option<ProxyEndpoint>> {
    let p = providers
        .iter()
        .find(|p| p.id() == provider)
        .ok_or(BrowserError::NotFound("proxy provider"))?;
    if require_session_affinity && !p.capabilities().session_affinity {
        return Err(BrowserError::Unsupported("provider session affinity"));
    }
    let mut request = request.clone();
    request.preferred_region = region.clone();
    let endpoint = p
        .acquire(&request)
        .map_err(|e| BrowserError::Policy(e.safe_message))?;
    if let Some(wanted) = region {
        if endpoint.region.as_deref() != Some(wanted.as_str()) {
            return Err(BrowserError::Policy(
                "provider returned an endpoint outside the requested region".into(),
            ));
        }
    }
    Ok(Some(endpoint))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_policy_is_direct() {
        assert_eq!(EgressPolicy::default(), EgressPolicy::Direct);
    }
}
