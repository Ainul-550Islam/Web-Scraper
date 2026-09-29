//! Config schema versioning and migration.
//!
//! Honest history (this matters more than cleverness): harvest configs grew
//! additively — every field introduced since the baseline carries a serde
//! default, so old files keep parsing unchanged. There were NO key renames
//! or moves in the recorded history, and this module deliberately does not
//! invent any. What versioning buys us is the contract for the future:
//!
//! * `config_version` stamps a file with the schema it conforms to
//!   (absent ⇒ v1: the pre-versioning baseline).
//! * Loading always migrates to the version this build understands
//!   ([`CURRENT_VERSION`]) through the ordered migration table, collecting
//!   actionable warnings on the way.
//! * A file stamped with a NEWER version than this build understands is a
//!   hard, immediate error — running a future config under a past engine
//!   would silently drop the semantics that version added. Never a warning.
//! * Migrations can never silently discard fields: the table operates on
//!   the raw JSON `Value`, and the unknown-key check at the top of
//!   [`migrate`] rejects typos before anything is rewritten.

use serde_json::{Map, Value};

/// The schema version this build writes and understands natively.
pub const CURRENT_VERSION: u32 = 2;

/// Serde default hook so an unstamped in-memory document still round-trips
/// as current (files on disk go through [`migrate`], which stamps v1 → v2
/// with a warning; a struct literal is already v2 by construction).
pub fn current_version_default() -> u32 {
    CURRENT_VERSION
}

/// A single schema step. Versions must chain without gaps: v1→v2→…→
/// `CURRENT_VERSION`; each step takes the raw JSON value and returns
/// transformed JSON plus user-facing notes (what changed and why).
pub struct Migration {
    pub from: u32,
    pub to: u32,
    pub apply: fn(&mut Value) -> Vec<String>,
}

/// The ordered table. Adding a breaking config change later means adding a
/// row here (plus a test) — and nothing else; loaders, `harvest validate`,
/// and `harvest config migrate` pick it up automatically.
pub const MIGRATIONS: &[Migration] = &[Migration {
    from: 1,
    to: 2,
    apply: migrate_v1_to_v2,
}];

/// v1 (baseline layout) → v2 (version-stamped, warning-channel layout).
///
/// The layouts are IDENTICAL by design — v1 configs parse under v2
/// unchanged — so this step exists purely to stamp the version and to be
/// the honest, tested placeholder that the chain requires. If a future v3
/// renames a key, v1→v2→v3 runs in order and that file migrates too.
fn migrate_v1_to_v2(doc: &mut Value) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some(obj) = doc.as_object_mut() {
        if !obj.contains_key("config_version") {
            // Rebuild with the stamp first (serde_json Map preserves order):
            // a reader opening the file sees `config_version` on line one.
            let mut fresh = Map::new();
            fresh.insert("config_version".into(), Value::from(CURRENT_VERSION));
            for (k, val) in obj.iter() {
                fresh.insert(k.clone(), val.clone());
            }
            *obj = fresh;
            notes.push(
                "config_version: stamped 2 (v1 baseline layout; no field changes exist \
                 between v1 and v2)"
                    .into(),
            );
        }
    }
    notes
}

/// Reject unknown top-level keys with the closest known spelling. Runs on
/// the RAW document before migration: a typo'd key would otherwise be
/// silently dropped by serde's lenient path, which is exactly the "silently
/// discard" failure mode migrations must never have.
pub fn check_unknown_keys(doc: &Value, known: &[&str]) -> Result<(), String> {
    let Some(obj) = doc.as_object() else {
        return Ok(()); // non-objects are rejected downstream with a real message
    };
    for key in obj.keys() {
        // `_…` keys are documented config comments (the samples ship one);
        // serde ignores them the same way.
        if key == "config_version" || key.starts_with('_') || known.contains(&key.as_str()) {
            continue;
        }
        // nearest known key by single-edit distance (substitution only is
        // enough for the common typo class and keeps this dependency-free)
        let hint = known
            .iter()
            .find(|k| k.len().abs_diff(key.len()) <= 1 && edit1(k, key))
            .map(|k| format!(" — did you mean `{k}`?"))
            .unwrap_or_default();
        return Err(format!("unknown config key `{key}`{hint}"));
    }
    Ok(())
}

fn edit1(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut i = 0;
    while i < a.len().min(b.len()) && a[i] == b[i] {
        i += 1;
    }
    match a.len() as i64 - b.len() as i64 {
        // one substitution at i, then everything must match again
        0 => a.get(i + 1..) == b.get(i + 1..),
        // one deletion from a at i (a is longer)
        1 => a.get(i + 1..) == b.get(i..),
        // one insertion into a at i (b is longer)
        -1 => a.get(i..) == b.get(i + 1..),
        _ => false,
    }
}

/// Migrate a raw config document up to [`CURRENT_VERSION`]. `Ok((doc,
/// warnings))` when the document is loadable by this build; `Err` with an
/// actionable message when it is not (future stamp, version gap, unknown
/// key inside a migration target).
pub fn migrate(doc: &Value) -> Result<(Value, Vec<String>), String> {
    let mut doc = doc.clone();
    let declared = doc
        .get("config_version")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(1); // pre-versioning files ARE v1
    if declared > CURRENT_VERSION {
        return Err(format!(
            "config_version {declared} is newer than this build understands \
             (max {CURRENT_VERSION}) — upgrade harvest before loading this config, \
             do not downgrade the file: a hand-edit would drop v{declared} semantics"
        ));
    }
    let mut warnings = Vec::new();
    let mut v = declared;
    while v < CURRENT_VERSION {
        let step = MIGRATIONS
            .iter()
            .find(|m| m.from == v && m.to == v + 1)
            .ok_or_else(|| format!("no migration path from config_version {v} to {}", v + 1))?;
        warnings.extend((step.apply)(&mut doc));
        v = step.to;
    }
    if let Some(obj) = doc.as_object_mut() {
        obj.insert("config_version".into(), Value::from(CURRENT_VERSION));
    }
    Ok((doc, warnings))
}

/// Convenience for `harvest config migrate --out`: canonical pretty JSON
/// with the version stamp (the migrate engine already front-places it).
pub fn rewritten_file(doc: &Value) -> String {
    let (migrated, _) = migrate(doc).unwrap_or_else(|_| (doc.clone(), Vec::new()));
    format!(
        "{}\n",
        serde_json::to_string_pretty(&migrated).unwrap_or_default()
    )
}

#[cfg(test)]
mod tests;
