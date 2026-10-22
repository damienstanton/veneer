//! Cycles: a per-cycle ledger of the cache reads each lifecycle cycle cost.
//! One JSON line is appended to `.veneer/usage.jsonl` when a cycle ships, from
//! the agent session that shipped it. Deliberately a sidecar, like the graph:
//! never read by `check`, never part of the integrity-hashed state file or the
//! ship gate, and every failure here is swallowed — accounting must not be
//! able to refuse a transition.

use crate::usage::{claude_code_dir, parse, summarize, SessionUsage, TranscriptFormat};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// One shipped cycle. `turns`/`cache_read` are the cycle's own cost: the
/// session's running totals minus those at the previous record from the same
/// transcript, or the whole session when the transcript is new — so a cycle
/// that started in a fresh session reads as cheap and one that dragged a long
/// context along reads as expensive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycleRecord {
    pub cycle: u64,
    pub transcript: String,
    pub turns: u64,
    pub cache_read: u64,
    /// Context size on the cycle's last turn.
    pub end_context: u64,
    /// Session-total cache reads at ship, for differencing the next record.
    pub session_turns: u64,
    pub session_cache_read: u64,
    #[serde(default)]
    pub refs: BTreeMap<String, String>,
}

fn ledger_path(root: &Path) -> PathBuf {
    root.join(".veneer/usage.jsonl")
}

/// Every well-formed record, oldest first. A missing file is no history;
/// a malformed line is skipped, never an error.
pub fn load(root: &Path) -> Vec<CycleRecord> {
    let Ok(text) = std::fs::read_to_string(ledger_path(root)) else { return Vec::new() };
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// Pure: the record for the next cycle, given the history and the shipping
/// session's summary.
pub fn next_record(prev: &[CycleRecord], s: &SessionUsage, refs: &BTreeMap<String, String>) -> CycleRecord {
    let (base_turns, base_reads) = prev
        .iter()
        .rev()
        .find(|r| r.transcript == s.path)
        .map_or((0, 0), |r| (r.session_turns, r.session_cache_read));
    CycleRecord {
        cycle: prev.last().map_or(1, |r| r.cycle + 1),
        transcript: s.path.clone(),
        turns: s.turns.saturating_sub(base_turns),
        cache_read: s.cache_read.saturating_sub(base_reads),
        end_context: s.last_context,
        session_turns: s.turns,
        session_cache_read: s.cache_read,
        refs: refs.clone(),
    }
}

/// The session in progress: the most recently modified top-level transcript
/// in the agent's project directory (subagent transcripts live in
/// subdirectories and are not the shipping session).
fn current_transcript(root: &Path) -> Option<PathBuf> {
    let files: Vec<PathBuf> = std::fs::read_dir(claude_code_dir(root)?)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    crate::usage::latest(&files)
}

/// Append the cycle that just shipped. Best-effort: returns the record on
/// success, `None` when there is no transcript or the write fails.
pub fn record_ship(root: &Path, refs: &BTreeMap<String, String>) -> Option<CycleRecord> {
    let path = current_transcript(root)?;
    let text = std::fs::read_to_string(&path).ok()?;
    let label = path.to_string_lossy().replace('\\', "/");
    let session = summarize(&label, &parse(TranscriptFormat::ClaudeCode, &text), 0);
    let rec = next_record(&load(root), &session, refs);
    std::fs::create_dir_all(root.join(".veneer")).ok()?;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(ledger_path(root)).ok()?;
    writeln!(f, "{}", serde_json::to_string(&rec).ok()?).ok()?;
    Some(rec)
}

/// Hook for every surface that sets a phase: record exactly when a cycle
/// ships (a real transition into Ship, not the idempotent Ship → Ship).
pub fn on_transition(root: &Path, before: crate::state::Phase, after: &crate::state::State) {
    use crate::state::Phase::Ship;
    if before != Ship && after.phase == Ship {
        let _ = record_ship(root, &after.refs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::ContextKind;

    fn session(path: &str, turns: u64, reads: u64) -> SessionUsage {
        SessionUsage {
            path: path.into(), kind: ContextKind::Main, turns, input: 0, cache_write: 0,
            cache_read: reads, output: 0, mean_context: 0, peak_context: 0, last_context: 7,
            over_budget_turns: 0, cache_read_over_budget: 0,
        }
    }

    #[test]
    fn same_session_is_differenced_new_session_is_whole() {
        let none = BTreeMap::new();
        let c1 = next_record(&[], &session("a", 10, 1000), &none);
        assert_eq!((c1.cycle, c1.turns, c1.cache_read), (1, 10, 1000));
        let c2 = next_record(&[c1.clone()], &session("a", 25, 6000), &none);
        assert_eq!((c2.cycle, c2.turns, c2.cache_read), (2, 15, 5000));
        let c3 = next_record(&[c1, c2], &session("b", 4, 300), &none);
        assert_eq!((c3.cycle, c3.turns, c3.cache_read), (3, 4, 300));
    }

    #[test]
    fn malformed_ledger_lines_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".veneer")).unwrap();
        let good = serde_json::to_string(&next_record(&[], &session("a", 1, 2), &BTreeMap::new())).unwrap();
        std::fs::write(ledger_path(dir.path()), format!("junk\n{good}\n{{}}\n")).unwrap();
        assert_eq!(load(dir.path()).len(), 1);
    }
}
