//! Minimal but correct-enough robots.txt parser (RFC 9309 semantics,
//! simplified): group matching by user-agent substring, longest-match
//! rule wins, `Crawl-delay` extraction.

use std::time::Duration;

// Serializable refusal marker: local and Redis fleets share failure policy.
pub(crate) const UNAVAILABLE_POLICY: &str =
    "# harvest: robots policy unavailable\nUser-agent: *\nDisallow: /\n";

#[derive(Debug, Clone, Default)]
pub struct Robots {
    /// True when the whole site is disallowed for us (`Disallow: /` with no Allow).
    pub full_disallow: bool,
    rules: Vec<(String, bool, regex::bytes::Regex)>, // (path prefix, allowed?)
    pub crawl_delay: Option<Duration>,
    pub sitemap: Option<String>,
    /// True when parsed from an actual robots.txt document (vs. the
    /// absent-file allow-all default). Surfaced in the politeness audit.
    pub present: bool,
    policy_unavailable: bool,
    excessive_delay: bool,
    required_delay: Option<Duration>,
}

#[derive(Debug, Clone, Default)]
struct Group {
    agents: Vec<String>,
    allows: Vec<String>,
    disallows: Vec<String>,
    crawl_delay: Option<f64>,
}

fn normalize_path(p: &str) -> String {
    let t = p.trim();
    if t.is_empty() || t == "/" {
        return t.trim().to_string();
    }
    if t.starts_with('/') {
        t.to_string()
    } else {
        format!("/{t}")
    }
}

impl Robots {
    /// Parse robots.txt content for a given user-agent token (e.g. "harvest").
    pub fn parse(text: &str, ua: &str) -> Robots {
        if text == UNAVAILABLE_POLICY || text.len() > 512 * 1024 || text.lines().count() > 8192 {
            return Robots {
                policy_unavailable: true,
                ..Default::default()
            };
        }
        let ua = ua.to_lowercase();
        let mut groups: Vec<Group> = Vec::new();
        // None while between groups; Some(i) = group currently receiving lines.
        // A new group starts when a `user-agent` line follows any rule line.
        let mut last_was_rule = false;

        for raw in text.lines() {
            let line = match raw.split_once('#') {
                Some((l, _)) => l,
                None => raw,
            };
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            let key = k.trim().to_lowercase();
            let val = v.trim();

            if key == "user-agent" {
                if last_was_rule || groups.is_empty() {
                    groups.push(Group::default());
                }
                groups.last_mut().unwrap().agents.push(val.to_lowercase());
                last_was_rule = false;
                continue;
            }

            // rule line: ensure a group exists
            if groups.is_empty() {
                groups.push(Group::default());
            }
            let g = groups.last_mut().unwrap();
            match key.as_str() {
                "disallow" => g.disallows.push(normalize_path(val)),
                "allow" => g.allows.push(normalize_path(val)),
                "crawl-delay" => {
                    if let Ok(d) = val.parse::<f64>() {
                        if d.is_finite() && d >= 0.0 && Duration::try_from_secs_f64(d).is_ok() {
                            g.crawl_delay = Some(g.crawl_delay.map_or(d, |x| x.max(d)));
                        }
                    }
                }
                "sitemap" => {} // global record; parsed after the loop
                _ => {}
            }
            if matches!(key.as_str(), "allow" | "disallow" | "crawl-delay") {
                last_was_rule = true;
            }
        }

        let sitemap = text
            .lines()
            .filter_map(|l| {
                let (k, v) = l.split_once(':')?;
                if k.trim().eq_ignore_ascii_case("sitemap") && !v.trim().is_empty() {
                    Some(v.trim().to_string())
                } else {
                    None
                }
            })
            .next();

        // Select matching groups: UA-specific beats "*" (RFC 9309 picks the
        // most specific group; several matching groups are merged).
        let mut chosen: Vec<&Group> = Vec::new();
        let mut star: Vec<&Group> = Vec::new();
        for g in &groups {
            let mut match_specific = false;
            let mut match_star = false;
            for agent in &g.agents {
                if agent == "*" {
                    match_star = true;
                } else if !agent.is_empty() && !ua.is_empty() && ua.contains(agent) {
                    match_specific = true;
                }
            }
            if match_specific {
                chosen.push(g);
            } else if match_star {
                star.push(g);
            }
        }
        let used: Vec<&Group> = if chosen.is_empty() { star } else { chosen };

        let mut allows: Vec<String> = Vec::new();
        let mut disallows: Vec<String> = Vec::new();
        let mut delays: Vec<f64> = Vec::new();
        for g in used {
            allows.extend(g.allows.iter().filter(|p| !p.is_empty()).cloned());
            disallows.extend(g.disallows.iter().filter(|p| !p.is_empty()).cloned());
            if let Some(d) = g.crawl_delay {
                delays.push(d);
            }
        }

        let mut out = Robots::default();
        let all_disallowed = disallows.iter().any(|p| p == "/");
        let allow_all = allows.iter().any(|p| p == "/");
        if all_disallowed && !allow_all {
            out.full_disallow = true;
        }
        if disallows.len() + allows.len() > 4096 {
            out.policy_unavailable = true;
            return out;
        }
        for (raw, allow) in disallows
            .into_iter()
            .map(|p| (p, false))
            .chain(allows.into_iter().map(|p| (p, true)))
        {
            if raw.len() > 8192 {
                out.policy_unavailable = true;
                return out;
            }
            let exact = raw.ends_with('$');
            let body = if exact {
                &raw[..raw.len() - 1]
            } else {
                raw.as_str()
            };
            let pattern = normalize_octets(body, true);
            let mut source = String::from("^");
            for (i, literal) in pattern.split('*').enumerate() {
                if i > 0 {
                    source.push_str(".*");
                }
                source.push_str(&regex::escape(literal));
            }
            if exact {
                source.push('$');
            }
            let matcher = regex::bytes::RegexBuilder::new(&source)
                .unicode(false)
                .size_limit(256 * 1024)
                .dfa_size_limit(64 * 1024)
                .build();
            let Ok(matcher) = matcher else {
                out.policy_unavailable = true;
                return out;
            };
            out.rules.push((pattern, allow, matcher));
        }
        if let Some(&d) = delays.iter().max_by(|a, b| a.total_cmp(b)) {
            out.required_delay = Some(Duration::from_secs_f64(d));
            if d > 3600.0 {
                out.excessive_delay = true;
            } else {
                out.crawl_delay = Some(Duration::from_secs_f64(d));
            }
        }
        out.sitemap = sitemap;
        out.present = true;
        out
    }

