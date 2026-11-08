//! Prompt eval runner (`pnpm evals`). Runs YAML scenarios in `evals/scenarios/` through the real
//! orchestrator, prompt assembly and tool pipeline against a live model, then checks a rubric.
//!
//! Provider selection (env):
//!   EVAL_PROVIDER=ollama (default) with OLLAMA_URL and optional OLLAMA_MODEL
//!   EVAL_PROVIDER=anthropic with ANTHROPIC_API_KEY and optional ANTHROPIC_MODEL
//! Usage: pnpm evals [scenario-id-substring]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use tandem_core::checkin::{CheckinAction, CheckinReason};
use tandem_core::conversation::prompt::Tone;
use tandem_core::conversation::Via;
use tandem_core::events::NullSink;
use tandem_core::providers::anthropic::{self, AnthropicProvider};
use tandem_core::providers::ollama::OllamaProvider;
use tandem_core::providers::ChatProvider;
use tandem_core::secrets::MemorySecretStore;
use tandem_core::session::SessionPhase;
use tandem_core::store::Store;
use tandem_core::Core;

/// Applied to every reply in every scenario (CLAUDE.md rule 5).
const GLOBAL_BANNED: &[&str] = &[
    r"(?i)\byou should\b",
    r"(?i)\blazy\b",
    r"(?i)\bwasted\b",
    r"(?i)\bno excuses?\b",
    r"(?i)\bdisappoint",
    r"(?i)\bfocus score\b",
    r"(?i)\bstreak\b",
    r"(?i)\bshame\b",
];

#[derive(Debug, Deserialize)]
struct Scenario {
    id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    setup: Setup,
    /// Either user turns, or a scheduled check-in cue.
    #[serde(default)]
    turns: Vec<String>,
    #[serde(default)]
    checkin: Option<CheckinSpec>,
    #[serde(default)]
    expect: Expect,
    #[serde(default = "one")]
    repeat: u32,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Default, Deserialize)]
struct Setup {
    session_task: Option<String>,
    session_minutes: Option<u32>,
    companion_name: Option<String>,
    tone: Option<Tone>,
}

#[derive(Debug, Deserialize)]
struct CheckinSpec {
    reason: CheckinReason,
    action: CheckinAction,
}

#[derive(Debug, Default, Deserialize)]
struct Expect {
    /// Max sentences in the final reply.
    max_sentences: Option<usize>,
    /// At least one of these regexes must match the reply.
    #[serde(default)]
    must_match_any: Vec<String>,
    /// None of these may match.
    #[serde(default)]
    must_not_match: Vec<String>,
    /// Session phase after the scenario.
    phase: Option<SessionPhase>,
    /// Session task must contain this (case-insensitive).
    task_contains: Option<String>,
    planned_minutes: Option<u32>,
    min_subtasks: Option<usize>,
    max_subtasks: Option<usize>,
    quiet: Option<bool>,
}

fn count_sentences(text: &str) -> usize {
    let mut n = 0;
    let chars: Vec<char> = text.trim().chars().collect();
    for (i, c) in chars.iter().enumerate() {
        // Only the last mark of a run ("...", "?!") is followed by whitespace or the end.
        if matches!(c, '.' | '!' | '?') && chars.get(i + 1).is_none_or(|n| n.is_whitespace()) {
            n += 1;
        }
    }
    if n == 0 && !text.trim().is_empty() {
        1
    } else {
        n
    }
}

/// Tiny regex subset: `(?i)` prefix, `\b` word boundaries, `|` alternation, literal text.
/// Keeps the runner dependency-free; patterns in scenarios stick to this subset.
fn matches(pattern: &str, text: &str) -> bool {
    let (pat, insensitive) = match pattern.strip_prefix("(?i)") {
        Some(p) => (p, true),
        None => (pattern, false),
    };
    let hay = if insensitive {
        text.to_lowercase()
    } else {
        text.to_string()
    };
    pat.split('|').any(|alt| {
        let alt = if insensitive {
            alt.to_lowercase()
        } else {
            alt.to_string()
        };
        let left_b = alt.starts_with(r"\b");
        let core = alt.trim_start_matches(r"\b");
        let right_b = core.ends_with(r"\b");
        let core = core.trim_end_matches(r"\b").replace('\\', "");
        if core.is_empty() {
            return false;
        }
        hay.match_indices(&core).any(|(i, m)| {
            let before = hay[..i].chars().last();
            let after = hay[i + m.len()..].chars().next();
            let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            (!left_b || !is_word(before)) && (!right_b || !is_word(after))
        })
    })
}

