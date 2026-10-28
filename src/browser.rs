//! Optional browser automation subsystem.
//!
//! Browser transport is deliberately separate from the default HTTP fetcher.
//! The implementation uses the W3C WebDriver protocol and is only executable
//! in builds compiled with the `browser` Cargo feature. No browser is launched
//! by a library singleton: callers own a [`runtime::BrowserRuntime`] and shut
//! it down explicitly. Browser actions, egress selection and challenge policy
//! remain auditable and never turn a site refusal into an evasion attempt.

pub mod actions;
pub mod api;
pub mod api_models;
pub mod artifacts;
pub mod auth;
pub mod challenge;
pub mod challenge_policy;
pub mod config;
pub mod context;
pub mod cookies;
pub mod credentials;
pub mod device;
pub mod error;
pub mod fingerprint;
pub mod interaction;
pub mod javascript;
pub mod jobs;
pub mod navigation;
pub mod page;
pub mod profile;
pub mod profile_store;
pub mod repository;
pub mod retry;
pub mod session;
pub mod session_store;
pub mod state;
pub mod storage;
pub mod wait;
pub mod workflow;

#[cfg(feature = "browser")]
pub mod runtime;

pub use config::{BrowserConfig, TargetBrowserConfig};
pub use error::{BrowserError, BrowserResult};
pub use page::{PageArtifactRef, PageObservation};
#[cfg(feature = "browser")]
pub use runtime::BrowserRuntime;
