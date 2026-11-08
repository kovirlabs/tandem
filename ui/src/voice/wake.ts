// Wake-phrase hints (SPEC §3.3). The core does the actual matching (core/src/audio/wake.rs);
// this only decides whether to show the "may trigger by accident" warning in Settings.

/** Everyday words that show up in normal speech and would make "Hey <name>" fire by accident. */
const COMMON_WORDS = new Set(
  [
    "a", "about", "after", "all", "also", "and", "any", "away", "baby", "back", "bad", "best", "big", "bit",
    "book", "boss", "bro", "buddy", "but", "by", "can", "car", "cat", "come", "cool", "could", "day", "dear",
    "do", "dog", "done", "down", "dude", "even", "every", "fine", "first", "for", "friend", "from", "get",
    "go", "good", "great", "guy", "guys", "have", "hello", "help", "her", "here", "hey", "hi", "him", "home",
    "honey", "how", "just", "kid", "know", "last", "like", "listen", "little", "look", "lot", "love", "make",
    "man", "maybe", "me", "mom", "money", "more", "much", "my", "never", "new", "next", "nice", "no", "not",
    "now", "ok", "okay", "one", "only", "other", "out", "over", "people", "please", "pretty", "really",
    "right", "say", "see", "so", "some", "sorry", "stop", "sure", "sweetie", "take", "thanks", "that", "the",
    "then", "there", "they", "thing", "think", "this", "time", "to", "today", "very", "wait", "want", "way",
    "we", "well", "what", "when", "where", "who", "why", "with", "work", "would", "yeah", "yes", "you",
  ],
);

/** Rough English syllable count; `null` when the name has no Latin letters (no guess). */
export function syllables(word: string): number | null {
  const w = word.toLowerCase().replace(/[^a-z]/g, "");
  if (!w) return null;
  const groups = w.match(/[aeiouy]+/g)?.length ?? 0;
  // A trailing silent "e" (Mike, Jane) doesn't add a syllable; "-le" after a consonant (Bobble) does.
  const silentE = /[^aeiouy]e$/.test(w) && !/[^aeiouy]le$/.test(w) && groups > 1 ? 1 : 0;
  return Math.max(1, groups - silentE);
}

/** True when "Hey <name>" is likely to trigger by accident: one syllable, or a common word. */
export function wakeNameWarning(name: string): boolean {
  const words = name.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return false;
  if (words.length === 1 && COMMON_WORDS.has(words[0] ?? "")) return true;
  const total = words.reduce<number | null>((acc, w) => {
    const n = syllables(w);
    return acc === null || n === null ? null : acc + n;
  }, 0);
  return total !== null && total < 2;
}
