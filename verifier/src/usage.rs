//! Usage: per-session token accounting read from an agent's own transcript.
//! The metric is cache reads — every turn re-reads the whole context, so a
//! session's cache-read bill is roughly its context size times its turns.
//! Deterministic for a fixed transcript; orthogonal to the laws and the ship
//! gate (nothing here is read by `check` or `set ship`). Errors are data.

use crate::laws::{Finding, Law};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const BUDGET_FIX: &str =
    "compact now, or finish the cycle and start the next one in a fresh session (`veneer state get` resumes)";

/// The `[usage]` TOML section, surfaced on `Config`. `budget_tokens` is the
/// context size past which a session is told to compact or hand off.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UsageConfig {
    #[serde(default = "default_budget")]
    pub budget_tokens: u64,
}

fn default_budget() -> u64 { 150_000 }

impl Default for UsageConfig {
    fn default() -> UsageConfig { UsageConfig { budget_tokens: default_budget() } }
}

/// Transcript dialects veneer can read. A sum type so each new agent is one
/// variant and one parser, with exhaustive dispatch in `parse`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptFormat {
    /// Claude Code session JSONL (`~/.claude/projects/<slug>/<session>.jsonl`).
    ClaudeCode,
}

/// Which context window a transcript's stats describe: the main chain, or a
/// subagent's own window (a transcript with only sidechain turns).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    Main,
    Subagent,
}

/// One API response's token counts. `context` is the full prompt it was
/// billed for: fresh input + cache writes + cache reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Turn {
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    pub sidechain: bool,
}

impl Turn {
    pub fn context(&self) -> u64 { self.input + self.cache_write + self.cache_read }
}

/// One transcript, summarized. Context statistics (`mean`/`peak`/`last`,
/// `over_budget_turns`) cover the window named by `kind`; token totals cover
/// every turn in the file. `cache_read_over_budget` sums each turn's cache
/// read beyond the budget — a lower bound on what compacting at the budget
/// would have saved, and the number to drive toward zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionUsage {
    pub path: String,
    pub kind: ContextKind,
    pub turns: u64,
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    pub mean_context: u64,
    pub peak_context: u64,
    pub last_context: u64,
    pub over_budget_turns: u64,
    pub cache_read_over_budget: u64,
}

/// Totals across every session in a report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UsageTotal {
    pub sessions: u64,
    pub turns: u64,
    pub cache_read: u64,
    pub cache_read_over_budget: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageReport {
    pub budget_tokens: u64,
    pub sessions: Vec<SessionUsage>,
    pub total: UsageTotal,
    pub findings: Vec<Finding>,
}

#[derive(Deserialize)]
struct CcLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    #[serde(rename = "isSidechain", default)]
    is_sidechain: bool,
    message: Option<CcMessage>,
}

#[derive(Deserialize)]
struct CcMessage {
    id: Option<String>,
    usage: Option<CcUsage>,
}

#[derive(Deserialize)]
struct CcUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

/// Claude Code writes one line per content block, each repeating the whole
/// response's usage, so a response is counted once — keyed by message id
/// (falling back to request id). Counting lines instead roughly doubles the
/// totals. Unparseable lines and non-assistant entries are skipped.
fn parse_claude_code(text: &str) -> Vec<Turn> {
    let mut seen = BTreeSet::new();
    let mut turns = Vec::new();
    for line in text.lines() {
        let Ok(l) = serde_json::from_str::<CcLine>(line) else { continue };
        if l.kind.as_deref() != Some("assistant") {
            continue;
        }
        let Some(msg) = l.message else { continue };
        let Some(u) = msg.usage else { continue };
        if let Some(key) = msg.id.or(l.request_id) {
            if !seen.insert(key) {
                continue;
            }
        }
        turns.push(Turn {
            input: u.input_tokens,
            cache_write: u.cache_creation_input_tokens,
            cache_read: u.cache_read_input_tokens,
            output: u.output_tokens,
            sidechain: l.is_sidechain,
        });
    }
    turns
}

pub fn parse(format: TranscriptFormat, text: &str) -> Vec<Turn> {
    match format {
        TranscriptFormat::ClaudeCode => parse_claude_code(text),
    }
}

