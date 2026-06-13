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
    assert!(before.contains("loc_exclude"), "default config must document loc_exclude");
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

#[test]
fn check_diff_runs_idempotency_via_cli() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("greet.txt"), "hello\nworld\n").unwrap();
    std::fs::write(
        dir.path().join("p.patch"),
        "--- a/greet.txt\n+++ b/greet.txt\n@@ -1,2 +1,2 @@\n hello\n-world\n+veneer\n",
    )
    .unwrap();
    let out = veneer(dir.path(), &["check", "--diff", "p.patch"]);
    assert_eq!(out.status.code(), Some(0));
    let findings: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    assert!(findings.is_empty());
    // Garbage patch → protocol finding, exit 1
    std::fs::write(dir.path().join("bad.patch"), "garbage").unwrap();
    let out = veneer(dir.path(), &["check", "--diff", "bad.patch"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("protocol"));
}

#[cfg(unix)]
#[test]
fn init_link_symlinks_resolvably_and_missing_value_errors() {
    let dir = tempfile::tempdir().unwrap();
    // Missing value → usage error, nothing materialized
    let out = veneer(dir.path(), &["init", "--link"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!dir.path().join(".claude/skills/veneer").exists());
    // Relative source → canonicalized, resolvable symlink
    std::fs::create_dir_all(dir.path().join("skillsrc")).unwrap();
    std::fs::write(dir.path().join("skillsrc/SKILL.md"), "stub\n").unwrap();
    let out = veneer(dir.path(), &["init", "--link", "skillsrc"]);
    assert_eq!(out.status.code(), Some(0));
    let content = std::fs::read_to_string(dir.path().join(".claude/skills/veneer/SKILL.md"))
        .expect("symlink must resolve");
    assert_eq!(content, "stub\n");
    // Nonexistent source → exit 2
    let dir2 = tempfile::tempdir().unwrap();
    assert_eq!(veneer(dir2.path(), &["init", "--link", "nope"]).status.code(), Some(2));
}

#[test]
fn stale_gate_refused_via_cli() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    assert_eq!(veneer(dir.path(), &["state", "set", "implement"]).status.code(), Some(0));
    assert_eq!(veneer(dir.path(), &["state", "set", "verify"]).status.code(), Some(0));
    assert_eq!(veneer(dir.path(), &["check"]).status.code(), Some(0));
    std::fs::write(dir.path().join("a.rs"), "fn a() { changed(); }\n").unwrap();
    let out = veneer(dir.path(), &["state", "set", "ship"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("stale"));
}

#[test]
fn mcp_tools_call_invalid_action_returns_protocol_finding() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_veneer"))
        .current_dir(dir.path())
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.as_mut().unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2024-11-05","capabilities":{{}},"clientInfo":{{"name":"t","version":"0"}}}}}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"veneer_state","arguments":{{"action":"bogus"}}}}}}"#).unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains(r#"\"law\":\"protocol\""#) || text.contains(r#""law":"protocol""#),
        "expected protocol finding in: {text}");
}

#[test]
fn mcp_lists_check_and_state_tools() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_veneer"))
        .current_dir(dir.path())
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.as_mut().unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2024-11-05","capabilities":{{}},"clientInfo":{{"name":"t","version":"0"}}}}}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("veneer_check"), "missing veneer_check in: {text}");
    assert!(text.contains("veneer_state"), "missing veneer_state in: {text}");
}

