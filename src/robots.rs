//! Minimal but correct-enough robots.txt parser (RFC 9309 semantics,
//! simplified): group matching by user-agent substring, longest-match
//! rule wins, `Crawl-delay` extraction.

use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct Robots {
    /// True when the whole site is disallowed for us (`Disallow: /` with no Allow).
    pub full_disallow: bool,
    rules: Vec<(String, bool)>, // (path prefix, allowed?)
    pub crawl_delay: Option<Duration>,
    pub sitemap: Option<String>,
    /// True when parsed from an actual robots.txt document (vs. the
    /// absent-file allow-all default). Surfaced in the politeness audit.
    pub present: bool,
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
                        g.crawl_delay = Some(g.crawl_delay.map_or(d, |x| x.min(d)));
                    }
                }
                "sitemap" => {} // global record; parsed after the loop
                _ => {}
            }
            last_was_rule = true;
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
                } else if !ua.is_empty() && agent.contains(&ua) {
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
        for p in disallows {
            out.rules.push((p, false));
        }
        for p in allows {
            out.rules.push((p, true));
        }
        if let Some(&d) = delays.iter().min_by(|a, b| a.total_cmp(b)) {
            out.crawl_delay = Some(Duration::from_secs_f64(d));
        }
        out.sitemap = sitemap;
        out.present = true;
        out
    }

    /// Should `path` be fetched? Empty rule set => allow everything.
    pub fn allows(&self, path: &str) -> bool {
        if self.rules.is_empty() {
            return true;
        }
        let path = if path.is_empty() { "/" } else { path };
        let mut best: Option<(usize, bool)> = None; // (prefix len, allow?)
        for (prefix, allow) in &self.rules {
            if prefix.is_empty() {
                continue;
            }
            if path.starts_with(prefix.as_str()) {
                let len = prefix.len();
                match best {
                    Some((bl, _)) if bl >= len => {}
                    _ => best = Some((len, *allow)),
                }
            }
        }
        best.map(|(_, a)| a).unwrap_or(!self.full_disallow)
    }
}

#[cfg(test)]
mod tests;
