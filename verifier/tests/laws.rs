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
    let c = load_config(dir.path()).unwrap();
    assert_eq!(c.loc_soft, 300);
    assert_eq!(c.loc_hard, 800);
    assert_eq!(c.modules[0].path, "src/core");
    assert_eq!(c.modules[0].public, vec!["api.rs"]);
}

#[test]
fn missing_config_falls_back_to_default() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load_config(dir.path()).unwrap().loc_soft, 500);
}

#[test]
fn config_parses_loc_exclude() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".veneer")).unwrap();
    std::fs::write(
        dir.path().join(".veneer/config.toml"),
        "loc_exclude = [\".json\", \"docs/\"]\n",
    )
    .unwrap();
    let c = load_config(dir.path()).unwrap();
    assert_eq!(c.loc_exclude, vec![".json".to_string(), "docs/".to_string()]);
    assert!(Config::default().loc_exclude.is_empty());
}

#[test]
fn malformed_config_is_a_protocol_finding() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".veneer")).unwrap();
    std::fs::write(dir.path().join(".veneer/config.toml"), "not [ valid").unwrap();
    let f = load_config(dir.path()).unwrap_err();
    assert_eq!(f.law, Law::Protocol);
    assert!(f.message.contains("malformed config"));
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

use std::collections::BTreeMap;
use veneer::laws::{apply_patch, parse_patch};

const SIMPLE_PATCH: &str = "\
--- a/greet.txt
+++ b/greet.txt
@@ -1,2 +1,2 @@
 hello
-world
+veneer
";

fn tree(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn parse_extracts_paths_and_hunks() {
    let p = parse_patch(SIMPLE_PATCH).unwrap();
    assert_eq!(p.files.len(), 1);
    assert_eq!(p.files[0].path, "greet.txt");
    assert_eq!(p.files[0].hunks.len(), 1);
}

#[test]
fn apply_replaces_lines() {
    let t0 = tree(&[("greet.txt", "hello\nworld\n")]);
    let p = parse_patch(SIMPLE_PATCH).unwrap();
    let t1 = apply_patch(&t0, &p).unwrap();
    assert_eq!(t1["greet.txt"], "hello\nveneer\n");
}

#[test]
fn apply_fails_cleanly_on_context_mismatch() {
    let t0 = tree(&[("greet.txt", "totally\ndifferent\n")]);
    let p = parse_patch(SIMPLE_PATCH).unwrap();
    assert!(apply_patch(&t0, &p).is_err());
}

#[test]
fn new_file_patch_creates_file() {
    let patch = "\
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,2 @@
+alpha
+beta
";
    let p = parse_patch(patch).unwrap();
    let t1 = apply_patch(&tree(&[]), &p).unwrap();
    assert_eq!(t1["new.txt"], "alpha\nbeta\n");
}

#[test]
fn malformed_patch_is_an_error_not_a_panic() {
    assert!(parse_patch("not a patch at all").is_err());
    assert!(parse_patch("--- a/x\n+++ b/x\n@@ garbage @@\n").is_err());
}

use veneer::laws::{check_idempotency, read_tree, tree_hash};

#[test]
fn tree_hash_is_deterministic_and_content_sensitive() {
    let t1 = tree(&[("a.txt", "x\n"), ("b.txt", "y\n")]);
    let t2 = tree(&[("b.txt", "y\n"), ("a.txt", "x\n")]); // same content, BTreeMap orders
    assert_eq!(tree_hash(&t1), tree_hash(&t2));
    let t3 = tree(&[("a.txt", "x\n"), ("b.txt", "z\n")]);
    assert_ne!(tree_hash(&t1), tree_hash(&t3));
}

#[test]
fn read_tree_reads_utf8_files_relative_to_root() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.rs", "fn a() {}\n");
    let t = read_tree(dir.path());
    assert_eq!(t["src/a.rs"], "fn a() {}\n");
}

#[test]
fn replacement_patch_is_idempotent() {
    // Re-application fails context match → detectable no-op → idempotent.
    let t0 = tree(&[("greet.txt", "hello\nworld\n")]);
    assert!(check_idempotency(&t0, SIMPLE_PATCH).is_empty());
}

#[test]
fn pure_insertion_patch_is_not_idempotent() {
    // A hunk that still applies after first application duplicates its line.
    let t0 = tree(&[("log.txt", "start\n")]);
    let patch = "\
--- a/log.txt
+++ b/log.txt
@@ -1,1 +1,2 @@
 start
+entry
";
    // First apply: start,entry. Second apply: context 'start' still matches
    // at line 1 → start,entry,entry. Hashes differ → Idempotency finding.
    let findings = check_idempotency(&t0, patch);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].law, Law::Idempotency);
}

#[test]
fn unparseable_or_inapplicable_patch_is_a_protocol_finding() {
    let t0 = tree(&[]);
    let findings = check_idempotency(&t0, "garbage");
    assert_eq!(findings[0].law, Law::Protocol);
    let findings = check_idempotency(&t0, SIMPLE_PATCH); // greet.txt absent
    assert_eq!(findings[0].law, Law::Protocol);
}

#[cfg(unix)]
#[test]
fn walker_does_not_follow_symlink_cycles() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.rs", "x");
    std::os::unix::fs::symlink(dir.path(), dir.path().join("src/loop")).unwrap();
    let files = walk_files(dir.path());
    assert_eq!(files.len(), 1, "symlinked dir must not be walked: {files:?}");
}

#[test]
fn deletion_patch_removes_file() {
    let patch = "\
--- a/gone.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-alpha
-beta
";
    let t0 = tree(&[("gone.txt", "alpha\nbeta\n")]);
    let p = parse_patch(patch).unwrap();
    let t1 = apply_patch(&t0, &p).unwrap();
    assert!(!t1.contains_key("gone.txt"));
}

