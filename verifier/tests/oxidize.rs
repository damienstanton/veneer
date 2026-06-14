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
