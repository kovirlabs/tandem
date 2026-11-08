//! OWNER: output agent. Turns streamed reply text into speakable sentences (plan task O2).
//!
//! Text arrives in small deltas. We hold it until a sentence ends, clean it for speech
//! (markdown, emoji, `<think>` blocks) and hand it to the TTS queue, so sentence one can be
//! synthesised while the model is still writing sentence two.

/// A first chunk shorter than this is merged with the next one (very short clips sound choppy).
const MIN_FIRST_CHARS: usize = 20;

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

/// Words that end with a period but don't end a sentence (compared lowercase, without the dot).
const ABBREVIATIONS: &[&str] = &[
    "e.g", "i.e", "dr", "mr", "mrs", "ms", "mx", "st", "vs", "prof", "approx", "no", "fig", "jr",
    "sr",
];

#[derive(Debug, Default)]
pub struct SentenceSplitter {
    /// Raw text not yet scanned for `<think>` tags (may hold a partial tag).
    pending: String,
    /// Visible text waiting for a sentence boundary.
    text: String,
    in_think: bool,
    /// A short first sentence held back to merge with the next one.
    held: Option<String>,
    emitted_any: bool,
}

impl SentenceSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one streamed delta. Returns every sentence that is now complete, already cleaned.
    pub fn push(&mut self, delta: &str) -> Vec<String> {
        self.pending.push_str(delta);
        self.drain_think(false);
        let mut out = Vec::new();
        while let Some(end) = find_boundary(&self.text) {
            let raw: String = self.text.drain(..end).collect();
            if let Some(s) = self.accept(&raw, false) {
                out.push(s);
            }
        }
        out
    }

    /// The reply is complete: return whatever is left (including a held short first sentence).
    pub fn finish(&mut self) -> Option<String> {
        self.drain_think(true);
        let raw = std::mem::take(&mut self.text);
        let out = self.accept(&raw, true);
        self.in_think = false;
        out
    }

    /// Move visible text from `pending` to `text`, dropping `<think>…</think>` content.
    /// Keeps a possible partial tag at the end of `pending` unless `final_flush`.
    fn drain_think(&mut self, final_flush: bool) {
        loop {
            if self.in_think {
                if let Some(i) = self.pending.find(THINK_CLOSE) {
                    self.pending.drain(..i + THINK_CLOSE.len());
                    self.in_think = false;
                    continue;
                }
                // Drop thought text but keep a tail that could be the start of `</think>`.
                let keep = if final_flush {
                    0
                } else {
                    partial_suffix_len(&self.pending, THINK_CLOSE)
                };
                let cut = self.pending.len() - keep;
                self.pending.drain(..cut);
                return;
            }
            if let Some(i) = self.pending.find(THINK_OPEN) {
                let before: String = self.pending.drain(..i).collect();
                self.text.push_str(&before);
                self.pending.drain(..THINK_OPEN.len());
                self.in_think = true;
                continue;
            }
            let keep = if final_flush {
                0
            } else {
                partial_suffix_len(&self.pending, THINK_OPEN)
            };
            let cut = self.pending.len() - keep;
            let visible: String = self.pending.drain(..cut).collect();
            self.text.push_str(&visible);
            return;
        }
    }

    /// Clean a raw sentence and apply the short-first-sentence merge.
    fn accept(&mut self, raw: &str, last: bool) -> Option<String> {
        let cleaned = clean_for_speech(raw);
        let combined = match (self.held.take(), cleaned.is_empty()) {
            (Some(h), true) => h,
            (Some(h), false) => format!("{h} {cleaned}"),
            (None, true) => return None,
            (None, false) => cleaned,
        };
        if !self.emitted_any && !last && combined.chars().count() < MIN_FIRST_CHARS {
            self.held = Some(combined);
            return None;
        }
        self.emitted_any = true;
        Some(combined)
    }
}

