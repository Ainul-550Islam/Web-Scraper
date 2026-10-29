//! Unit pricing.
//!
//! One request is billed as the sum of a few bounded components: a fixed
//! base, the renderer, an extraction pass, the transforms, any screenshot,
//! and the bytes handed back. Every component is saturated on overflow so a
//! pathological response can never wrap the counter.
//!
//! The table is the operator's published price; a key can only ever lower
//! its own byte rates, never raise the per-request charge.

use crate::config::{ManagedApiCfg, PricingTable};
use crate::scrape_api::models::{OutputFormat, ScrapeRequest};
use crate::usage::ledger::RenderMode;

/// A resolved price plan: the operator's published table.
#[derive(Debug, Clone, PartialEq)]
pub struct Pricing {
    pub table: PricingTable,
}

impl Pricing {
    pub fn from_cfg(cfg: &ManagedApiCfg) -> Self {
        Pricing {
            table: cfg.pricing.clone(),
        }
    }

    /// Total units for one finished request.
    pub fn units_for(&self, req: &ScrapeRequest, body_bytes: u64, artifact_bytes: u64) -> u32 {
        let t = &self.table;
        let mut units = t.base_units;

        if req.render == RenderMode::Browser {
            units = units.saturating_add(t.render_units);
        }
        match (
            req.extract.fields.is_empty(),
            req.extract.selector.is_some(),
        ) {
            (false, true) => {
                units = units.saturating_add(t.selector_units);
            }
            (false, false) => {
                units = units.saturating_add(t.extract_units);
            }
            (true, true) => {
                units = units.saturating_add(t.selector_units);
            }
            _ => {}
        }
        let transforms: u32 = req
            .extract
            .fields
            .values()
            .map(|f| f.transforms.len() as u32)
            .sum();
        if transforms > 0 {
            units = units.saturating_add(t.transform_units.saturating_mul(transforms.min(4)));
        }
        if req.screenshot || req.format == OutputFormat::Screenshot {
            units = units.saturating_add(t.screenshot_units);
        }
        if let Some(region) = &req.proxy_region {
            if let Some(extra) = t.proxy_region_units.get(region) {
                units = units.saturating_add(*extra);
            }
        }

        units = units.saturating_add(bytes_units(body_bytes, t.per_1k_bytes));
        units = units.saturating_add(bytes_units(artifact_bytes, t.artifact_units_per_k));
        units
    }

    /// Human-readable cost for a number of units.
    pub fn cost(&self, units: u32) -> String {
        format!(
            "{:.6} {}",
            units as f64 * self.table.rate,
            self.table.currency
        )
    }
}

/// Units for `bytes / 1024` whole kibibytes, saturated.
fn bytes_units(bytes: u64, per_k: u32) -> u32 {
    if bytes == 0 || per_k == 0 {
        return 0;
    }
    let kb = (bytes / 1024) as u32;
    kb.saturating_mul(per_k)
}