/// Summarize one transcript's turns against `budget`.
pub fn summarize(path: &str, turns: &[Turn], budget: u64) -> SessionUsage {
    let kind = if turns.iter().any(|t| !t.sidechain) { ContextKind::Main } else { ContextKind::Subagent };
    let window: Vec<&Turn> =
        turns.iter().filter(|t| t.sidechain == (kind == ContextKind::Subagent)).collect();
    let sum = |f: fn(&Turn) -> u64| turns.iter().map(f).sum::<u64>();
    let contexts: Vec<u64> = window.iter().map(|t| t.context()).collect();
    SessionUsage {
        path: path.to_string(),
        kind,
        turns: turns.len() as u64,
        input: sum(|t| t.input),
        cache_write: sum(|t| t.cache_write),
        cache_read: sum(|t| t.cache_read),
        output: sum(|t| t.output),
        mean_context: if contexts.is_empty() { 0 } else { contexts.iter().sum::<u64>() / contexts.len() as u64 },
        peak_context: contexts.iter().copied().max().unwrap_or(0),
        last_context: contexts.last().copied().unwrap_or(0),
        over_budget_turns: contexts.iter().filter(|&&c| c > budget).count() as u64,
        cache_read_over_budget: window.iter().map(|t| t.cache_read.saturating_sub(budget)).sum(),
    }
}

fn kilo(n: u64) -> String { format!("{}K", (n + 500) / 1000) }

/// A session whose context ever passed the budget yields one warning — a
/// nudge, never an error: the budget is about the agent's window, not the
/// tree, so it can never block a transition.
pub fn budget_finding(s: &SessionUsage, budget: u64) -> Option<Finding> {
    if s.peak_context <= budget {
        return None;
    }
    Some(Finding::warning(
        Law::ContextBudget,
        &s.path,
        None,
        &format!(
            "context peaked at {} tokens (budget {}); {} of {} turns ran over, re-reading at least {} cache tokens past the budget",
            kilo(s.peak_context),
            kilo(budget),
            s.over_budget_turns,
            s.turns,
            kilo(s.cache_read_over_budget)
        ),
        Some(BUDGET_FIX),
    ))
}

/// Every `.jsonl` file under each given path (a file, or a directory walked
/// recursively), sorted and deduplicated — deterministic report order.
pub fn transcripts(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = BTreeSet::new();
    for p in paths {
        let mut stack = vec![p.clone()];
        while let Some(cur) = stack.pop() {
            if cur.is_dir() {
                let Ok(entries) = std::fs::read_dir(&cur) else { continue };
                stack.extend(entries.flatten().map(|e| e.path()));
            } else if cur.extension().is_some_and(|e| e == "jsonl") {
                out.insert(cur);
            }
        }
    }
    out.into_iter().collect()
}

/// The session transcripts directly in an agent's project directory, sorted.
/// Claude Code keeps subagent transcripts in per-session subdirectories, so
/// the top level holds only sessions — the candidates for "the session I am
/// in".
pub fn top_level(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    out.sort();
    out
}

/// The most recently modified transcript — "the session I am in", for a
/// caller that cannot name its own transcript.
pub fn latest(files: &[PathBuf]) -> Option<PathBuf> {
    files
        .iter()
        .filter_map(|f| Some((std::fs::metadata(f).ok()?.modified().ok()?, f)))
        .max()
        .map(|(_, f)| f.clone())
}

/// Claude Code's transcript directory for one working directory: every
/// non-alphanumeric character of the absolute path becomes `-`, under
/// `$CLAUDE_CONFIG_DIR` (default `~/.claude`) `/projects/`.
pub fn claude_code_slug_dir(cwd: &Path) -> Option<PathBuf> {
    let base = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".claude"),
    };
    let slug: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    Some(base.join("projects").join(slug))
}

/// The transcript directory for a project root. The agent is often launched
/// in a parent of the project (a workspace holding several clones), so the
/// nearest of `root` and its ancestors that has a transcript directory wins;
/// when none has one, `root`'s own (absent) directory — an empty report.
pub fn claude_code_dir(root: &Path) -> Option<PathBuf> {
    root.ancestors()
        .filter_map(claude_code_slug_dir)
        .find(|d| d.is_dir())
        .or_else(|| claude_code_slug_dir(root))
}

/// Build a report over transcript files. An unreadable file is a Protocol
/// finding in the report, never a failure of the whole report.
pub fn report(files: &[PathBuf], budget: u64) -> UsageReport {
    let mut sessions = Vec::new();
    let mut findings = Vec::new();
    for f in files {
        let label = f.to_string_lossy().replace('\\', "/");
        match std::fs::read_to_string(f) {
            Ok(text) => {
                let s = summarize(&label, &parse(TranscriptFormat::ClaudeCode, &text), budget);
                findings.extend(budget_finding(&s, budget));
                sessions.push(s);
            }
            Err(e) => findings.push(Finding::error(
                Law::Protocol,
                &label,
                None,
                &format!("cannot read transcript: {e}"),
                None,
            )),
        }
    }
    let total = UsageTotal {
        sessions: sessions.len() as u64,
        turns: sessions.iter().map(|s| s.turns).sum(),
        cache_read: sessions.iter().map(|s| s.cache_read).sum(),
        cache_read_over_budget: sessions.iter().map(|s| s.cache_read_over_budget).sum(),
    };
    UsageReport { budget_tokens: budget, sessions, total, findings }
}

