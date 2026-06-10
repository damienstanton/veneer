use veneer::laws::{load_config, Config, Finding, Law, Location, Severity};

#[test]
fn finding_json_is_stable() {
    // Golden: the machine trace schema. Identical input ⇒ byte-identical output.
    let f = Finding {
        law: Law::ModuleBudget,
        severity: Severity::Warning,
        location: Location { path: "src/big.rs".into(), line: None },
        message: "module is 612 LoC, above soft bound 500".into(),
        suggested_fix: Some("split into first-principles modules (target ~500 LoC)".into()),
    };
    assert_eq!(
        serde_json::to_string(&f).unwrap(),
        r#"{"law":"module_budget","severity":"warning","location":{"path":"src/big.rs"},"message":"module is 612 LoC, above soft bound 500","suggested_fix":"split into first-principles modules (target ~500 LoC)"}"#
    );
}

#[test]
fn finding_with_line_serializes_line() {
    let f = Finding::error(Law::Protocol, "x.json", Some(3), "bad intent", None);
    let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
    assert_eq!(v["location"]["line"], 3);
    assert_eq!(v["suggested_fix"], serde_json::Value::Null);
}

#[test]
fn config_defaults_are_the_band() {
    let c = Config::default();
    assert_eq!(c.loc_soft, 500);
    assert_eq!(c.loc_hard, 1000);
    assert!(c.modules.is_empty());
}

#[test]
fn config_loads_from_veneer_dir() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".veneer")).unwrap();
    std::fs::write(
        dir.path().join(".veneer/config.toml"),
        "loc_soft = 300\nloc_hard = 800\n\n[[modules]]\npath = \"src/core\"\npublic = [\"api.rs\"]\n",
    )
    .unwrap();
    let c = load_config(dir.path());
    assert_eq!(c.loc_soft, 300);
    assert_eq!(c.loc_hard, 800);
    assert_eq!(c.modules[0].path, "src/core");
    assert_eq!(c.modules[0].public, vec!["api.rs"]);
}

#[test]
fn missing_or_corrupt_config_falls_back_to_default() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load_config(dir.path()).loc_soft, 500);
    std::fs::create_dir(dir.path().join(".veneer")).unwrap();
    std::fs::write(dir.path().join(".veneer/config.toml"), "not [ valid").unwrap();
    assert_eq!(load_config(dir.path()).loc_soft, 500);
}

use std::path::PathBuf;
use veneer::laws::{check_module_budget, loc, walk_files};

fn write(dir: &std::path::Path, rel: &str, contents: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, contents).unwrap();
}

#[test]
fn loc_counts_non_blank_lines() {
    assert_eq!(loc("a\n\nb\n  \nc\n"), 3);
    assert_eq!(loc(""), 0);
}

#[test]
fn walker_skips_ignored_dirs_and_is_sorted() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.rs", "x");
    write(dir.path(), "src/b.rs", "x");
    write(dir.path(), ".git/c", "x");
    write(dir.path(), "target/d.rs", "x");
    write(dir.path(), ".veneer/state.json", "x");
    let files = walk_files(dir.path());
    let rels: Vec<PathBuf> = files
        .iter()
        .map(|f| f.strip_prefix(dir.path()).unwrap().to_path_buf())
        .collect();
    assert_eq!(rels, vec![PathBuf::from("src/a.rs"), PathBuf::from("src/b.rs")]);
}

#[test]
fn module_budget_warns_above_soft_errors_above_hard() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "ok.rs", &"line\n".repeat(400));
    write(dir.path(), "warn.rs", &"line\n".repeat(600));
    write(dir.path(), "err.rs", &"line\n".repeat(1200));
    let cfg = veneer::laws::Config::default();
    let files = walk_files(dir.path());
    let findings = check_module_budget(dir.path(), &files, &cfg);
    assert_eq!(findings.len(), 2);
    let err = findings.iter().find(|f| f.location.path == "err.rs").unwrap();
    assert_eq!(err.severity, veneer::laws::Severity::Error);
    assert!(err.message.contains("1200 LoC"));
    assert!(err.message.contains("hard bound 1000"));
    let warn = findings.iter().find(|f| f.location.path == "warn.rs").unwrap();
    assert_eq!(warn.severity, veneer::laws::Severity::Warning);
}

#[test]
fn binary_files_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("blob.bin"), [0u8, 159, 146, 150]).unwrap();
    let cfg = veneer::laws::Config::default();
    let files = walk_files(dir.path());
    assert!(check_module_budget(dir.path(), &files, &cfg).is_empty());
}
