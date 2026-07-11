//! The upgrade contract, end to end: a `.veneer/` directory written by an
//! older veneer (legacy JSON state, stale/foreign graph format) must be fully
//! usable by this binary with zero manual steps.

use std::path::Path;

fn veneer(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_veneer"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("binary runs")
}

/// A legacy state.json exactly as an older veneer wrote it: logical state
/// hashed over its canonical JSON form (format-independent witness).
fn write_legacy_state(root: &Path, phase: &str, issue: &str) {
    let mut refs = std::collections::BTreeMap::new();
    refs.insert("issue".to_string(), issue.to_string());
    let logical = veneer::state::State {
        phase: veneer::state::Phase::parse(phase).unwrap(),
        refs: refs.clone(),
        last_clean_check: None,
    };
    let hash = format!("fnv:{:016x}", veneer::laws::fnv64(&serde_json::to_vec(&logical).unwrap()));
    let od = serde_json::json!({ "phase": phase, "refs": refs, "hash": hash });
    std::fs::create_dir_all(root.join(".veneer")).unwrap();
    std::fs::write(root.join(".veneer/state.json"), serde_json::to_string(&od).unwrap()).unwrap();
}

#[test]
fn old_veneer_project_upgrades_with_zero_manual_steps() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("m.rs"), "pub fn f() {}\n").unwrap();
    write_legacy_state(root, "plan", "7");
    // A graph in a foreign/old format: pre-1.0 wire shape (no version field).
    std::fs::write(root.join(".veneer/graph.toon"), "entries:\n  m.rs: whatever\nhash: fnv:0000000000000000\n").unwrap();

    // 1. State reads through the legacy file.
    let out = veneer(root, &["state", "get"]);
    assert!(out.status.success());
    let got: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(got["phase"], "plan");
    assert_eq!(got["refs"]["issue"], "7");

    // 2. The unreadable-format graph self-heals at query time: no finding.
    let out = veneer(root, &["graph", "query", "m.rs"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let got: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(got["entry"], serde_json::Value::Null);
    assert_eq!(got["stale"], true);

    // 3. A clean check runs, records the witness, and regenerates the graph.
    let out = veneer(root, &["check", "--compact"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "[]");
    let raw = std::fs::read_to_string(root.join(".veneer/graph.toon")).unwrap();
    assert!(raw.contains("version:"), "regenerated graph carries the format version");

    // 4. The first state write migrates: TOON appears, legacy JSON removed.
    let out = veneer(root, &["state", "set", "implement"]);
    assert!(out.status.success());
    assert!(root.join(".veneer/state.toon").exists());
    assert!(!root.join(".veneer/state.json").exists());

    // 5. The migrated lifecycle proceeds to the gate.
    assert!(veneer(root, &["state", "set", "verify"]).status.success());
    assert!(veneer(root, &["state", "set", "ship"]).status.success());
}
