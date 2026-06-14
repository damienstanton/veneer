use veneer::laws::{Law, Severity};
use veneer::oxidize::parse_diagnostics;

// A real `cargo check --message-format=json` line for a use-after-move error.
const MOVED: &str = r#"{"reason":"compiler-message","message":{"message":"use of moved value: `c`","level":"error","spans":[{"line_start":4,"is_primary":false},{"line_start":5,"is_primary":true},{"line_start":3,"is_primary":false}]}}"#;

#[test]
fn maps_error_using_primary_span_line() {
    let fs = parse_diagnostics(MOVED);
    assert_eq!(fs.len(), 1);
    assert_eq!(fs[0].law, Law::Oxidation);
    assert_eq!(fs[0].severity, Severity::Error);
    assert_eq!(fs[0].location.path, "<shadow>");
    assert_eq!(fs[0].location.line, Some(5)); // the primary span, not the first
    assert_eq!(fs[0].message, "use of moved value: `c`");
}

#[test]
fn maps_warning_severity() {
    let line = r#"{"reason":"compiler-message","message":{"message":"unused variable: `x`","level":"warning","spans":[{"line_start":2,"is_primary":true}]}}"#;
    let fs = parse_diagnostics(line);
    assert_eq!(fs.len(), 1);
    assert_eq!(fs[0].severity, Severity::Warning);
}

#[test]
fn skips_failure_notes_and_non_messages() {
    let stream = [
        r#"{"reason":"compiler-message","message":{"message":"For more information about this error, try `rustc --explain E0382`.","level":"failure-note","spans":[]}}"#,
        r#"{"reason":"build-finished","success":false}"#,
        r#"{"reason":"compiler-artifact"}"#,
    ].join("\n");
    assert!(parse_diagnostics(&stream).is_empty());
}

#[test]
fn skips_aborting_summary() {
    let line = r#"{"reason":"compiler-message","message":{"message":"aborting due to 1 previous error","level":"error","spans":[]}}"#;
    assert!(parse_diagnostics(line).is_empty());
}

#[test]
fn no_span_error_maps_to_null_line() {
    let line = r#"{"reason":"compiler-message","message":{"message":"crate-level problem","level":"error","spans":[]}}"#;
    let fs = parse_diagnostics(line);
    assert_eq!(fs.len(), 1);
    assert_eq!(fs[0].location.line, None);
}

#[test]
fn findings_are_sorted_for_determinism() {
    let stream = [
        r#"{"reason":"compiler-message","message":{"message":"zeta","level":"error","spans":[{"line_start":9,"is_primary":true}]}}"#,
        r#"{"reason":"compiler-message","message":{"message":"alpha","level":"error","spans":[{"line_start":2,"is_primary":true}]}}"#,
    ].join("\n");
    let fs = parse_diagnostics(&stream);
    assert_eq!(fs[0].location.line, Some(2));
    assert_eq!(fs[1].location.line, Some(9));
}

use std::path::Path;

fn cargo_available() -> bool {
    std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn ox(root: &Path, shadow: &str) -> Vec<veneer::laws::Finding> {
    veneer::oxidize::oxidize(root, shadow, &veneer::oxidize::OxidizeConfig::default())
}

#[test]
fn coherent_shadow_has_no_findings() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let shadow = "pub fn add(a: u8, b: u8) -> u8 { a + b }\n";
    assert!(ox(dir.path(), shadow).is_empty());
}

#[test]
fn borrow_error_shadow_yields_one_oxidation_error() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let shadow = "\
fn close(c: Vec<u8>) { let _ = c; }
pub fn bad() {
    let c = vec![1u8];
    close(c);
    close(c);
}
";
    let fs = ox(dir.path(), shadow);
    let errors: Vec<_> = fs
        .iter()
        .filter(|f| f.law == veneer::laws::Law::Oxidation && f.severity == veneer::laws::Severity::Error)
        .collect();
    assert_eq!(errors.len(), 1, "got: {fs:?}");
    assert!(errors[0].message.contains("moved"));
}

#[test]
fn same_shadow_twice_is_byte_identical() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let shadow = "pub fn f(x: bool) -> u8 { if x { 1 } }\n"; // missing else: type error
    let a = serde_json::to_string(&ox(dir.path(), shadow)).unwrap();
    let b = serde_json::to_string(&ox(dir.path(), shadow)).unwrap();
    assert_eq!(a, b);
    assert!(!a.is_empty() && a != "[]");
}

#[test]
fn scratch_crate_is_invisible_to_run_checks() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let _ = ox(dir.path(), "pub fn f() {}\n"); // creates .veneer/oxidize/
    let cfg = veneer::laws::Config::default();
    let findings = veneer::laws::run_checks(dir.path(), &[], None, &cfg);
    // Nothing under .veneer/ is walked, so the (huge) target/ never trips the
    // module budget and no oxidize file appears in any finding location.
    assert!(findings.iter().all(|f| !f.location.path.contains("oxidize")), "got: {findings:?}");
}