/// The report as JSON; `compact` strips `suggested_fix`, like every other
/// compact surface.
pub fn report_json(r: &UsageReport, compact: bool) -> String {
    let mut v = serde_json::to_value(r).expect("report serialization is infallible");
    if compact {
        if let Some(fs) = v.get_mut("findings").and_then(|f| f.as_array_mut()) {
            for f in fs.iter_mut().filter_map(|f| f.as_object_mut()) {
                f.remove("suggested_fix");
            }
        }
    }
    v.to_string()
}

#[derive(Deserialize)]
struct HookInput {
    transcript_path: Option<String>,
    hook_event_name: Option<String>,
}

/// A Claude Code hook: given the hook's stdin JSON, the hook stdout to emit —
/// `None` (emit nothing) while the session's current context is within the
/// budget, or when anything is missing or unreadable. A hook must never
/// stand between the user and their prompt, so every failure degrades to
/// silence rather than an error.
pub fn hook_response(stdin: &str, budget: u64) -> Option<String> {
    let input: HookInput = serde_json::from_str(stdin).ok()?;
    let text = std::fs::read_to_string(input.transcript_path?).ok()?;
    let s = summarize("", &parse(TranscriptFormat::ClaudeCode, &text), budget);
    if s.last_context <= budget {
        return None;
    }
    let note = format!(
        "veneer: this session's context is {} tokens, over its {} budget, and every turn re-reads all of it. \
         At the next phase boundary, compact, or finish the cycle and start the next one in a fresh session \
         (`veneer state get` resumes). Push further exploration into subagents.",
        kilo(s.last_context),
        kilo(budget)
    );
    let event = input.hook_event_name.unwrap_or_else(|| "UserPromptSubmit".into());
    Some(serde_json::json!({ "hookSpecificOutput": { "hookEventName": event, "additionalContext": note } }).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, read: u64, sidechain: bool) -> String {
        format!(
            r#"{{"type":"assistant","isSidechain":{sidechain},"requestId":"r{id}","message":{{"id":"m{id}","usage":{{"input_tokens":2,"cache_creation_input_tokens":8,"cache_read_input_tokens":{read},"output_tokens":5}}}}}}"#
        )
    }

    #[test]
    fn repeated_content_block_lines_count_once() {
        let t = [line("1", 100, false), line("1", 100, false), line("2", 200, false)].join("\n");
        let turns = parse(TranscriptFormat::ClaudeCode, &t);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].context(), 210);
    }

    #[test]
    fn junk_and_non_assistant_lines_are_skipped() {
        let t = format!("not json\n{{\"type\":\"user\"}}\n{}\n", line("1", 10, false));
        assert_eq!(parse(TranscriptFormat::ClaudeCode, &t).len(), 1);
    }

    #[test]
    fn main_chain_context_ignores_sidechain_turns() {
        let t = [line("1", 100, false), line("2", 900, true)].join("\n");
        let s = summarize("x", &parse(TranscriptFormat::ClaudeCode, &t), 1000);
        assert_eq!((s.kind, s.peak_context, s.turns, s.cache_read), (ContextKind::Main, 110, 2, 1000));
        let sub = summarize("y", &parse(TranscriptFormat::ClaudeCode, &line("3", 50, true)), 1000);
        assert_eq!((sub.kind, sub.last_context), (ContextKind::Subagent, 60));
    }

    #[test]
    fn over_budget_reads_sum_the_excess_only() {
        let t = [line("1", 100, false), line("2", 250, false), line("3", 300, false)].join("\n");
        let s = summarize("x", &parse(TranscriptFormat::ClaudeCode, &t), 200);
        assert_eq!((s.over_budget_turns, s.cache_read_over_budget), (2, 150));
        assert_eq!(s.mean_context, (110 + 260 + 310) / 3);
        let f = budget_finding(&s, 200).expect("over budget warns");
        assert_eq!((f.law, f.severity), (Law::ContextBudget, crate::laws::Severity::Warning));
        assert!(budget_finding(&s, 1000).is_none());
    }

    #[test]
    fn empty_transcript_summarizes_to_zeros() {
        let s = summarize("x", &[], 10);
        assert_eq!((s.turns, s.peak_context, s.mean_context), (0, 0, 0));
        assert!(budget_finding(&s, 10).is_none());
    }

    #[test]
    fn claude_code_slug_replaces_every_non_alphanumeric() {
        let d = claude_code_slug_dir(Path::new("/home/user/my.repo")).unwrap();
        assert!(d.ends_with("projects/-home-user-my-repo"));
    }

    #[test]
    fn hook_is_silent_on_bad_input() {
        assert_eq!(hook_response("", 10), None);
        assert_eq!(hook_response("{}", 10), None);
        assert_eq!(hook_response(r#"{"transcript_path":"/nonexistent/x.jsonl"}"#, 10), None);
    }
}
