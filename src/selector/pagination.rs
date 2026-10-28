//! Bounded pagination plans, page-loop detection and action generation.
use super::super::browser::error::{BrowserError, BrowserResult};
use super::super::browser::workflow::WorkflowStep;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case", deny_unknown_fields)]
pub enum PaginationStrategy {
    Auto {
        max_pages: usize,
    },
    NextLink {
        selector: String,
        max_pages: usize,
    },
    Click {
        selector: String,
        max_pages: usize,
    },
    LoadMore {
        selector: String,
        max_clicks: usize,
        max_pages: usize,
    },
    InfiniteScroll {
        pixels: u32,
        max_steps: usize,
        max_pages: usize,
    },
    Scripted {
        steps: Vec<WorkflowStep>,
        max_pages: usize,
    },
}
impl PaginationStrategy {
    pub fn max_pages(&self) -> usize {
        match self {
            Self::Auto { max_pages }
            | Self::NextLink { max_pages, .. }
            | Self::Click { max_pages, .. }
            | Self::LoadMore { max_pages, .. }
            | Self::InfiniteScroll { max_pages, .. }
            | Self::Scripted { max_pages, .. } => *max_pages,
        }
    }
    pub fn validate(&self) -> BrowserResult<()> {
        if !(1..=10_000).contains(&self.max_pages()) {
            return Err(BrowserError::InvalidInput("pagination max_pages"));
        }
        match self {
            Self::NextLink { selector, .. }
            | Self::Click { selector, .. }
            | Self::LoadMore { selector, .. } => {
                scraper::Selector::parse(selector)
                    .map_err(|_| BrowserError::InvalidInput("pagination CSS selector"))?;
            }
            Self::InfiniteScroll {
                pixels, max_steps, ..
            } => {
                if !(1..=10_000).contains(pixels) || !(1..=10_000).contains(max_steps) {
                    return Err(BrowserError::InvalidInput("infinite-scroll bounds"));
                }
            }
            Self::Scripted { steps, .. } => {
                if steps.len() > 64 {
                    return Err(BrowserError::InvalidInput("scripted pagination steps"));
                }
            }
            _ => {}
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PaginationAction {
    Navigate { url: String },
    Click { selector: String },
    Scroll { pixels: u32 },
    RunScript { steps: Vec<WorkflowStep> },
    Stop { reason: String },
}

pub struct PaginationController {
    strategy: PaginationStrategy,
    visited: HashSet<String>,
    pages: usize,
    clicks: usize,
}
impl PaginationController {
    pub fn new(strategy: PaginationStrategy) -> BrowserResult<Self> {
        strategy.validate()?;
        Ok(Self {
            strategy,
            visited: HashSet::new(),
            pages: 0,
            clicks: 0,
        })
    }
    pub fn record_page(&mut self, url: &str) -> BrowserResult<bool> {
        let normalized = normalize_url(url)?;
        self.pages = self.pages.saturating_add(1);
        if self.pages > self.strategy.max_pages() {
            return Ok(false);
        }
        Ok(self.visited.insert(normalized))
    }
    pub fn next_action(
        &mut self,
        current_url: &str,
        links: &[String],
        has_selector: bool,
    ) -> BrowserResult<PaginationAction> {
        if self.pages >= self.strategy.max_pages() {
            return Ok(PaginationAction::Stop {
                reason: "maximum pages reached".into(),
            });
        }
        match &self.strategy {
            PaginationStrategy::Auto { .. } | PaginationStrategy::NextLink { .. } => {
                for href in links {
                    let url = resolve_same_host(current_url, href)?;
                    if !self.visited.contains(&normalize_url(&url)?) {
                        return Ok(PaginationAction::Navigate { url });
                    }
                }
                Ok(PaginationAction::Stop {
                    reason: "no new same-host next link".into(),
                })
            }
            PaginationStrategy::Click { selector, .. } => {
                if has_selector {
                    Ok(PaginationAction::Click {
                        selector: selector.clone(),
                    })
                } else {
                    Ok(PaginationAction::Stop {
                        reason: "pagination control not found".into(),
                    })
                }
            }
            PaginationStrategy::LoadMore {
                selector,
                max_clicks,
                ..
            } => {
                if has_selector && self.clicks < *max_clicks {
                    self.clicks += 1;
                    Ok(PaginationAction::Click {
                        selector: selector.clone(),
                    })
                } else {
                    Ok(PaginationAction::Stop {
                        reason: "load-more limit reached or control absent".into(),
                    })
                }
            }
            PaginationStrategy::InfiniteScroll {
                pixels, max_steps, ..
            } => {
                if self.clicks < *max_steps {
                    self.clicks += 1;
                    Ok(PaginationAction::Scroll { pixels: *pixels })
                } else {
                    Ok(PaginationAction::Stop {
                        reason: "infinite-scroll step limit reached".into(),
                    })
                }
            }
            PaginationStrategy::Scripted { steps, .. } => Ok(PaginationAction::RunScript {
                steps: steps.clone(),
            }),
        }
    }
}
fn normalize_url(url: &str) -> BrowserResult<String> {
    let mut u = url::Url::parse(url).map_err(|_| BrowserError::InvalidInput("pagination URL"))?;
    u.set_fragment(None);
    Ok(u.to_string())
}
fn resolve_same_host(base: &str, href: &str) -> BrowserResult<String> {
    let base =
        url::Url::parse(base).map_err(|_| BrowserError::InvalidInput("pagination base URL"))?;
    let next = base
        .join(href)
        .map_err(|_| BrowserError::InvalidInput("pagination link"))?;
    if !matches!(next.scheme(), "http" | "https") || next.host_str() != base.host_str() {
        return Err(BrowserError::Policy(
            "pagination link is outside the current host".into(),
        ));
    }
    if !next.username().is_empty() || next.password().is_some() {
        return Err(BrowserError::Policy(
            "pagination link contains credentials".into(),
        ));
    }
    Ok(next.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_cross_host_pagination() {
        assert!(resolve_same_host("https://a.test/list", "https://b.test/next").is_err());
    }
    #[test]
    fn loop_detection_stops_duplicate_pages() {
        let mut c = PaginationController::new(PaginationStrategy::Auto { max_pages: 3 }).unwrap();
        assert!(c.record_page("https://a.test/1#x").unwrap());
        assert!(!c.record_page("https://a.test/1#y").unwrap());
    }
}