#[test]
fn deletion_patch_is_idempotent() {
    // Second apply fails cleanly (file gone) → detectable no-op → idempotent.
    let t0 = tree(&[("gone.txt", "alpha\nbeta\n")]);
    let patch = "--- a/gone.txt\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-alpha\n-beta\n";
    assert!(check_idempotency(&t0, patch).is_empty());
}

#[test]
fn deletion_of_missing_file_fails_cleanly() {
    let patch = "--- a/gone.txt\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-alpha\n";
    assert!(apply_patch(&tree(&[]), &parse_patch(patch).unwrap()).is_err());
}

#[test]
fn multi_hunk_patch_applies_in_order() {
    let t0 = tree(&[("f.txt", "a\nb\nc\nd\ne\nf\n")]);
    let patch = "\
--- a/f.txt
+++ b/f.txt
@@ -1,2 +1,2 @@
 a
-b
+B
@@ -5,2 +5,2 @@
 e
-f
+F
";
    let t1 = apply_patch(&t0, &parse_patch(patch).unwrap()).unwrap();
    assert_eq!(t1["f.txt"], "a\nB\nc\nd\ne\nF\n");
}

#[test]
fn out_of_order_hunks_are_rejected() {
    let t0 = tree(&[("f.txt", "a\nb\nc\nd\ne\nf\n")]);
    let patch = "\
--- a/f.txt
+++ b/f.txt
@@ -5,1 +5,1 @@
-e
+E
@@ -1,1 +1,1 @@
-a
+A
";
    assert!(apply_patch(&t0, &parse_patch(patch).unwrap()).is_err());
}

#[test]
fn no_comma_hunk_header_parses() {
    let t0 = tree(&[("f.txt", "a\nb\nc\n")]);
    let patch = "--- a/f.txt\n+++ b/f.txt\n@@ -2 +2 @@\n-b\n+B\n";
    let t1 = apply_patch(&t0, &parse_patch(patch).unwrap()).unwrap();
    assert_eq!(t1["f.txt"], "a\nB\nc\n");
}

use veneer::laws::{check_sealing, run_checks, ModuleDecl};

fn sealed_cfg() -> Config {
    Config {
        modules: vec![ModuleDecl { path: "src/core".into(), public: vec!["api.rs".into()] }],
        ..Config::default()
    }
}

#[test]
fn reference_to_internal_file_is_a_sealing_error() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/core/api.rs", "pub fn entry() {}\n");
    write(dir.path(), "src/core/detail.rs", "pub fn secret() {}\n");
    write(dir.path(), "src/ui/view.rs", "use crate::core::detail;\n// or: include!(\"../core/detail.rs\")\ncore/detail helper\n");
    let files = walk_files(dir.path());
    let findings = check_sealing(dir.path(), &files, &sealed_cfg());
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].law, Law::ModuleSealing);
    assert_eq!(findings[0].location.path, "src/ui/view.rs");
}

#[test]
fn reference_to_public_surface_is_allowed() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/core/api.rs", "pub fn entry() {}\n");
    write(dir.path(), "src/core/detail.rs", "pub fn secret() {}\n");
    write(dir.path(), "src/ui/view.rs", "use core/api stuff\n");
    let files = walk_files(dir.path());
    assert!(check_sealing(dir.path(), &files, &sealed_cfg()).is_empty());
}

#[test]
fn module_internals_may_reference_each_other() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/core/api.rs", "uses core/detail internally\n");
    write(dir.path(), "src/core/detail.rs", "fn x() {}\n");
    let files = walk_files(dir.path());
    assert!(check_sealing(dir.path(), &files, &sealed_cfg()).is_empty());
}

#[test]
fn no_declared_modules_means_no_sealing_findings() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.rs", "whatever\n");
    let files = walk_files(dir.path());
    assert!(check_sealing(dir.path(), &files, &Config::default()).is_empty());
}

#[test]
fn run_checks_composes_all_laws() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "big.rs", &"l\n".repeat(1200));
    let cfg = Config::default();
    let findings = run_checks(dir.path(), &[], None, &cfg);
    assert!(findings.iter().any(|f| f.law == Law::ModuleBudget));
    // With a diff, idempotency runs against the on-disk tree:
    write(dir.path(), "greet.txt", "hello\nworld\n");
    let findings = run_checks(dir.path(), &[], Some(SIMPLE_PATCH), &cfg);
    assert!(findings.iter().all(|f| f.law != Law::Idempotency));
}

#[test]
fn path_containing_dev_null_is_not_a_deletion() {
    let t0 = tree(&[("src/dev/null_handler.rs", "old\n")]);
    let patch = "\
--- a/src/dev/null_handler.rs
+++ b/src/dev/null_handler.rs
@@ -1,1 +1,1 @@
-old
+new
";
    let t1 = apply_patch(&t0, &parse_patch(patch).unwrap()).unwrap();
    assert_eq!(t1["src/dev/null_handler.rs"], "new\n");
}

#[test]
fn deletion_patch_with_additions_is_rejected() {
    let patch = "--- a/gone.txt\n+++ /dev/null\n@@ -1,1 +1,1 @@\n-alpha\n+beta\n";
    assert!(parse_patch(patch).is_err());
}

#[test]
fn lockfiles_are_not_modules() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "deps.lock", &"line\n".repeat(1200));
    write(dir.path(), "package-lock.json", &"line\n".repeat(1200));
    write(dir.path(), "code.rs", "fn main() {}\n");
    let files = walk_files(dir.path());
    assert_eq!(files.len(), 1, "lockfile must be excluded: {files:?}");
    let cfg = Config::default();
    assert!(check_module_budget(dir.path(), &files, &cfg).is_empty());
}
