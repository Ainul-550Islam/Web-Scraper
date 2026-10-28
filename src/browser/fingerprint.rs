//! Deterministic browser identity/profile consistency. No randomization or
//! spoofing is provided; assignments are stable and audit-attributable.
use super::config::{BrowserConfig, BrowserEngine};
use super::error::{BrowserError, BrowserResult};
use super::profile::BrowserProfile;

#[derive(Clone, Debug, serde::Serialize)]
pub struct ProfileAssignment {
    pub profile_id: String,
    pub target_id: String,
    pub stable_bucket: usize,
}

pub fn select_stable<'a>(
    profiles: &'a [BrowserProfile],
    target_id: &str,
    seed: &str,
) -> BrowserResult<(&'a BrowserProfile, ProfileAssignment)> {
    if profiles.is_empty() {
        return Err(BrowserError::NotFound("browser profile"));
    }
    if target_id.is_empty() || target_id.len() > 128 || seed.len() > 256 {
        return Err(BrowserError::InvalidInput("profile assignment key"));
    }
    let mut valid: Vec<_> = profiles
        .iter()
        .filter(|p| p.validate().is_ok() && !p.expired(super::page::now_ms()))
        .collect();
    if valid.is_empty() {
        return Err(BrowserError::NotFound("active browser profile"));
    }
    valid.sort_by(|a, b| a.id.cmp(&b.id));
    let h = stable_hash(target_id, seed);
    let bucket = (h as usize) % valid.len();
    let p = valid[bucket];
    Ok((
        p,
        ProfileAssignment {
            profile_id: p.id.clone(),
            target_id: target_id.into(),
            stable_bucket: bucket,
        },
    ))
}

pub fn check_consistency(config: &BrowserConfig, profile: &BrowserProfile) -> BrowserResult<()> {
    profile.validate()?;
    if config.engine != profile.engine {
        return Err(BrowserError::Config(
            "profile engine differs from runtime engine".into(),
        ));
    }
    if config.viewport.width != profile.viewport.width
        || config.viewport.height != profile.viewport.height
    {
        return Err(BrowserError::Config(
            "profile viewport differs from runtime viewport".into(),
        ));
    }
    if config.locale != profile.locale {
        return Err(BrowserError::Config(
            "profile locale differs from runtime locale".into(),
        ));
    }
    if let Some(ua) = &profile.user_agent {
        if config
            .user_agent_override
            .as_ref()
            .is_some_and(|configured| configured != ua)
        {
            return Err(BrowserError::Config(
                "profile user-agent differs from runtime override".into(),
            ));
        }
    }
    if profile.engine == BrowserEngine::Firefox
        && config
            .browser_executable
            .as_ref()
            .is_some_and(|p| p.to_string_lossy().to_ascii_lowercase().contains("chrome"))
    {
        return Err(BrowserError::Config(
            "Firefox profile points to a Chrome executable".into(),
        ));
    }
    Ok(())
}

fn stable_hash(a: &str, b: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for byte in a.bytes().chain([0xff]).chain(b.bytes()) {
        h ^= byte as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::config::{BrowserEngine, Viewport};
    fn p(id: &str) -> BrowserProfile {
        BrowserProfile {
            id: id.into(),
            version: 1,
            engine: BrowserEngine::Chrome,
            viewport: Viewport::default(),
            locale: "en-US".into(),
            timezone: None,
            user_agent: None,
            created_at_ms: 0,
            expires_at_ms: None,
        }
    }
    #[test]
    fn assignment_is_deterministic() {
        let ps = vec![p("a"), p("b")];
        let (_, a) = select_stable(&ps, "target", "seed").unwrap();
        let (_, b) = select_stable(&ps, "target", "seed").unwrap();
        assert_eq!(a.profile_id, b.profile_id);
    }
}