#[test]
fn check_compact_omits_fix_and_stderr() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("huge.rs"), "l\n".repeat(1200)).unwrap();
    let out = veneer(dir.path(), &["check", "--compact"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty(), "compact mode must not render to stderr");
    let findings: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(findings[0]["law"], "module_budget");
    assert!(findings[0].get("suggested_fix").is_none());
    // Default mode is unchanged: stderr render + full schema.
    let out = veneer(dir.path(), &["check"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("module_budget"));
    let findings: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    assert!(findings[0].get("suggested_fix").is_some());
}

#[test]
fn mcp_check_findings_are_compact() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("huge.rs"), "l\n".repeat(1200)).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_veneer"))
        .current_dir(dir.path())
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.as_mut().unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2024-11-05","capabilities":{{}},"clientInfo":{{"name":"t","version":"0"}}}}}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"veneer_check","arguments":{{}}}}}}"#).unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("module_budget"), "expected a budget finding in: {text}");
    // Find the tools/call response (id 2) and parse its embedded findings text.
    let resp = text
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["id"] == 2)
        .expect("tools/call response present");
    let payload = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("content text present");
    let findings: Vec<serde_json::Value> =
        serde_json::from_str(payload).expect("payload is a JSON array of findings");
    assert_eq!(findings[0]["law"], "module_budget");
    assert!(findings[0].get("suggested_fix").is_none(), "MCP findings must be compact: {payload}");
}

#[test]
fn malformed_config_fails_check_via_cli() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".veneer")).unwrap();
    std::fs::write(dir.path().join(".veneer/config.toml"), "loc_soft = \"oops").unwrap();
    let out = veneer(dir.path(), &["check"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("malformed config"));
}

#[test]
fn state_output_omits_gate_internals() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    veneer(dir.path(), &["state", "set", "implement"]);
    veneer(dir.path(), &["state", "set", "verify"]);
    assert_eq!(veneer(dir.path(), &["check"]).status.code(), Some(0)); // records the gate hash
    let out = veneer(dir.path(), &["state", "get"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v.get("last_clean_check").is_none(), "gate internals must not be echoed: {v}");
    assert_eq!(v["phase"], "verify");
    // The gate itself still functions on the file's data.
    assert_eq!(veneer(dir.path(), &["state", "set", "ship"]).status.code(), Some(0));
}

#[test]
fn mcp_state_response_omits_gate_internals() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_veneer"))
        .current_dir(dir.path())
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.as_mut().unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2024-11-05","capabilities":{{}},"clientInfo":{{"name":"t","version":"0"}}}}}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"veneer_state","arguments":{{"action":"get"}}}}}}"#).unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("plan"), "expected default phase in: {text}");
    assert!(!text.contains("last_clean_check"), "MCP state must be trimmed: {text}");
}

#[test]
fn clean_check_short_circuits_and_edits_revive_findings() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    assert_eq!(veneer(dir.path(), &["check"]).status.code(), Some(0)); // records clean
    // Unchanged tree+config: still clean, identical output.
    let out = veneer(dir.path(), &["check"]);
    assert_eq!(out.status.code(), Some(0));
    let findings: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    assert!(findings.is_empty());
    // An edit after the clean check re-runs the laws and finds violations.
    std::fs::write(dir.path().join("a.rs"), "l\n".repeat(1200)).unwrap();
    assert_eq!(veneer(dir.path(), &["check"]).status.code(), Some(1));
    // A config edit alone also defeats the short-circuit (soundness):
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    assert_eq!(veneer(dir.path(), &["check"]).status.code(), Some(0)); // clean again, records
    std::fs::create_dir_all(dir.path().join(".veneer")).unwrap();
    std::fs::write(dir.path().join(".veneer/config.toml"), "loc_hard = 0\nloc_soft = 0\n").unwrap();
    assert_eq!(veneer(dir.path(), &["check"]).status.code(), Some(1)); // re-checked under new rules
}

#[test]
fn compact_check_honors_malformed_config() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".veneer")).unwrap();
    std::fs::write(dir.path().join(".veneer/config.toml"), "loc_soft = \"oops").unwrap();
    let out = veneer(dir.path(), &["check", "--compact"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty(), "compact mode must not render to stderr");
    let findings: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(findings[0]["law"], "protocol");
    assert!(findings[0].get("suggested_fix").is_none());
}
