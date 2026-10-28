//! Authorized challenge-provider boundary. Challenges default to stop/manual
//! review; no solver is selected without explicit policy and scope approval.
pub mod budget;
pub mod captcha;
pub mod provider;
pub mod registry;
#[cfg(test)]
mod tests;
pub mod unblock;
pub use provider::{ChallengeProvider, ProviderCapabilities, ProviderRequest, ProviderResult};
