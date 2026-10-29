//! Tenancy and API keys for the managed scrape surface.
//!
//! The engine was built as an operator-facing crawler. The managed API is the
//! layer that turns it into a product: tenants, keys, scopes, per-key limits,
//! metering and quota. Everything here is deliberately small and explicit —
//! a key is a hash plus a scope list, a tenant is a name plus a status, and
//! the store is one atomically-rewritten JSON document.
//!
//! Module map:
//! * [`tenant`] — the owning account and its status.
//! * [`key`] — key generation, hashing and constant-time verification.
//! * [`scope`] — the closed capability vocabulary and its checks.
//! * [`limits`] — per-key pacing/concurrency/quota bounds and validation.
//! * [`store`] — the crash-safe, corruption-fail-closed persistence layer.
//! * [`auth`] — header parsing and the fail-closed authentication boundary.
//!
//! See `docs/modules/tenancy.md`.

pub mod auth;
pub mod key;
pub mod limits;
pub mod scope;
pub mod store;
pub mod tenant;

pub use auth::{authenticate, authenticate_headers, Principal};
pub use key::{generate_key, hash_key, verify_key, ApiKey, GeneratedKey, KeyId, KeyStatus};
pub use limits::KeyLimits;
pub use scope::{Scope, ScopeSet};
pub use store::{StoreError, TenancyStore};
pub use tenant::{Tenant, TenantId, TenantStatus};

#[cfg(test)]
mod tests;