fn provider_from_env() -> Result<(String, Arc<dyn ChatProvider>), String> {
    let kind = std::env::var("EVAL_PROVIDER").unwrap_or_else(|_| "ollama".into());
    match kind.as_str() {
        "anthropic" => {
            let key = std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| "set ANTHROPIC_API_KEY for EVAL_PROVIDER=anthropic")?;
            let model = std::env::var("ANTHROPIC_MODEL")
                .unwrap_or_else(|_| anthropic::DEFAULT_MODEL.into());
            Ok((
                format!("anthropic/{model}"),
                Arc::new(AnthropicProvider::new(key, &model, "low")),
            ))
        }
        _ => Err(format!("unknown EVAL_PROVIDER '{kind}'")),
    }
}

async fn ollama_from_env() -> Result<(String, Arc<dyn ChatProvider>), String> {
    let url = std::env::var("OLLAMA_URL")
        .map_err(|_| "set OLLAMA_URL (or EVAL_PROVIDER=anthropic + ANTHROPIC_API_KEY)")?;
    let model = match std::env::var("OLLAMA_MODEL") {
        Ok(m) => m,
        Err(_) => OllamaProvider::list_models(&url)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
            .ok_or("Ollama has no models")?,
    };
    let caps = OllamaProvider::probe_caps(&url, &model)
        .await
        .map_err(|e| e.to_string())?;
    let label = format!(
        "ollama/{model}{}",
        if caps.tools {
            ""
        } else {
            " (json tool fallback)"
        }
    );
    Ok((
        label,
        Arc::new(OllamaProvider::new(&url, &model, &[], caps)),
    ))
}

fn load_scenarios(dir: &Path, filter: Option<&str>) -> Result<Vec<Scenario>, String> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "yaml" || x == "yml"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        let text = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
        let list: Vec<Scenario> =
            serde_yaml::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
        out.extend(
            list.into_iter()
                .filter(|s| filter.is_none_or(|f| s.id.contains(f))),
        );
    }
    Ok(out)
}

