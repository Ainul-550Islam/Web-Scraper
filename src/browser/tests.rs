//! Cross-module tests for the browser subsystem's pure logic.
//!
//! The browser feature is optional and its runtime needs a WebDriver endpoint,
//! so this file covers the parts that must hold regardless: deterministic
//! profile assignment, the consistency contract between runtime config and
//! profile, device-profile validation, and secret redaction on error surfaces.

use std::path::PathBuf;

use crate::browser::config::{BrowserConfig, BrowserEngine, Viewport};
use crate::browser::device::{standard_profiles, DeviceClass, DeviceProfile};
use crate::browser::error::{safe_message, BrowserError};
use crate::browser::fingerprint::{check_consistency, select_stable};
use crate::browser::profile::BrowserProfile;

fn profile(id: &str) -> BrowserProfile {
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

fn config() -> BrowserConfig {
    BrowserConfig::default()
}

#[test]
fn stable_assignment_is_deterministic_and_within_range() {
    let profiles = vec![profile("alpha"), profile("beta"), profile("gamma")];
    let (first, first_assignment) = select_stable(&profiles, "target-a", "seed-1").unwrap();
    for _ in 0..32 {
        let (again, assignment) = select_stable(&profiles, "target-a", "seed-1").unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(assignment.stable_bucket, first_assignment.stable_bucket);
        assert_eq!(assignment.target_id, "target-a");
    }
    assert!(first_assignment.stable_bucket < profiles.len());

    // A different key can land on a different profile, but never outside the set.
    let (other, other_assignment) = select_stable(&profiles, "target-b", "seed-1").unwrap();
    assert!(profiles.iter().any(|p| p.id == other.id));
    assert!(other_assignment.stable_bucket < profiles.len());
}

#[test]
fn stable_assignment_rejects_empty_profiles_and_oversized_keys() {
    assert!(select_stable(&[], "target", "seed").is_err());
    assert!(select_stable(&[profile("a")], "", "seed").is_err());
    assert!(select_stable(&[profile("a")], "target", "").is_ok());
    assert!(select_stable(&[profile("a")], &"t".repeat(129), "seed").is_err());
    assert!(select_stable(&[profile("a")], "target", &"s".repeat(257)).is_err());
    assert!(select_stable(&[profile("a")], "target", &"s".repeat(256)).is_ok());
}

#[test]
fn stable_assignment_skips_expired_and_invalid_profiles() {
    let mut expired = profile("old");
    expired.expires_at_ms = Some(1);
    let mut invalid = profile("bad id!");
    invalid.version = 0;
    let profiles = vec![expired.clone(), invalid, profile("good")];
    let (chosen, assignment) = select_stable(&profiles, "target", "seed").unwrap();
    assert_eq!(chosen.id, "good");
    assert_eq!(assignment.profile_id, "good");

    // With every candidate filtered out the assignment must fail, not fall
    // back to an unusable profile.
    let all_expired = vec![expired.clone()];
    assert!(select_stable(&all_expired, "target", "seed").is_err());
}

#[test]
fn consistency_rejects_engine_viewport_and_locale_drift() {
    let mut wrong_engine = profile("p");
    wrong_engine.engine = BrowserEngine::Firefox;
    assert!(check_consistency(&config(), &wrong_engine).is_err());

    let mut wrong_viewport = profile("p");
    wrong_viewport.viewport = Viewport {
        width: 800,
        height: 600,
        device_scale_factor: 1.0,
    };
    assert!(check_consistency(&config(), &wrong_viewport).is_err());

    let mut wrong_locale = profile("p");
    wrong_locale.locale = "de-DE".into();
    assert!(check_consistency(&config(), &wrong_locale).is_err());

    let mut ua = profile("p");
    ua.user_agent = Some("another-agent".into());
    let mut cfg = config();
    cfg.user_agent_override = Some("the-configured-agent".into());
    assert!(check_consistency(&cfg, &ua).is_err());

    // The same override on both sides is consistent, and an unset override
    // leaves a profile's own user agent alone.
    ua.user_agent = Some("the-configured-agent".into());
    assert!(check_consistency(&cfg, &ua).is_ok());
    assert!(check_consistency(&config(), &ua).is_ok());
    assert!(check_consistency(&config(), &profile("p")).is_ok());
}

#[test]
fn consistency_rejects_a_firefox_profile_pointed_at_chrome() {
    let mut firefox = profile("fx");
    firefox.engine = BrowserEngine::Firefox;
    let mut cfg = config();
    cfg.engine = BrowserEngine::Firefox;
    assert!(check_consistency(&cfg, &firefox).is_ok());

    cfg.browser_executable = Some(PathBuf::from("/opt/browsers/chrome"));
    assert!(check_consistency(&cfg, &firefox).is_err());

    // The same executable is fine when the runtime and profile agree on Chrome.
    let mut chrome_cfg = config();
    chrome_cfg.browser_executable = Some(PathBuf::from("/opt/browsers/chrome"));
    assert!(check_consistency(&chrome_cfg, &profile("chrome")).is_ok());
}

#[test]
fn standard_profiles_are_valid_unique_and_round_trip() {
    let profiles = standard_profiles();
    assert!(
        profiles.len() >= 3,
        "expected desktop/tablet/mobile at minimum"
    );

    let mut ids: Vec<&str> = profiles.iter().map(|p| p.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(
        ids.len(),
        profiles.len(),
        "device profile ids must be unique"
    );

    for profile in &profiles {
        assert!(
            profile.validate().is_ok(),
            "profile {} must validate",
            profile.id
        );
        assert!(
            matches!(
                profile.class,
                DeviceClass::Desktop | DeviceClass::Tablet | DeviceClass::Mobile
            ),
            "unexpected class"
        );
        let converted = profile.clone().into_profile(1_700_000_000_000);
        assert_eq!(converted.id, profile.id);
        assert_eq!(converted.engine, profile.engine);
        assert_eq!(converted.viewport.width, profile.viewport.width);
        assert_eq!(converted.viewport.height, profile.viewport.height);
        assert_eq!(
            converted.viewport.device_scale_factor,
            profile.viewport.device_scale_factor
        );
        assert_eq!(converted.locale, profile.locale);
        assert_eq!(converted.timezone, profile.timezone);
        assert_eq!(converted.user_agent, profile.user_agent);
        assert_eq!(converted.created_at_ms, 1_700_000_000_000);
        assert_eq!(converted.expires_at_ms, None);
        assert!(converted.validate().is_ok());
    }
}

#[test]
fn device_profile_validation_enforces_bounds() {
    let mut base = DeviceProfile {
        id: "phone".into(),
        class: DeviceClass::Mobile,
        engine: BrowserEngine::Chrome,
        viewport: Viewport::default(),
        locale: "en-US".into(),
        timezone: None,
        user_agent: None,
    };
    assert!(base.validate().is_ok());

    base.id = String::new();
    assert!(base.validate().is_err());
    base.id = "x".repeat(129);
    assert!(base.validate().is_err());
    base.id = "phone".into();

    base.viewport.width = 239;
    assert!(base.validate().is_err());
    base.viewport.width = 7681;
    assert!(base.validate().is_err());
    base.viewport = Viewport::default();

    base.locale = String::new();
    assert!(base.validate().is_err());
    base.locale = "e".repeat(65);
    assert!(base.validate().is_err());
}

#[test]
fn safe_message_redacts_credentials_and_url_userinfo() {
    assert_eq!(
        safe_message("password=hunter2&user=bob"),
        "password=[REDACTED]&user=bob"
    );
    assert_eq!(
        safe_message("Authorization: token=abc123; x=1"),
        "Authorization: token=[REDACTED]; x=1"
    );
    assert_eq!(
        safe_message("https://user:secret@example.com/path"),
        "https://[REDACTED]@example.com/path"
    );
    // A URL without userinfo is left alone.
    assert_eq!(
        safe_message("https://example.com/path"),
        "https://example.com/path"
    );
    // Truncation is a hard bound, so a long message cannot grow unbounded.
    assert_eq!(safe_message(&"x".repeat(400)).len(), 256);
}

#[test]
fn browser_error_display_redacts_credential_bearing_values() {
    // `safe_message` redacts the documented credential shapes; the error
    // surfaces must never echo the raw value back out.
    let secret = "sup3r-secret-token-value";
    let rendered =
        BrowserError::Config(format!("driver refused connection: token={secret}")).to_string();
    assert!(
        !rendered.contains(secret),
        "display leaked the secret: {rendered}"
    );
    assert!(rendered.contains("[REDACTED]"));

    let with_password =
        BrowserError::Policy(format!("policy denied: password={secret}&user=bob")).to_string();
    assert!(!with_password.contains(secret));
    assert!(with_password.contains("[REDACTED]"));

    let with_url =
        BrowserError::Launch(format!("failed to open https://u:{secret}@host/x")).to_string();
    assert!(!with_url.contains(secret));
    assert!(with_url.contains("[REDACTED]"));

    assert_eq!(
        BrowserError::Timeout { operation: "click" }.to_string(),
        "browser click timed out"
    );
    assert_eq!(
        BrowserError::InvalidInput("profile id").to_string(),
        "invalid browser profile id"
    );
}
