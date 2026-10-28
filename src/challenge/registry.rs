//! Capability- and domain-scoped provider registry.
use super::provider::{ChallengeProvider, ProviderCapabilities};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
struct Entry {
    provider: Arc<dyn ChallengeProvider>,
    allowed_domains: Vec<String>,
}
pub struct ChallengeRegistry {
    entries: RwLock<BTreeMap<String, Entry>>,
}
impl Default for ChallengeRegistry {
    fn default() -> Self {
        Self {
            entries: RwLock::new(BTreeMap::new()),
        }
    }
}
impl ChallengeRegistry {
    pub fn register(
        &self,
        provider: Arc<dyn ChallengeProvider>,
        allowed_domains: Vec<String>,
    ) -> Result<(), String> {
        let id = provider.id().to_owned();
        if id.is_empty() || id.len() > 128 || allowed_domains.len() > 256 {
            return Err("invalid challenge provider registration".into());
        }
        if provider.capabilities().challenge_kinds.is_empty() {
            return Err("challenge provider declares no capabilities".into());
        }
        for d in &allowed_domains {
            if d.is_empty() || d.len() > 253 || d.contains('/') || d.contains('@') {
                return Err("invalid provider domain scope".into());
            }
        }
        let mut m = self.entries.write().unwrap_or_else(|p| p.into_inner());
        if m.contains_key(&id) {
            return Err("duplicate challenge provider id".into());
        }
        m.insert(
            id,
            Entry {
                provider,
                allowed_domains,
            },
        );
        Ok(())
    }
    pub fn select(
        &self,
        domain: &str,
        kind: &str,
        allowlist: &[String],
    ) -> Option<Arc<dyn ChallengeProvider>> {
        let m = self.entries.read().unwrap_or_else(|p| p.into_inner());
        m.iter()
            .filter(|(id, e)| {
                allowlist.iter().any(|a| a == *id)
                    && e.allowed_domains.iter().any(|d| {
                        domain == d
                            || domain
                                .strip_suffix(d)
                                .is_some_and(|prefix| prefix.ends_with('.'))
                    })
                    && e.provider.health()
                    && e.provider
                        .capabilities()
                        .challenge_kinds
                        .iter()
                        .any(|k| k == kind)
            })
            .map(|(_, e)| e.provider.clone())
            .next()
    }
    pub fn inventory(&self) -> BTreeMap<String, ProviderCapabilities> {
        self.entries
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(id, e)| (id.clone(), e.provider.capabilities()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::challenge::provider::*;
    struct P;
    impl ChallengeProvider for P {
        fn id(&self) -> &str {
            "p"
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                challenge_kinds: vec!["captcha".into()],
                supports_authorized_scope: true,
                ..Default::default()
            }
        }
        fn submit(&self, _: &ProviderRequest) -> Result<ProviderResult, ProviderError> {
            unreachable!()
        }
        fn health(&self) -> bool {
            true
        }
    }
    #[test]
    fn domain_and_allowlist_both_required() {
        let r = ChallengeRegistry::default();
        r.register(Arc::new(P), vec!["example.test".into()])
            .unwrap();
        assert!(r
            .select("sub.example.test", "captcha", &["p".into()])
            .is_some());
        assert!(r.select("sub.example.test", "captcha", &[]).is_none());
        assert!(r.select("else.test", "captcha", &["p".into()]).is_none());
    }
}
