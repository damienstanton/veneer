use veneer::laws::Law;
use veneer::state::{load, record_clean_check, set_phase, store, transition, Phase, State};


#[test]
fn transition_table_is_exactly_the_lifecycle() {
    use Phase::*;
    let valid = [
        (Plan, Implement),
        (Implement, Verify),
        (Verify, Implement),
        (Verify, Ship),
        (Ship, Plan),
        (Plan, Plan), // no-op re-entry is always valid (idempotency)
        (Verify, Verify),
    ];
    for (a, b) in valid {
        assert!(transition(a, b).is_ok(), "{a:?}→{b:?} should be valid");
    }
    let invalid = [(Plan, Verify), (Plan, Ship), (Implement, Ship), (Ship, Verify), (Implement, Plan)];
    for (a, b) in invalid {
        let f = transition(a, b).unwrap_err();
        assert_eq!(f.law, Law::Protocol);
        assert!(f.message.contains("invalid transition"));
    }
}

#[test]
fn state_roundtrips_with_content_hash() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = State::default();
    s.phase = Phase::Implement;
    s.refs.insert("issue".into(), "42".into());
    store(dir.path(), &s).unwrap();
    let loaded = load(dir.path()).unwrap();
    assert_eq!(loaded.phase, Phase::Implement);
    assert_eq!(loaded.refs["issue"], "42");
}

#[test]
fn missing_state_defaults_to_plan() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load(dir.path()).unwrap().phase, Phase::Plan);
}

#[test]
fn corrupt_state_is_a_protocol_finding() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".veneer")).unwrap();
    std::fs::write(dir.path().join(".veneer/state.json"), "{\"phase\":\"plan\",\"hash\":\"tampered\"}").unwrap();
    let f = load(dir.path()).unwrap_err();
    assert_eq!(f.law, Law::Protocol);
}

#[test]
fn ship_gate_requires_fresh_clean_check() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("code.rs"), "fn main() {}\n").unwrap();
    // Walk the lifecycle to Verify.
    set_phase(dir.path(), Phase::Implement, &[]).unwrap();
    set_phase(dir.path(), Phase::Verify, &[]).unwrap();
    // No clean check recorded → gate refuses.
    let f = set_phase(dir.path(), Phase::Ship, &[]).unwrap_err();
    assert!(f.message.contains("clean check"));
    // Record a clean check of the current tree → gate opens.
    record_clean_check(dir.path(), veneer::laws::clean_hash(dir.path())).unwrap();
    set_phase(dir.path(), Phase::Ship, &[("pr".into(), "7".into())]).unwrap();
    assert_eq!(load(dir.path()).unwrap().phase, Phase::Ship);
    assert_eq!(load(dir.path()).unwrap().refs["pr"], "7");
}

#[test]
fn ship_gate_detects_stale_check() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("code.rs"), "fn main() {}\n").unwrap();
    set_phase(dir.path(), Phase::Implement, &[]).unwrap();
    set_phase(dir.path(), Phase::Verify, &[]).unwrap();
    record_clean_check(dir.path(), veneer::laws::clean_hash(dir.path())).unwrap();
    // Tree changes after the clean check → stale → gate refuses.
    std::fs::write(dir.path().join("code.rs"), "fn main() { changed(); }\n").unwrap();
    let f = set_phase(dir.path(), Phase::Ship, &[]).unwrap_err();
    assert!(f.message.contains("stale"));
}

#[test]
fn set_phase_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    set_phase(dir.path(), Phase::Implement, &[]).unwrap();
    set_phase(dir.path(), Phase::Implement, &[]).unwrap(); // no-op success
    assert_eq!(load(dir.path()).unwrap().phase, Phase::Implement);
}

#[test]
fn full_state_roundtrips_including_clean_check() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = State::default();
    s.phase = Phase::Verify;
    s.refs.insert("issue".into(), "9".into());
    s.last_clean_check = Some(0xdead_beef);
    store(dir.path(), &s).unwrap();
    assert_eq!(load(dir.path()).unwrap(), s);
}

#[test]
fn new_cycle_requires_fresh_clean_check() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("code.rs"), "fn main() {}\n").unwrap();
    set_phase(dir.path(), Phase::Implement, &[]).unwrap();
    set_phase(dir.path(), Phase::Verify, &[]).unwrap();
    record_clean_check(dir.path(), veneer::laws::clean_hash(dir.path())).unwrap();
    set_phase(dir.path(), Phase::Ship, &[]).unwrap();
    // New cycle; tree is byte-identical, but the old check must not count.
    set_phase(dir.path(), Phase::Plan, &[]).unwrap();
    set_phase(dir.path(), Phase::Implement, &[]).unwrap();
    set_phase(dir.path(), Phase::Verify, &[]).unwrap();
    let f = set_phase(dir.path(), Phase::Ship, &[]).unwrap_err();
    assert!(f.message.contains("clean check"));
}

#[test]
fn truncated_state_file_is_a_protocol_finding() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".veneer")).unwrap();
    std::fs::write(dir.path().join(".veneer/state.json"), "{\"phase\":\"pl").unwrap();
    assert_eq!(load(dir.path()).unwrap_err().law, Law::Protocol);
}

#[test]
fn adversarial_state_shapes_are_findings_not_panics() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".veneer")).unwrap();
    for bad in ["[]", "{\"hash\": 42}", "{\"phase\": 99, \"hash\": \"x\"}", "null"] {
        std::fs::write(dir.path().join(".veneer/state.json"), bad).unwrap();
        assert_eq!(load(dir.path()).unwrap_err().law, Law::Protocol, "input: {bad}");
    }
}

#[test]
fn config_change_stales_clean_check() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("code.rs"), "fn main() {}\n").unwrap();
    set_phase(dir.path(), Phase::Implement, &[]).unwrap();
    set_phase(dir.path(), Phase::Verify, &[]).unwrap();
    record_clean_check(dir.path(), veneer::laws::clean_hash(dir.path())).unwrap();
    // Tightening the config after the clean check must stale the gate:
    // the recorded verdict was earned under different rules.
    std::fs::write(dir.path().join(".veneer/config.toml"), "loc_hard = 1\n").unwrap();
    let f = set_phase(dir.path(), Phase::Ship, &[]).unwrap_err();
    assert!(f.message.contains("stale"));
}