/// Length in bytes of the longest suffix of `s` that is a proper prefix of `tag`.
fn partial_suffix_len(s: &str, tag: &str) -> usize {
    (1..tag.len())
        .rev()
        .find(|&n| {
            s.len() >= n && s.is_char_boundary(s.len() - n) && tag.starts_with(&s[s.len() - n..])
        })
        .unwrap_or(0)
}

/// Byte index just past the first sentence boundary in `text`, if there is one.
/// A boundary is `.`, `!`, `?` or `…` (plus closing quotes/brackets) followed by whitespace,
/// or a newline. Decimals ("v1.2") never match because no whitespace follows the dot.
fn find_boundary(text: &str) -> Option<usize> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        if c == '\n' {
            return Some(pos + 1);
        }
        if matches!(c, '.' | '!' | '?' | '…') {
            // Swallow runs like "?!" or "..." and closing quotes/brackets.
            let mut j = i + 1;
            while j < chars.len()
                && matches!(
                    chars[j].1,
                    '.' | '!' | '?' | '…' | '"' | '\'' | ')' | '”' | '’' | '*' | '_'
                )
            {
                j += 1;
            }
            if j >= chars.len() {
                return None; // need to see what follows
            }
            if chars[j].1.is_whitespace() {
                if c == '.' && j == i + 1 && is_abbreviation(text, pos) {
                    i = j;
                    continue;
                }
                return Some(chars[j].0 + chars[j].1.len_utf8());
            }
            i = j;
            continue;
        }
        i += 1;
    }
    None
}

/// True if the `.` at byte `dot` ends an abbreviation, a single initial, or a list number.
fn is_abbreviation(text: &str, dot: usize) -> bool {
    let before = &text[..dot];
    let start = before
        .rfind(|c: char| c.is_whitespace() || c == '(' || c == '"')
        .map(|i| i + 1)
        .unwrap_or(0);
    let word = &before[start..];
    if word.is_empty() {
        return false;
    }
    let lower = word.to_lowercase();
    if ABBREVIATIONS.contains(&lower.as_str()) {
        return true;
    }
    // Single initial like "J." in "J. R. R. Tolkien".
    let mut it = word.chars();
    if let (Some(c), None) = (it.next(), it.next()) {
        if c.is_alphabetic() && c.is_uppercase() {
            return true;
        }
    }
    // "1. Open the doc" at the start of a line is a list marker, not a sentence.
    let line_start = before[..start].trim_end_matches([' ', '\t']).is_empty()
        || before[..start]
            .trim_end_matches([' ', '\t'])
            .ends_with('\n');
    line_start && word.chars().all(|c| c.is_ascii_digit())
}

/// Make one chunk of model text speakable: drop markdown syntax, emoji and list markers,
/// keep link text, collapse whitespace.
pub fn clean_for_speech(raw: &str) -> String {
    let no_links = strip_links(raw);
    let mut out = String::with_capacity(no_links.len());
    for line in no_links.lines() {
        let line = strip_list_marker(line.trim());
        for c in line.chars() {
            if matches!(c, '*' | '_' | '#' | '`' | '~' | '>' | '|') || is_emoji(c) {
                continue;
            }
            out.push(c);
        }
        out.push(' ');
    }
    let mut joined = out.split_whitespace().collect::<Vec<_>>().join(" ");
    // Removing an emoji can leave "line ." behind.
    for p in [" .", " ,", " !", " ?", " ;", " :"] {
        joined = joined.replace(p, &p[1..]);
    }
    joined
}

