//! End-to-end CLI/MCP tests for the knowledge graph: `graph build`/`query`,
//! the automatic refresh on a clean full `check`, and proof that the graph
//! stays orthogonal to `check`'s findings and the ship gate. Split out from
//! `cli.rs` so each test module stays within the first-principles LoC band.

use std::process::Command;

fn veneer(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_veneer"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("binary runs")
}

/// One initialize + one tools/call over a fresh MCP session, returning the
/// call's text payload. Two dependent calls (e.g. build-then-query) must use
/// two separate sessions: rmcp dispatch does not guarantee that pipelining a
/// second request before reading the first response preserves completion
/// order, and `build`'s `cargo check` is far slower than `query`'s file read.
fn mcp_call(dir: &std::path::Path, name: &str, args_json: &str) -> String {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_veneer"))
        .current_dir(dir)
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.as_mut().unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2024-11-05","capabilities":{{}},"clientInfo":{{"name":"t","version":"0"}}}}}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"{name}","arguments":{args_json}}}}}"#).unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    text.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["id"] == 2)
        .unwrap_or_else(|| panic!("no tools/call response in: {text}"))["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no text payload in: {text}"))
        .to_string()
}

#[test]
fn mcp_graph_build_then_query_is_compact() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "pub fn pick(x: &Foo, y: &Foo) -> &Foo { x }\n").unwrap();
    mcp_call(dir.path(), "veneer_graph", r#"{"action":"build"}"#);
    assert!(dir.path().join(".veneer/graph.toon").exists(), "build must have persisted the graph");

    let query_payload = mcp_call(dir.path(), "veneer_graph", r#"{"action":"query","query":"a.rs"}"#);
    assert!(!query_payload.contains("suggested_fix"), "MCP graph query must be compact: {query_payload}");
    let v: serde_json::Value = serde_json::from_str(&query_payload).unwrap();
    assert_eq!(v["entry"]["path"], "a.rs");
    assert_eq!(v["stale"], false);
}

#[test]
fn graph_build_then_query_round_trips_via_cli() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "//! Doc.\npub fn add(x: i32, y: i32) -> i32 { x + y }\n").unwrap();
    let build_out = veneer(dir.path(), &["graph", "build"]);
    assert_eq!(build_out.status.code(), Some(0));
    let findings: serde_json::Value = serde_json::from_slice(&build_out.stdout).expect("build emits a findings array");
    assert!(findings.is_array());
    assert!(dir.path().join(".veneer/graph.toon").exists());

    let query_out = veneer(dir.path(), &["graph", "query", "a.rs"]);
    assert_eq!(query_out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&query_out.stdout).expect("query emits JSON");
    assert_eq!(v["stale"], false);
    assert_eq!(v["entry"]["path"], "a.rs");
    assert_eq!(v["entry"]["doc_summary"], "Doc.");
}

#[test]
fn graph_query_missing_entry_returns_null() {
    let dir = tempfile::tempdir().unwrap();
    let out = veneer(dir.path(), &["graph", "query", "nope.rs"]);
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["entry"].is_null());
}

#[test]
fn graph_query_without_target_is_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = veneer(dir.path(), &["graph", "query"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn graph_build_rejects_extra_args_after_compact() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(veneer(dir.path(), &["graph", "build", "--compact", "extra"]).status.code(), Some(2));
    assert_eq!(veneer(dir.path(), &["graph", "build", "bogus"]).status.code(), Some(2));
}

#[test]
fn graph_query_rejects_multiple_targets_and_unknown_flags() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(veneer(dir.path(), &["graph", "query", "a.rs", "b.rs"]).status.code(), Some(2));
    assert_eq!(veneer(dir.path(), &["graph", "query", "--comapct", "a.rs"]).status.code(), Some(2));
    // Both orderings of a valid --compact + target still work.
    assert_eq!(veneer(dir.path(), &["graph", "query", "--compact", "a.rs"]).status.code(), Some(0));
    assert_eq!(veneer(dir.path(), &["graph", "query", "a.rs", "--compact"]).status.code(), Some(0));
}

#[test]
fn check_findings_are_identical_with_or_without_a_graph_present() {
    // .veneer/ is already walker-skipped, but this proves it end-to-end for
    // the graph cache specifically: building it must not change `check`'s
    // verdict on an unchanged source tree. `check <path>` is used (rather
    // than the bare, path-less form) to bypass the clean-tree short-circuit
    // and force a real re-evaluation on both sides of the comparison.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    let without = veneer(dir.path(), &["check", "--compact", "a.rs"]);
    assert_eq!(without.status.code(), Some(0));

    assert_eq!(veneer(dir.path(), &["graph", "build"]).status.code(), Some(0));
    assert!(dir.path().join(".veneer/graph.toon").exists());

    let with = veneer(dir.path(), &["check", "--compact", "a.rs"]);
    assert_eq!(with.status.code(), Some(0));
    assert_eq!(without.stdout, with.stdout, "graph presence must not change check findings");
}

#[test]
fn clean_full_check_rebuilds_the_graph_automatically() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "pub fn f(x: i32) -> i32 { x }\n").unwrap();
    // No explicit `graph build` is run.
    assert!(!dir.path().join(".veneer/graph.toon").exists());
    let out = veneer(dir.path(), &["check", "--compact"]);
    assert_eq!(out.status.code(), Some(0));
    // The clean full check refreshed the cache as a transparent side effect.
    assert!(dir.path().join(".veneer/graph.toon").exists(), "clean check should rebuild the graph");
    let q = veneer(dir.path(), &["graph", "query", "a.rs"]);
    let v: serde_json::Value = serde_json::from_slice(&q.stdout).unwrap();
    assert_eq!(v["stale"], false, "auto-rebuilt graph must be fresh");
    assert_eq!(v["entry"]["path"], "a.rs");
}

#[test]
fn path_scoped_check_does_not_rebuild_the_graph() {
    // Auto-rebuild is gated to full checks only; a path-scoped check is a
    // targeted query, not the once-per-cycle "tree is clean" event.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "pub fn f() {}\n").unwrap();
    assert_eq!(veneer(dir.path(), &["check", "--compact", "a.rs"]).status.code(), Some(0));
    assert!(!dir.path().join(".veneer/graph.toon").exists(), "path-scoped check must not rebuild");
}

#[test]
fn building_or_rebuilding_the_graph_does_not_stale_the_ship_gate() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    assert_eq!(veneer(dir.path(), &["graph", "build"]).status.code(), Some(0)); // before the clean check
    assert_eq!(veneer(dir.path(), &["state", "set", "implement"]).status.code(), Some(0));
    assert_eq!(veneer(dir.path(), &["state", "set", "verify"]).status.code(), Some(0));
    assert_eq!(veneer(dir.path(), &["check"]).status.code(), Some(0)); // records the clean check
    assert_eq!(veneer(dir.path(), &["graph", "build"]).status.code(), Some(0)); // rebuild after
    let out = veneer(dir.path(), &["state", "set", "ship"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", String::from_utf8_lossy(&out.stdout));
}
