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
            ".veneer/state.toon",
            None,
            &format!("invalid transition {} → {}", current.name(), requested.name()),
            Some("lifecycle is plan → implement → verify → ship (verify may return to implement; ship returns to plan)"),
        ))
    }
}

fn state_path(root: &Path) -> std::path::PathBuf {
    root.join(".veneer/state.toon")
}

/// Pre-TOON state files. Read for backward compatibility and superseded on the
/// next write (see `store`), so a project written by an older veneer keeps
/// loading until its next state mutation.
fn legacy_path(root: &Path) -> std::path::PathBuf {
    root.join(".veneer/state.json")
}

/// The format-independent integrity witness: the FNV-1a hash is taken over the
/// JSON serialization of the logical `State`, identical whether the on-disk
/// file is legacy JSON or TOON — so the witness survives migration.
fn canonical_bytes(s: &State) -> Vec<u8> {
    serde_json::to_vec(s).expect("state serialization is infallible")
}

/// Current on-disk state wire version. Version 1 armors refs via `wire`; a
/// missing/0 version is a pre-1.0 file whose refs are raw.
const STATE_VERSION: u32 = 1;

/// The on-disk document: the logical state plus its embedded integrity hash.
/// `skip_serializing_if` keeps absent-equivalent fields out of the encoding so
/// TOON — which renders an empty value for null/empty — round-trips exactly.
#[derive(Serialize, Deserialize)]
struct OnDisk {
    #[serde(default)]
    version: u32,
    #[serde(default = "default_phase")]
    phase: Phase,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    refs: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "clean_check_repr")]
    last_clean_check: Option<u64>,
    hash: String,
}

/// On-disk representation of `last_clean_check`: serialized as a decimal string
/// so TOON stores it as a quoted scalar that round-trips exactly even past
/// `i64::MAX` (the witness is a full-width FNV-1a u64, frequently above it).
/// Deserialization also accepts a JSON number, so legacy state files load.
mod clean_check_repr {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(n) => s.serialize_str(&n.to_string()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
        match Option::<serde_json::Value>::deserialize(d)? {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::String(s)) if s.is_empty() => Ok(None),
            Some(serde_json::Value::String(s)) => {
                s.parse::<u64>().map(Some).map_err(|_| D::Error::custom("invalid last_clean_check"))
            }
            Some(serde_json::Value::Number(n)) => {
                n.as_u64().map(Some).ok_or_else(|| D::Error::custom("last_clean_check out of range"))
            }
            Some(_) => Err(D::Error::custom("last_clean_check must be a number or string")),
        }
    }
}

/// Load state; absent file is the Plan default; corruption (bad encoding or
/// hash mismatch) is a Protocol finding, never a crash. Prefers the TOON file
/// and falls back to a legacy JSON file (decoded identically).
///
/// The legacy fallback triggers only when the TOON file is genuinely absent
/// (`NotFound`); any other read error on it (permission denied, invalid UTF-8,
/// etc.) is a real failure on the authoritative file and must not be masked by
/// silently reading a possibly-stale legacy file. Findings report whichever
/// path was actually the source, never a hardcoded one.
pub fn load(root: &Path) -> Result<State, Finding> {
    use std::io::ErrorKind;
    let io_finding = |path: &str, e: &std::io::Error| {
        Finding::error(
            Law::Protocol,
            path,
            None,
            &format!("cannot read state file: {e}"),
            Some("run `veneer state reset` to start a fresh cycle"),
        )
    };
    let toon = state_path(root);
    let legacy = legacy_path(root);
    let (raw, is_toon, source) = match std::fs::read_to_string(&toon) {
        Ok(r) => (r, true, ".veneer/state.toon"),
        Err(e) if e.kind() == ErrorKind::NotFound => match std::fs::read_to_string(&legacy) {
            Ok(r) => (r, false, ".veneer/state.json"),
            Err(e2) if e2.kind() == ErrorKind::NotFound => return Ok(State::default()),
            Err(e2) => return Err(io_finding(".veneer/state.json", &e2)),
        },
        Err(e) => return Err(io_finding(".veneer/state.toon", &e)),
    };
    let corrupt = |msg: &str| {
        Finding::error(
            Law::Protocol,
            source,
            None,
            msg,
            Some("run `veneer state reset` to start a fresh cycle"),
        )
    };
    let od: OnDisk = if is_toon {
        toon_rust::from_str(&raw).map_err(|_| corrupt("state file is not valid TOON"))?
    } else {
        serde_json::from_str(&raw).map_err(|_| corrupt("state file is not valid JSON"))?
    };
    // Refs are armored only from version 1 onward. Legacy JSON, and a
    // pre-1.0 TOON file (version absent/0, written by an older veneer before
    // the wire was versioned), wrote refs raw — decoding those unconditionally
    // would corrupt any ref that happens to contain a valid `%XX` sequence or
    // a bare `%`, so only a version-1 TOON document is decoded.
    let refs = if is_toon && od.version == STATE_VERSION {
        od.refs.into_iter().map(|(k, v)| (crate::wire::decode(&k), crate::wire::decode(&v))).collect()
    } else {
        od.refs
    };
    let state = State { phase: od.phase, refs, last_clean_check: od.last_clean_check };
    let expect = format!("fnv:{:016x}", fnv64(&canonical_bytes(&state)));
    if od.hash != expect {
        return Err(corrupt("state file content hash mismatch"));
    }
    Ok(state)
}