    pub(crate) fn required_delay(&self) -> Option<Duration> {
        self.required_delay
    }

    /// Match the URL's path AND query, never its fragment or authority.
    pub(crate) fn allows_url(&self, url: &url::Url) -> bool {
        let mut path = url.path().to_string();
        if let Some(query) = url.query() {
            path.push('?');
            path.push_str(query);
        }
        self.allows(&path)
    }

    /// Should `path` be fetched? Empty rule set => allow everything.
    pub fn allows(&self, path: &str) -> bool {
        if self.policy_unavailable || self.excessive_delay {
            return false;
        }
        if self.rules.is_empty() {
            return true;
        }
        let path = normalize_octets(if path.is_empty() { "/" } else { path }, false);
        let mut best: Option<(usize, bool)> = None; // (prefix len, allow?)
        for (prefix, allow, matcher) in &self.rules {
            if prefix.is_empty() {
                continue;
            }
            if matcher.is_match(path.as_bytes()) {
                let len = specificity(prefix);
                match best {
                    Some((bl, ba)) if bl > len || (bl == len && (ba || !*allow)) => {}
                    _ => best = Some((len, *allow)),
                }
            }
        }
        best.map(|(_, a)| a).unwrap_or(!self.full_disallow)
    }
}

/// RFC 9309 octet comparison: percent-encoded unreserved ASCII decodes;
/// reserved/non-ASCII octets compare in uppercase percent-encoded form.
/// `*` is syntax in rules only, never in the requested URL.
fn normalize_octets(text: &str, rule: bool) -> String {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    let mut query = false;
    let mut value = false;
    while i < bytes.len() {
        let mut b = bytes[i];
        if b == b'%' && i + 2 < bytes.len() {
            let h = (bytes[i + 1] as char).to_digit(16);
            let l = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (h, l) {
                b = (h * 16 + l) as u8;
                i += 3;
                if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                    out.push(b as char);
                } else {
                    out.push_str(&format!("%{b:02X}"));
                }
                continue;
            }
        }
        i += 1;
        if !query && b == b'?' {
            query = true;
            out.push('?');
            continue;
        }
        if query && b == b'&' {
            value = false;
            out.push('&');
            continue;
        }
        if query && !value && b == b'=' {
            value = true;
            out.push('=');
            continue;
        }
        if !query && b == b'/' {
            out.push('/');
            continue;
        }
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'.' | b'_' | b'~')
            || (rule && b == b'*')
        {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
fn specificity(pattern: &str) -> usize {
    let mut count = 0;
    let mut i = 0;
    let bytes = pattern.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'*' {
            i += 1;
            continue;
        }
        count += 1;
        i += if bytes[i] == b'%' && i + 2 < bytes.len() {
            3
        } else {
            1
        };
    }
    count
}

#[cfg(test)]
mod tests;
