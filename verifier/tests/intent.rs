use veneer::intent::{execute, parse_intent, AgentIntent, Outcome};
use veneer::laws::{Config, Law};

#[test]
fn intents_roundtrip_through_json() {
    let cases = [
        (r#"{"intent":"expand_context","query":"src/api.rs"}"#, AgentIntent::ExpandContext { query: "src/api.rs".into() }),
        (r#"{"intent":"propose_diff","patch":"--- a/x\n"}"#, AgentIntent::ProposeDiff { patch: "--- a/x\n".into() }),
        (r#"{"intent":"conclude","summary":"done"}"#, AgentIntent::Conclude { summary: "done".into() }),
    ];
    for (json, expected) in cases {
        assert_eq!(parse_intent(json).unwrap(), expected);
    }
}

#[test]
fn malformed_intent_is_a_protocol_finding() {
    let f = parse_intent(r#"{"intent":"launch_missiles"}"#).unwrap_err();
    assert_eq!(f.law, Law::Protocol);
    let f = parse_intent("not json").unwrap_err();
    assert_eq!(f.law, Law::Protocol);
}

#[test]
fn expand_context_returns_content_within_budget() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("sig.rs"), "pub fn api();\n").unwrap();
    let out = execute(
        dir.path(),
        AgentIntent::ExpandContext { query: "sig.rs".into() },
        &Config::default(),
    );
    match out {
        Outcome::Context(c) => assert_eq!(c, "pub fn api();\n"),
        other => panic!("expected Context, got {other:?}"),
    }
}

#[test]
fn expand_context_refuses_over_budget_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("huge.rs"), "l\n".repeat(1500)).unwrap();
    let out = execute(
        dir.path(),
        AgentIntent::ExpandContext { query: "huge.rs".into() },
        &Config::default(),
    );
    match out {
        Outcome::Findings(fs) => {
            assert_eq!(fs[0].law, Law::Protocol);
            assert!(fs[0].message.contains("exceeds the context budget"));
        }
        other => panic!("expected Findings, got {other:?}"),
    }
}

#[test]
fn propose_diff_runs_the_laws() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("greet.txt"), "hello\nworld\n").unwrap();
    let patch = "--- a/greet.txt\n+++ b/greet.txt\n@@ -1,2 +1,2 @@\n hello\n-world\n+veneer\n";
    let out = execute(dir.path(), AgentIntent::ProposeDiff { patch: patch.into() }, &Config::default());
    match out {
        Outcome::Findings(fs) => assert!(fs.is_empty()),
        other => panic!("expected Findings, got {other:?}"),
    }
}

#[test]
fn oxidize_intent_roundtrips() {
    let json = r#"{"intent":"oxidize","shadow":"pub fn f() {}\n"}"#;
    let parsed = parse_intent(json).unwrap();
    assert_eq!(parsed, AgentIntent::Oxidize { shadow: "pub fn f() {}\n".into() });
}

#[test]
fn query_graph_intent_roundtrips() {
    let json = r#"{"intent":"query_graph","query":"src/api.rs"}"#;
    let parsed = parse_intent(json).unwrap();
    assert_eq!(parsed, AgentIntent::QueryGraph { query: "src/api.rs".into() });
}

#[test]
fn query_graph_returns_entry_and_staleness() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "pub fn f() {}\n").unwrap();
    let g = veneer::graph::build(dir.path(), &Config::default()).unwrap();
    veneer::graph::store(dir.path(), &g).unwrap();
    let out = execute(dir.path(), AgentIntent::QueryGraph { query: "a.rs".into() }, &Config::default());
    match out {
        Outcome::GraphQuery(Some(entry), stale) => {
            assert_eq!(entry.path, "a.rs");
            assert!(!stale);
        }
        other => panic!("expected GraphQuery(Some, false), got {other:?}"),
    }
}

#[test]
fn query_graph_missing_entry_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let out = execute(dir.path(), AgentIntent::QueryGraph { query: "missing.rs".into() }, &Config::default());
    match out {
        Outcome::GraphQuery(None, _) => {}
        other => panic!("expected GraphQuery(None, _), got {other:?}"),
    }
}