/// `[text](url)` → `text`; bare `http(s)://…` URLs are dropped.
fn strip_links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        if let Some(close) = after.find("](") {
            if let Some(end) = after[close + 2..].find(')') {
                out.push_str(&rest[..open]);
                out.push_str(&after[..close]);
                rest = &after[close + 2 + end + 1..];
                continue;
            }
        }
        out.push_str(&rest[..open + 1]);
        rest = after;
    }
    out.push_str(rest);
    out.split(' ')
        .filter(|w| !(w.starts_with("http://") || w.starts_with("https://")))
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_list_marker(line: &str) -> &str {
    for m in ["- ", "* ", "+ ", "• "] {
        if let Some(rest) = line.strip_prefix(m) {
            return rest;
        }
    }
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits <= 3 {
        let rest = &line[digits..];
        if let Some(r) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return r;
        }
    }
    line
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF   // pictographs, emoticons, transport, flags, supplemental symbols
        | 0x2600..=0x27BF   // misc symbols, dingbats
        | 0x2B00..=0x2BFF   // arrows & stars like ⭐
        | 0x2300..=0x23FF   // ⌚ ⏰ etc.
        | 0xFE00..=0xFE0F   // variation selectors
        | 0x200D            // zero-width joiner
        | 0x20E3            // keycap
        | 0xE0020..=0xE007F // tags
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(parts: &[&str]) -> Vec<String> {
        let mut s = SentenceSplitter::new();
        let mut out = Vec::new();
        for p in parts {
            out.extend(s.push(p));
        }
        out.extend(s.finish());
        out
    }

    #[test]
    fn splits_streamed_deltas() {
        let mut s = SentenceSplitter::new();
        let mut got = Vec::new();
        for d in [
            "Let's start with the outl",
            "ine for your essay. Then we",
            "'ll pick ",
            "one quote! Sound good?",
            " Great",
        ] {
            got.extend(s.push(d));
        }
        assert_eq!(
            got,
            vec![
                "Let's start with the outline for your essay.",
                "Then we'll pick one quote!",
                "Sound good?"
            ]
        );
        assert_eq!(s.finish().as_deref(), Some("Great"));
    }

    #[test]
    fn keeps_decimals_and_abbrev() {
        let got =
            feed(&["Version v1.2 is out, e.g. for Dr. Lee and J. Smith. It costs 3.50 now. Done"]);
        assert_eq!(
            got,
            vec![
                "Version v1.2 is out, e.g. for Dr. Lee and J. Smith.",
                "It costs 3.50 now.",
                "Done"
            ]
        );
    }

    #[test]
    fn short_first_merges() {
        let got = feed(&["Okay! ", "Let's open the document together. ", "Nice."]);
        assert_eq!(
            got,
            vec!["Okay! Let's open the document together.", "Nice."]
        );
        // A short reply on its own is still spoken.
        assert_eq!(feed(&["Got it!"]), vec!["Got it!"]);
    }

    #[test]
    fn strips_markdown_and_emoji() {
        let got = feed(&[
            "**Great** job 🎉 on the `draft`! ",
            "## Next\n- Open [the doc](https://example.com/x) and _skim_ it. ",
            "1. Write one line ✍️.",
        ]);
        assert_eq!(
            got,
            vec![
                "Great job on the draft!",
                "Next",
                "Open the doc and skim it.",
                "Write one line."
            ]
        );
    }

    #[test]
    fn finish_flushes_tail() {
        let mut s = SentenceSplitter::new();
        assert!(s
            .push("You've got this, one small step at a time")
            .is_empty());
        assert_eq!(
            s.finish().as_deref(),
            Some("You've got this, one small step at a time")
        );
        assert_eq!(s.finish(), None);
    }

    #[test]
    fn think_removed_across_deltas() {
        let got = feed(&[
            "<thi",
            "nk>The user seems stuck. Suggest. a tiny step.</th",
            "ink>Try opening the file for just two minutes. ",
            "I'm right here <think>keep it short</think>with you.",
        ]);
        assert_eq!(
            got,
            vec![
                "Try opening the file for just two minutes.",
                "I'm right here with you."
            ]
        );
    }

    #[test]
    fn unterminated_think_is_dropped_on_finish() {
        assert!(feed(&["<think>still thinking about it"]).is_empty());
    }

    #[test]
    fn partial_suffix() {
        assert_eq!(partial_suffix_len("abc <th", THINK_OPEN), 3);
        assert_eq!(partial_suffix_len("abc", THINK_OPEN), 0);
        assert_eq!(partial_suffix_len("x</thin", THINK_CLOSE), 6);
    }
}
