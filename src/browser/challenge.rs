//! Structured detection of observable access challenges. Detection never
//! attempts to defeat a site's control; it returns evidence for policy.
use super::page::PageObservation;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeKind {
    Captcha,
    BotCheck,
    AccessDenied,
    Interstitial,
    JavascriptVerification,
    LoginWall,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChallengeObservation {
    pub detected: bool,
    pub kind: Option<ChallengeKind>,
    pub confidence: f32,
    pub evidence: Vec<String>,
    pub url: String,
    pub title: Option<String>,
}

pub fn detect(page: &PageObservation) -> ChallengeObservation {
    let title = page.title.clone();
    let url = page
        .final_url
        .clone()
        .unwrap_or_else(|| page.requested_url.clone());
    let text = format!(
        "{} {} {}",
        title.as_deref().unwrap_or(""),
        url,
        page.visible_text.as_deref().unwrap_or("")
    )
    .to_ascii_lowercase();
    let patterns: [(ChallengeKind, &[&str], f32); 7] = [
        (
            ChallengeKind::Captcha,
            &["captcha", "hcaptcha", "recaptcha", "verify you are human"],
            0.99,
        ),
        (
            ChallengeKind::BotCheck,
            &[
                "checking your browser",
                "unusual traffic",
                "automated requests",
                "bot detection",
            ],
            0.9,
        ),
        (
            ChallengeKind::AccessDenied,
            &["access denied", "request blocked", "forbidden"],
            0.9,
        ),
        (
            ChallengeKind::Interstitial,
            &[
                "security check",
                "one more step",
                "please wait while we verify",
            ],
            0.8,
        ),
        (
            ChallengeKind::JavascriptVerification,
            &[
                "enable javascript",
                "javascript verification",
                "browser verification",
            ],
            0.75,
        ),
        (
            ChallengeKind::LoginWall,
            &[
                "sign in to continue",
                "log in to view",
                "authentication required",
            ],
            0.75,
        ),
        (ChallengeKind::Unknown, &[], 0.0),
    ];
    let mut evidence = Vec::new();
    for (kind, needles, confidence) in patterns {
        let found: Vec<_> = needles
            .iter()
            .filter(|needle| text.contains(**needle))
            .copied()
            .collect();
        if !found.is_empty() {
            evidence.extend(found.iter().map(|s| (*s).to_owned()));
            return ChallengeObservation {
                detected: true,
                kind: Some(kind),
                confidence,
                evidence,
                url,
                title,
            };
        }
    }
    ChallengeObservation {
        detected: false,
        kind: None,
        confidence: 0.0,
        evidence,
        url,
        title,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::page::PageObservation;
    #[test]
    fn detects_challenge_without_claiming_success() {
        let mut p = PageObservation::new("s", "r", "https://example.test");
        p.title = Some("Verify you are human".into());
        p.visible_text = Some("captcha required".into());
        let d = detect(&p);
        assert!(d.detected);
        assert_eq!(d.kind, Some(ChallengeKind::Captcha));
    }
}
