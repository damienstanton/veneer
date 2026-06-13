//! Lifecycle state machine (Plan → Implement → Verify → Ship) with a
//! value-semantic, content-hashed state file. Re-runs converge: setting the
//! current phase is a no-op success; replayed writes produce identical bytes.

use crate::laws::{clean_hash, fnv64, Finding, Law};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Plan,
    Implement,
    Verify,
    Ship,
}

impl Phase {
    pub fn parse(s: &str) -> Option<Phase> {
        match s {
            "plan" => Some(Phase::Plan),
            "implement" => Some(Phase::Implement),
            "verify" => Some(Phase::Verify),
            "ship" => Some(Phase::Ship),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Phase::Plan => "plan",
            Phase::Implement => "implement",
            Phase::Verify => "verify",
            Phase::Ship => "ship",
        }
    }
}

impl Default for Phase {
    fn default() -> Phase {
        Phase::Plan
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default = "default_phase")]
    pub phase: Phase,
    #[serde(default)]
    pub refs: BTreeMap<String, String>,
    #[serde(default)]
    pub last_clean_check: Option<u64>,
}

fn default_phase() -> Phase {
    Phase::Plan
}

impl Default for State {
    fn default() -> State {
        State { phase: Phase::Plan, refs: BTreeMap::new(), last_clean_check: None }
    }
}

/// Total transition judgement: every (current, requested) pair yields a
/// value. Same-phase requests are valid no-ops (idempotency).
pub fn transition(current: Phase, requested: Phase) -> Result<(), Finding> {
    use Phase::*;
    let ok = current == requested
        || matches!(
            (current, requested),
            (Plan, Implement) | (Implement, Verify) | (Verify, Implement) | (Verify, Ship) | (Ship, Plan)
        );
    if ok {
        Ok(())
    } else {
        Err(Finding::error(
            Law::Protocol,
            ".veneer/state.json",
            None,
            &format!("invalid transition {} → {}", current.name(), requested.name()),
            Some("lifecycle is plan → implement → verify → ship (verify may return to implement; ship returns to plan)"),
        ))
    }
}

fn state_path(root: &Path) -> std::path::PathBuf {
    root.join(".veneer/state.json")
}

fn canonical_bytes(s: &State) -> Vec<u8> {
    serde_json::to_vec(s).expect("state serialization is infallible")
}

/// Load state; absent file is the Plan default; corruption (bad JSON or hash
/// mismatch) is a Protocol finding, never a crash.
pub fn load(root: &Path) -> Result<State, Finding> {
    let p = state_path(root);
    let Ok(raw) = std::fs::read_to_string(&p) else {
        return Ok(State::default());
    };
    let corrupt = |msg: &str| {
        Finding::error(
            Law::Protocol,
            ".veneer/state.json",
            None,
            msg,
            Some("run `veneer state reset` to start a fresh cycle"),
        )
    };
    let v: serde_json::Value =
        serde_json::from_str(&raw).map_err(|_| corrupt("state file is not valid JSON"))?;
    let stored_hash = v.get("hash").and_then(|h| h.as_str()).unwrap_or("").to_string();
    let mut obj = v.clone();
    obj.as_object_mut().map(|m| m.remove("hash"));
    let state: State =
        serde_json::from_value(obj).map_err(|_| corrupt("state file has unknown shape"))?;
    let expect = format!("fnv:{:016x}", fnv64(&canonical_bytes(&state)));
    if stored_hash != expect {
        return Err(corrupt("state file content hash mismatch"));
    }
    Ok(state)
}

/// Store state with its content hash embedded. Identical state ⇒ identical
/// bytes ⇒ replayed writes converge.
///
/// Crash-atomic: written to a temp file then renamed, so a partial write never
/// replaces good state.
pub fn store(root: &Path, s: &State) -> std::io::Result<()> {
    std::fs::create_dir_all(root.join(".veneer"))?;
    let mut v = serde_json::to_value(s).expect("state serialization is infallible");
    let hash = format!("fnv:{:016x}", fnv64(&canonical_bytes(s)));
    v.as_object_mut().unwrap().insert("hash".into(), serde_json::Value::String(hash));
    let tmp = state_path(root).with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&v).unwrap() + "\n")?;
    std::fs::rename(&tmp, state_path(root))
}

/// The agent-facing state view: phase and refs only. Gate internals
/// (`last_clean_check`) live in the file, not in responses.
pub fn public_json(s: &State) -> String {
    serde_json::json!({ "phase": s.phase.name(), "refs": s.refs }).to_string()
}

/// Record that `veneer check` ran clean against the tree with this hash.
pub fn record_clean_check(root: &Path, hash: u64) -> Result<(), Finding> {
    let mut s = load(root)?;
    s.last_clean_check = Some(hash);
    store(root, &s).map_err(|e| {
        Finding::error(Law::Protocol, ".veneer/state.json", None, &format!("cannot write state: {e}"), None)
    })
}

/// The phase-setting judgement: validates the transition, enforces the ship
/// gate, merges refs, persists. Returns the new state.
pub fn set_phase(root: &Path, requested: Phase, refs: &[(String, String)]) -> Result<State, Finding> {
    let mut s = load(root)?;
    transition(s.phase, requested)?;
    if s.phase == Phase::Ship && requested == Phase::Plan {
        // A new cycle requires a fresh clean check; stale validity-by-hash-match
        // across cycles is not accepted.
        s.last_clean_check = None;
    }
    // Ship→Ship is an idempotent no-op and intentionally not re-gated.
    if requested == Phase::Ship && s.phase != Phase::Ship {
        let current = clean_hash(root);
        match s.last_clean_check {
            None => {
                return Err(Finding::error(
                    Law::Protocol,
                    ".veneer/state.json",
                    None,
                    "ship gate: no clean check recorded",
                    Some("run `veneer check` until clean, then ship"),
                ))
            }
            Some(h) if h != current => {
                return Err(Finding::error(
                    Law::Protocol,
                    ".veneer/state.json",
                    None,
                    "ship gate: last clean check is stale (tree changed since)",
                    Some("re-run `veneer check`, then ship"),
                ))
            }
            Some(_) => {}
        }
    }
    s.phase = requested;
    for (k, v) in refs {
        s.refs.insert(k.clone(), v.clone());
    }
    store(root, &s).map_err(|e| {
        Finding::error(Law::Protocol, ".veneer/state.json", None, &format!("cannot write state: {e}"), None)
    })?;
    Ok(s)
}
