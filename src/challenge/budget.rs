//! Atomic challenge-provider request and cost limits per run, target, and provider.
use super::provider::ProviderResult;
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, Default)]
pub struct BudgetLimit {
    pub max_requests: u64,
    pub max_cost_microusd: u64,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Usage {
    pub requests: u64,
    pub cost_microusd: u64,
}
#[derive(Default)]
pub struct ChallengeBudget {
    limit: BudgetLimit,
    usage: Mutex<BTreeMap<(String, String), Usage>>,
}
impl ChallengeBudget {
    pub fn new(limit: BudgetLimit) -> Result<Self, &'static str> {
        if limit.max_requests > 1_000_000 || limit.max_cost_microusd > 10_000_000_000 {
            return Err("challenge budget limit exceeds supported ceiling");
        }
        Ok(Self {
            limit,
            usage: Mutex::new(BTreeMap::new()),
        })
    }
    /// Reserve one provider call before submission. No refund is applied on
    /// provider error: budgets count attempts to prevent unbounded spend.
    pub fn reserve(&self, target: &str, provider: &str) -> Result<Usage, &'static str> {
        let mut m = self.usage.lock().unwrap_or_else(|p| p.into_inner());
        let k = (target.into(), provider.into());
        let u = m.entry(k).or_default();
        if self.limit.max_requests > 0 && u.requests >= self.limit.max_requests {
            return Err("challenge request budget exhausted");
        }
        u.requests = u.requests.saturating_add(1);
        Ok(*u)
    }
    pub fn record_cost(
        &self,
        target: &str,
        provider: &str,
        cost_microusd: u64,
    ) -> Result<Usage, &'static str> {
        let mut m = self.usage.lock().unwrap_or_else(|p| p.into_inner());
        let u = m.entry((target.into(), provider.into())).or_default();
        if self.limit.max_cost_microusd > 0
            && u.cost_microusd.saturating_add(cost_microusd) > self.limit.max_cost_microusd
        {
            return Err("challenge cost budget exhausted");
        }
        u.cost_microusd = u.cost_microusd.saturating_add(cost_microusd);
        Ok(*u)
    }
    pub fn apply_result(
        &self,
        target: &str,
        provider: &str,
        result: &ProviderResult,
    ) -> Result<Usage, &'static str> {
        self.record_cost(target, provider, result.cost_microusd)
    }
    pub fn usage(&self) -> BTreeMap<(String, String), Usage> {
        self.usage.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caps_requests_and_cost() {
        let b = ChallengeBudget::new(BudgetLimit {
            max_requests: 1,
            max_cost_microusd: 10,
        })
        .unwrap();
        b.reserve("t", "p").unwrap();
        assert!(b.reserve("t", "p").is_err());
        b.record_cost("t", "p", 10).unwrap();
        assert!(b.record_cost("t", "p", 1).is_err());
    }
}
