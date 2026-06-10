use std::process::Command;

fn veneer(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_veneer"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("binary runs")
}

#[test]
fn usage_error_is_exit_2() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(veneer(dir.path(), &["bogus"]).status.code(), Some(2));
    assert_eq!(veneer(dir.path(), &[]).status.code(), Some(2));
}

#[test]
fn init_materializes_harness_idempotently() {
    let dir = tempfile::tempdir().unwrap();
    let out = veneer(dir.path(), &["init"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(dir.path().join(".veneer/config.toml").exists());
    assert!(dir.path().join(".claude/skills/veneer/SKILL.md").exists());
    assert!(dir.path().join(".agents/skills/veneer/references/verify.md").exists());
    // Re-run converges (idempotent)
    let before = std::fs::read_to_string(dir.path().join(".veneer/config.toml")).unwrap();
    assert_eq!(veneer(dir.path(), &["init"]).status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(dir.path().join(".veneer/config.toml")).unwrap(), before);
}

#[test]
fn check_clean_tree_exits_0_with_empty_json() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("small.rs"), "fn main() {}\n").unwrap();
    let out = veneer(dir.path(), &["check"]);
    assert_eq!(out.status.code(), Some(0));
    let findings: Vec<serde_json::Value> =
        serde_json::from_slice(&out.stdout).expect("stdout is a JSON array");
    assert!(findings.is_empty());
}

#[test]
fn check_error_finding_exits_1_warning_only_exits_0() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("huge.rs"), "l\n".repeat(1200)).unwrap();
    let out = veneer(dir.path(), &["check"]);
    assert_eq!(out.status.code(), Some(1));
    let findings: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(findings[0]["law"], "module_budget");
    // Human trace goes to stderr
    assert!(String::from_utf8_lossy(&out.stderr).contains("module_budget"));

    let dir2 = tempfile::tempdir().unwrap();
    std::fs::write(dir2.path().join("warm.rs"), "l\n".repeat(600)).unwrap();
    assert_eq!(veneer(dir2.path(), &["check"]).status.code(), Some(0));
}

#[test]
fn state_lifecycle_via_cli() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    let out = veneer(dir.path(), &["state", "get"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("plan"));
    assert_eq!(veneer(dir.path(), &["state", "set", "implement", "--ref", "issue=42"]).status.code(), Some(0));
    // Invalid transition → exit 1 with a protocol finding
    let out = veneer(dir.path(), &["state", "set", "ship"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("protocol"));
    // Walk to ship through a clean check
    assert_eq!(veneer(dir.path(), &["state", "set", "verify"]).status.code(), Some(0));
    assert_eq!(veneer(dir.path(), &["check"]).status.code(), Some(0)); // records clean tree
    assert_eq!(veneer(dir.path(), &["state", "set", "ship"]).status.code(), Some(0));
    let out = veneer(dir.path(), &["state", "get"]);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("ship") && s.contains("42"));
}

#[test]
fn check_intent_processes_an_intent_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("sig.rs"), "pub fn api();\n").unwrap();
    std::fs::write(
        dir.path().join("i.json"),
        r#"{"intent":"expand_context","query":"sig.rs"}"#,
    )
    .unwrap();
    let out = veneer(dir.path(), &["check", "--intent", "i.json"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("pub fn api()"));
}
