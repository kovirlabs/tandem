//! Wake phrase ("Hey {companion_name}") detected on transcripts (M2 Part C).
//!
//! No extra model: hands-free segments are transcribed anyway, so we look for a greeting
//! followed by the companion's name. This follows renames for free and never hard-codes a name.

/// Result of looking for the wake phrase in one transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeMatch {
    /// No wake phrase: drop the segment.
    None,
    /// Only the wake phrase ("Hey Timber."): open a short window for the next segment.
    Only,
    /// Wake phrase plus a request: send the rest.
    WithRest(String),
}

const GREETINGS: &[&str] = &["hey", "hi", "okay", "ok"];
/// How many words may come before the greeting ("um, hey Timber").
const MAX_LEAD_WORDS: usize = 2;

/// A word of the transcript with its byte span in the original text.
struct Word {
    norm: String,
    end: usize,
}

fn words(text: &str) -> Vec<Word> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut end = 0;
    for (i, c) in text.char_indices() {
        if c.is_alphanumeric() || c == '\'' {
            if c != '\'' {
                cur.extend(c.to_lowercase());
            }
            end = i + c.len_utf8();
        } else if !cur.is_empty() {
            out.push(Word {
                norm: std::mem::take(&mut cur),
                end,
            });
        }
    }
    if !cur.is_empty() {
        out.push(Word { norm: cur, end });
    }
    out
}

fn normalise_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Edit distance over chars (optimal string alignment: insert, delete, substitute, or swap
/// two neighbouring letters, each counting as one edit).
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let w = b.len() + 1;
    let mut d = vec![0usize; (a.len() + 1) * w];
    for (i, row) in d.chunks_mut(w).enumerate() {
        row[0] = i;
    }
    for (j, cell) in d.iter_mut().enumerate().take(w) {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[(i - 1) * w + j] + 1)
                .min(d[i * w + j - 1] + 1)
                .min(d[(i - 1) * w + j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[(i - 2) * w + j - 2] + 1);
            }
            d[i * w + j] = v;
        }
    }
    d[a.len() * w + b.len()]
}

fn allowed_edits(name: &str) -> usize {
    if name.chars().count() <= 6 {
        1
    } else {
        2
    }
}

/// Look for "hey/hi/okay + name" near the start of `transcript`. Case and punctuation are
/// ignored, and the name may be off by one edit (two for names longer than six letters),
/// because speech-to-text often misspells unusual names.
pub fn match_wake(transcript: &str, name: &str) -> WakeMatch {
    let target = normalise_name(name);
    if target.is_empty() {
        return WakeMatch::None;
    }
    let max_edits = allowed_edits(&target);
    let name_words = name.split_whitespace().count().max(1);
    let ws = words(transcript);
    for g in 0..ws.len().min(MAX_LEAD_WORDS + 1) {
        if !GREETINGS.contains(&ws[g].norm.as_str()) {
            continue;
        }
        // The name may be split or merged differently by the transcriber ("Mr Fox" / "Mrfox").
        let mut best: Option<(usize, usize)> = None; // (distance, last word index)
        for take in name_words.saturating_sub(1).max(1)..=name_words + 1 {
            let last = g + take;
            if last >= ws.len() {
                break;
            }
            let joined: String = ws[g + 1..=last].iter().map(|w| w.norm.as_str()).collect();
            let d = edit_distance(&joined, &target);
            if d <= max_edits && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, last));
            }
        }
        if let Some((_, last)) = best {
            let rest = transcript[ws[last].end..]
                .trim_start_matches(|c: char| !c.is_alphanumeric())
                .trim();
            return if rest.chars().any(char::is_alphanumeric) {
                WakeMatch::WithRest(rest.to_string())
            } else {
                WakeMatch::Only
            };
        }
    }
    WakeMatch::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_with_rest() {
        assert_eq!(
            match_wake("Hey Timber, help me start my essay.", "Timber"),
            WakeMatch::WithRest("help me start my essay.".into())
        );
        assert_eq!(
            match_wake("okay timber what's next", "Timber"),
            WakeMatch::WithRest("what's next".into())
        );
    }

    #[test]
    fn wake_only_opens_window() {
        assert_eq!(match_wake("Hey, Timber!", "Timber"), WakeMatch::Only);
        assert_eq!(match_wake("  hi timber...  ", "Timber"), WakeMatch::Only);
    }

    #[test]
    fn renamed_companion() {
        assert_eq!(
            match_wake("Hey Maple, I'm stuck", "Maple"),
            WakeMatch::WithRest("I'm stuck".into())
        );
        assert_eq!(
            match_wake("Hey Timber, I'm stuck", "Maple"),
            WakeMatch::None
        );
        assert_eq!(
            match_wake("Hi Mr Fox, ready", "Mr Fox"),
            WakeMatch::WithRest("ready".into())
        );
    }

    #[test]
    fn no_wake_dropped() {
        assert_eq!(
            match_wake("I need to start my essay", "Timber"),
            WakeMatch::None
        );
        assert_eq!(match_wake("Timber, help", "Timber"), WakeMatch::None);
        assert_eq!(match_wake("", "Timber"), WakeMatch::None);
        assert_eq!(match_wake("hey", "Timber"), WakeMatch::None);
        assert_eq!(match_wake("hey timber", ""), WakeMatch::None);
    }

    #[test]
    fn fuzzy_name_one_edit() {
        // Short names: one edit allowed.
        assert_eq!(
            match_wake("Hey Timbre, go", "Timber"),
            WakeMatch::WithRest("go".into())
        );
        assert_eq!(match_wake("Hey Timer", "Timber"), WakeMatch::Only);
        assert_eq!(match_wake("Hey Tumblr", "Timber"), WakeMatch::None);
        // Longer names: two edits allowed.
        assert_eq!(match_wake("Hey Bartolomeu", "Bartholomew"), WakeMatch::Only);
        // Leading filler is fine.
        assert_eq!(
            match_wake("Um, hey Timber, hi", "Timber"),
            WakeMatch::WithRest("hi".into())
        );
    }

    #[test]
    fn edit_distance_basics() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("same", "same"), 0);
        assert_eq!(edit_distance("timbre", "timber"), 1);
    }
}
