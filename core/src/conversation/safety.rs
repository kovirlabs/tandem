//! Deterministic crisis safety net (CLAUDE.md rule 6).
//!
//! The safety block in the system prompt does the real work. Small local models sometimes
//! answer kindly but forget the resources, so if the user's message matches a crisis phrase
//! and the reply has no resources, the core appends them. It never removes anything the
//! model said, and it also covers the case where no provider is reachable at all.

use std::sync::OnceLock;

const PATTERNS: &str = include_str!("../../../prompts/crisis_patterns.txt");
const RESOURCES: &str = include_str!("../../../prompts/crisis_resources.md");

fn patterns() -> &'static [String] {
    static P: OnceLock<Vec<String>> = OnceLock::new();
    P.get_or_init(|| {
        PATTERNS
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(normalize)
            .collect()
    })
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .replace(['\u{2019}', '\u{2018}'], "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Does this user message look like a crisis disclosure?
pub fn is_crisis(user_text: &str) -> bool {
    let t = normalize(user_text);
    patterns().iter().any(|p| t.contains(p.as_str()))
}

/// Does the reply already point to crisis resources?
pub fn has_resources(reply: &str) -> bool {
    let r = reply.to_lowercase();
    ["988", "116 123", "findahelpline", "emergency"]
        .iter()
        .any(|m| r.contains(m))
}

pub fn resources() -> &'static str {
    RESOURCES.trim()
}

/// Return the reply with resources appended when needed.
pub fn ensure_resources(user_text: &str, reply: &str) -> String {
    if !is_crisis(user_text) || has_resources(reply) {
        return reply.to_string();
    }
    if reply.trim().is_empty() {
        resources().to_string()
    } else {
        format!("{}\n\n{}", reply.trim(), resources())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_direct_and_indirect_phrases() {
        for t in [
            "I don't want to be alive anymore.",
            "everyone would be better off without me tbh",
            "I keep thinking about HURTING   myself",
            "honestly i want to die",
            "I can\u{2019}t go on like this",
        ] {
            assert!(is_crisis(t), "{t}");
        }
        for t in [
            "this report is killing me",
            "I wasted the whole day",
            "I don't want to be a burden on the team",
            "let's do 25 minutes",
        ] {
            assert!(!is_crisis(t), "{t}");
        }
    }

    #[test]
    fn appends_only_when_missing() {
        let user = "everyone would be better off without me";
        let kind = "I'm really glad you told me.";
        let out = ensure_resources(user, kind);
        assert!(out.starts_with(kind));
        assert!(out.contains("988") && out.contains("116 123"));
        let already = "Please call or text 988 right now.";
        assert_eq!(ensure_resources(user, already), already);
        assert_eq!(ensure_resources("hello", "Hi."), "Hi.");
        assert_eq!(ensure_resources(user, ""), resources());
    }

    #[test]
    fn pattern_file_is_loaded() {
        assert!(patterns().len() > 20);
        assert!(patterns().iter().all(|p| p == &p.to_lowercase()));
    }
}