/// Store state as TOON with its content hash embedded. Identical state ⇒
/// identical bytes ⇒ replayed writes converge.
///
/// Crash-atomic: written to a temp file then renamed, so a partial write never
/// replaces good state. Completes migration by removing any legacy JSON file
/// once the TOON file is authoritative.
pub fn store(root: &Path, s: &State) -> std::io::Result<()> {
    std::fs::create_dir_all(root.join(".veneer"))?;
    let od = OnDisk {
        version: STATE_VERSION,
        phase: s.phase,
        // Refs are the only free-text field in the state; armor them for the
        // TOON wire (see wire.rs). The integrity hash is over the *logical*
        // state, so armoring the wire never changes the witness.
        refs: s.refs.iter().map(|(k, v)| (crate::wire::encode(k), crate::wire::encode(v))).collect(),
        last_clean_check: s.last_clean_check,
        hash: format!("fnv:{:016x}", fnv64(&canonical_bytes(s))),
    };
    let body = toon_rust::to_string(&od).expect("state serialization is infallible") + "\n";
    let tmp = state_path(root).with_extension("toon.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, state_path(root))?;
    // Migration: the TOON file is now authoritative; drop a stale legacy file.
    let legacy = legacy_path(root);
    if legacy.exists() {
        let _ = std::fs::remove_file(&legacy);
    }
    Ok(())
}

/// The agent-facing state view: phase and refs only. Gate internals
/// (`last_clean_check`) live in the file, not in responses.
pub fn public_json(s: &State) -> String {
    serde_json::json!({ "phase": s.phase.name(), "refs": s.refs }).to_string()
}

/// The on-demand status report: the full state decoded to human-readable,
/// parseable JSON. Unlike `public_json` it includes the gate witness
/// (`last_clean_check`); the integrity hash is omitted (it is recomputed on
/// load, not part of the logical state).
pub fn full_json(s: &State) -> String {
    serde_json::to_string_pretty(s).expect("state serialization is infallible")
}

/// Record that `veneer check` ran clean, storing the clean-check witness
/// (the `clean_hash` over tree + config) for the ship gate.
pub fn record_clean_check(root: &Path, hash: u64) -> Result<(), Finding> {
    let mut s = load(root)?;
    s.last_clean_check = Some(hash);
    store(root, &s).map_err(|e| {
        Finding::error(Law::Protocol, ".veneer/state.toon", None, &format!("cannot write state: {e}"), None)
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
                    ".veneer/state.toon",
                    None,
                    "ship gate: no clean check recorded",
                    Some("run `veneer check` until clean, then ship"),
                ))
            }
            Some(h) if h != current => {
                return Err(Finding::error(
                    Law::Protocol,
                    ".veneer/state.toon",
                    None,
                    "ship gate: last clean check is stale (tree or config changed since)",
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
        Finding::error(Law::Protocol, ".veneer/state.toon", None, &format!("cannot write state: {e}"), None)
    })?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct W {
        #[serde(with = "clean_check_repr")]
        v: Option<u64>,
    }

    #[test]
    fn clean_check_accepts_string_number_null_and_empty() {
        assert_eq!(serde_json::from_str::<W>(r#"{"v":"18446744073709551615"}"#).unwrap().v, Some(u64::MAX));
        assert_eq!(serde_json::from_str::<W>(r#"{"v":42}"#).unwrap().v, Some(42));
        assert_eq!(serde_json::from_str::<W>(r#"{"v":null}"#).unwrap().v, None);
        assert_eq!(serde_json::from_str::<W>(r#"{"v":""}"#).unwrap().v, None);
    }

    #[test]
    fn clean_check_rejects_negative_bool_and_garbage() {
        assert!(serde_json::from_str::<W>(r#"{"v":-1}"#).is_err());
        assert!(serde_json::from_str::<W>(r#"{"v":true}"#).is_err());
        assert!(serde_json::from_str::<W>(r#"{"v":"abc"}"#).is_err());
    }

    #[test]
    fn clean_check_serializes_full_width_u64_as_decimal_string() {
        assert_eq!(
            serde_json::to_string(&W { v: Some(u64::MAX) }).unwrap(),
            r#"{"v":"18446744073709551615"}"#
        );
    }
}