async fn run_once(provider: Arc<dyn ChatProvider>, sc: &Scenario) -> Result<Vec<String>, String> {
    let store = Arc::new(Store::open_in_memory().map_err(|e| e.to_string())?);
    let core = Core::with_store(
        store,
        Arc::new(NullSink),
        Arc::new(MemorySecretStore::default()),
    )
    .await
    .map_err(|e| e.to_string())?;
    core.set_chat_provider(provider);
    let mut profile = core.profile();
    if let Some(n) = &sc.setup.companion_name {
        profile.companion.name = n.clone();
    }
    if let Some(t) = sc.setup.tone {
        profile.tone = t;
    }
    core.update_profile(profile).map_err(|e| e.to_string())?;
    if let Some(task) = &sc.setup.session_task {
        core.start_session(task, sc.setup.session_minutes, None)
            .map_err(|e| e.to_string())?;
    }

    let mut reply = String::new();
    if let Some(c) = &sc.checkin {
        reply = core
            .orchestrator()
            .checkin_line(c.reason, c.action)
            .await
            .unwrap_or_default();
    }
    for turn in &sc.turns {
        reply = core
            .send_message(turn, Via::Text)
            .await
            .map_err(|e| e.to_string())?
            .map(|m| m.text)
            .unwrap_or_default();
    }

    let mut failures = Vec::new();
    let e = &sc.expect;
    let snap = core.engine().snapshot();
    if let Some(max) = e.max_sentences {
        let n = count_sentences(&reply);
        if n > max {
            failures.push(format!("{n} sentences (max {max})"));
        }
    }
    if !e.must_match_any.is_empty() && !e.must_match_any.iter().any(|p| matches(p, &reply)) {
        failures.push(format!("reply matched none of {:?}", e.must_match_any));
    }
    for p in e
        .must_not_match
        .iter()
        .map(String::as_str)
        .chain(GLOBAL_BANNED.iter().copied())
    {
        if matches(p, &reply) {
            failures.push(format!("reply matched banned pattern {p}"));
        }
    }
    if let Some(ph) = e.phase {
        if snap.phase != ph {
            failures.push(format!("phase {:?} (expected {ph:?})", snap.phase));
        }
    }
    if let Some(sub) = &e.task_contains {
        let task = snap.task.clone().unwrap_or_default().to_lowercase();
        if !task.contains(&sub.to_lowercase()) {
            failures.push(format!("task {task:?} doesn't contain {sub:?}"));
        }
    }
    if let Some(m) = e.planned_minutes {
        if snap.planned_secs != m * 60 {
            failures.push(format!(
                "planned {} min (expected {m})",
                snap.planned_secs / 60
            ));
        }
    }
    if let Some(min) = e.min_subtasks {
        if snap.subtasks.len() < min {
            failures.push(format!("{} subtasks (min {min})", snap.subtasks.len()));
        }
    }
    if let Some(max) = e.max_subtasks {
        if snap.subtasks.len() > max {
            failures.push(format!("{} subtasks (max {max})", snap.subtasks.len()));
        }
    }
    if let Some(q) = e.quiet {
        if (snap.quiet_remaining_secs > 0) != q {
            failures.push(format!(
                "quiet = {} (expected {q})",
                snap.quiet_remaining_secs > 0
            ));
        }
    }
    if failures.is_empty() {
        Ok(vec![reply])
    } else {
        failures.push(format!("reply: {reply:?}"));
        Err(failures.join("\n      "))
    }
}

#[tokio::main]
async fn main() {
    let filter = std::env::args().nth(1);
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../evals/scenarios");
    let scenarios = match load_scenarios(&dir, filter.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error loading scenarios: {e}");
            std::process::exit(2);
        }
    };
    let provider = if std::env::var("EVAL_PROVIDER").is_ok_and(|p| p != "ollama") {
        provider_from_env()
    } else {
        ollama_from_env().await
    };
    let (label, provider) = match provider {
        Ok(p) => p,
        Err(e) => {
            eprintln!("evals: {e}");
            std::process::exit(2);
        }
    };
    println!("Running {} scenarios against {label}\n", scenarios.len());
    let (mut passed, mut failed) = (0, 0);
    for sc in &scenarios {
        let mut errors = Vec::new();
        for i in 0..sc.repeat {
            if let Err(e) = run_once(provider.clone(), sc).await {
                errors.push(format!("run {}: {e}", i + 1));
            }
        }
        if errors.is_empty() {
            passed += 1;
            println!("  PASS  {}", sc.id);
        } else {
            failed += 1;
            println!("  FAIL  {}  — {}", sc.id, sc.description);
            for e in errors {
                println!("      {e}");
            }
        }
    }
    println!("\n{passed} passed, {failed} failed");
    if failed > 0 {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_counting() {
        assert_eq!(count_sentences("One. Two!"), 2);
        assert_eq!(count_sentences("Wait... what?!"), 2);
        assert_eq!(count_sentences("no punctuation"), 1);
        assert_eq!(count_sentences(""), 0);
    }

    #[test]
    fn mini_regex() {
        assert!(matches(r"(?i)\byou should\b", "Maybe You should rest"));
        assert!(!matches(r"(?i)\blazy\b", "a lazyboy chair"));
        assert!(matches(r"988|116 123", "call 116 123"));
        assert!(!matches(r"988", "nothing"));
    }

    #[test]
    fn scenarios_parse() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../evals/scenarios");
        let s = load_scenarios(&dir, None).unwrap_or_else(|e| panic!("{e}"));
        assert!(s.len() >= 10);
        assert!(s
            .iter()
            .any(|x| x.id.starts_with("crisis") && x.repeat >= 3));
    }
}
