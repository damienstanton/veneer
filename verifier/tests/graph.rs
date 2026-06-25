use veneer::graph::{
    build, complexity_score, entry_json_compact, extract_doc_summary, extract_signatures, is_stale, lift_shadow,
    load, query, store, Graph,
};
use veneer::laws::Config;

fn cargo_available() -> bool {
    std::process::Command::new("cargo").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn ox(root: &std::path::Path, shadow: &str) -> Vec<veneer::laws::Finding> {
    veneer::oxidize::oxidize(root, shadow, &veneer::oxidize::OxidizeConfig::default())
}

#[test]
fn extracts_public_rust_signatures_only() {
    let text = "fn private_fn() {}\npub fn public_fn(x: i32) -> i32 { x }\npub struct Foo;\nstruct Bar;\n";
    let sigs = extract_signatures("a.rs", text);
    assert_eq!(sigs, vec!["pub fn public_fn(x: i32) -> i32".to_string(), "pub struct Foo;".to_string()]);
}

#[test]
fn extracts_top_level_python_def_and_class() {
    let text = "def top_level(x):\n    pass\n    def nested(y):\n        pass\nclass Foo:\n    pass\n";
    let sigs = extract_signatures("a.py", text);
    assert_eq!(sigs, vec!["def top_level(x):".to_string(), "class Foo:".to_string()]);
}

#[test]
fn extracts_exported_typescript_declarations() {
    let text = "function helper() {}\nexport function publicFn(x: number): number { return x; }\nexport interface Shape { x: number }\n";
    let sigs = extract_signatures("a.ts", text);
    assert_eq!(
        sigs,
        vec!["export function publicFn(x: number): number".to_string(), "export interface Shape".to_string()]
    );
}

#[test]
fn unknown_extension_yields_no_signatures() {
    let text = "pub fn looks_like_rust() {}\n";
    assert!(extract_signatures("notes.txt", text).is_empty());
}

#[test]
fn extracts_rust_module_doc_summary() {
    let text = "//! First line of module doc.\n//! Second line.\nuse std::fmt;\n";
    assert_eq!(extract_doc_summary("a.rs", text), Some("First line of module doc.\nSecond line.".to_string()));
}

#[test]
fn extracts_python_module_docstring() {
    let text = "\"\"\"Module summary.\nMore detail.\n\"\"\"\nimport os\n";
    assert_eq!(extract_doc_summary("a.py", text), Some("Module summary.\nMore detail.".to_string()));
}

#[test]
fn extracts_ts_leading_block_comment() {
    let text = "/**\n * Module summary.\n * More detail.\n */\nexport function f() {}\n";
    assert_eq!(extract_doc_summary("a.ts", text), Some("Module summary.\nMore detail.".to_string()));
}

#[test]
fn no_leading_doc_comment_yields_none() {
    let text = "pub fn f() {}\n";
    assert_eq!(extract_doc_summary("a.rs", text), None);
}

#[test]
fn complexity_score_counts_branch_keywords_and_logical_operators() {
    let text = "fn f() { if a && b || c { } else { for i in xs {} } }";
    // if, &&, ||, else, for = 5
    assert_eq!(complexity_score(text), 5);
}

#[test]
fn complexity_score_is_zero_for_branch_free_text() {
    assert_eq!(complexity_score("fn f() -> i32 { 42 }"), 0);
}

#[test]
fn complexity_score_does_not_match_substrings_of_words() {
    // "forest" and "matches" must not be counted as "for"/"match".
    assert_eq!(complexity_score("let forest = matches.len();"), 0);
}

#[test]
fn lift_shadow_leaves_primitive_only_signature_unchanged() {
    let shadow = lift_shadow(&["pub fn add(x: i32, y: i32) -> i32".to_string()]);
    assert!(shadow.starts_with("#![allow(unused, dead_code)]"));
    assert!(shadow.contains("pub fn add(x: i32, y: i32) -> i32 { todo!() }"), "got: {shadow}");
}

#[test]
fn lift_shadow_erases_one_custom_type_consistently() {
    let shadow = lift_shadow(&["pub fn f(x: Foo) -> Foo".to_string()]);
    assert!(shadow.contains("pub fn f<T0>(x: T0) -> T0 { todo!() }"), "got: {shadow}");
}

#[test]
fn lift_shadow_erases_distinct_custom_types_to_distinct_generics() {
    let shadow = lift_shadow(&["pub fn f(x: Foo, y: Bar) -> Foo".to_string()]);
    assert!(shadow.contains("pub fn f<T0, T1>(x: T0, y: T1) -> T0 { todo!() }"), "got: {shadow}");
}

#[test]
fn lift_shadow_preserves_ownership_shape_through_std_containers() {
    let shadow = lift_shadow(&["pub fn f(x: &mut Vec<Foo>, y: &Foo)".to_string()]);
    assert!(shadow.contains("pub fn f<T0>(x: &mut Vec<T0>, y: &T0) { todo!() }"), "got: {shadow}");
}

#[test]
fn lift_shadow_skips_signatures_with_pre_existing_generics_rather_than_mangling_them() {
    // A signature like `pub fn query<'a>(...)` already declares its own
    // generic/lifetime list; naively appending an erasure-generated `<T0>`
    // produces invalid syntax (`query<'a><T0>(...)`). Skipping it is honest:
    // this heuristic lift targets simple concrete signatures, not ones that
    // already use generics.
    let shadow = lift_shadow(&["pub fn query<'a>(g: &'a Graph, target: &str) -> Option<&'a GraphEntry>".to_string()]);
    assert!(!shadow.contains("query"), "got: {shadow}");
}

#[test]
fn lift_shadow_erases_qualified_paths_from_external_crates_as_one_unit() {
    // A real signature from this codebase: `serde_json::Value` cannot resolve
    // in the bare scratch crate (no dependencies), so the whole qualified
    // path must be erased — not just its trailing segment.
    let shadow = lift_shadow(&["pub fn f(x: serde_json::Value) -> serde_json::Value".to_string()]);
    assert!(
        shadow.contains("pub fn f<T0>(x: T0) -> T0 { todo!() }"),
        "got: {shadow}"
    );
}

#[test]
fn lift_shadow_leaves_std_qualified_paths_untouched() {
    // `std::`/`core::` paths resolve in any crate without a `use` — another
    // real signature from this codebase (`std::io::Result<()>`).
    let shadow = lift_shadow(&["pub fn f() -> std::io::Result<()>".to_string()]);
    assert!(shadow.contains("pub fn f() -> std::io::Result<()> { todo!() }"), "got: {shadow}");
}

#[test]
fn lift_shadow_skips_unbalanced_signatures_from_multi_line_declarations() {
    // `extract_signatures` captures only a declaration's opening line; a
    // multi-line parameter list (params continuing past `(` on later lines)
    // truncates to an unclosed paren. Lifting that as-is would corrupt the
    // *entire* shadow file with an unclosed-delimiter error, drowning out
    // every other (validly lifted) function in the same file — skip it
    // instead, exactly like a pre-existing-generics signature.
    let shadow = lift_shadow(&["pub fn apply_patch(".to_string()]);
    assert!(!shadow.contains("apply_patch"), "got: {shadow}");
}

#[test]
fn lift_shadow_drops_generic_args_of_an_erased_type() {
    // A real signature from this codebase: `BTreeMap` isn't in the prelude
    // allowlist, so it gets erased to `T0` — but a bare generic parameter
    // cannot itself take type arguments, so the trailing `<String, String>`
    // (which belonged to `BTreeMap`, not to the erased `T0`) must be dropped
    // too, not left dangling as `T0<String, String>`.
    let shadow =
        lift_shadow(&["pub fn tree_hash(tree: &BTreeMap<String, String>) -> u64".to_string()]);
    assert!(shadow.contains("pub fn tree_hash<T0>(tree: &T0) -> u64 { todo!() }"), "got: {shadow}");
}

#[test]
fn lift_shadow_skips_non_function_signatures() {
    let shadow = lift_shadow(&["pub struct Foo;".to_string()]);
    assert!(!shadow.contains("Foo"), "got: {shadow}");
}

#[test]
fn lifted_primitive_signature_actually_compiles() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let shadow = lift_shadow(&["pub fn add(x: i32, y: i32) -> i32".to_string()]);
    assert!(ox(dir.path(), &shadow).is_empty(), "lifted shadow should compile cleanly: {shadow}");
}

#[test]
fn lifted_ambiguous_lifetime_signature_yields_a_real_oxidation_finding() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Two reference params, one reference return, no self: lifetime elision
    // is genuinely ambiguous here regardless of the erased concrete type —
    // this is real rustc signal arising from the signature's shape alone.
    let shadow = lift_shadow(&["pub fn pick(x: &Foo, y: &Foo) -> &Foo".to_string()]);
    let findings = ox(dir.path(), &shadow);
    assert_eq!(findings.len(), 1, "expected one lifetime-ambiguity finding: {shadow}\nfindings: {findings:?}");
    assert_eq!(findings[0].law, veneer::laws::Law::Oxidation);
    assert!(findings[0].message.contains("lifetime"), "expected a lifetime-related message: {:?}", findings[0]);
}

#[test]
fn build_attributes_semantic_findings_to_the_real_file_not_the_generic_shadow_label() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "pub fn pick(x: &Foo, y: &Foo) -> &Foo { x }\n").unwrap();
    let g = build(dir.path(), &Config::default()).unwrap();
    let e = &g.entries["a.rs"];
    assert!(!e.semantic_findings.is_empty());
    assert_eq!(e.semantic_findings[0].location.path, "a.rs", "{:?}", e.semantic_findings);
}

#[test]
fn build_extracts_a_clean_rust_module_with_no_semantic_findings() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), "//! A tiny module.\npub fn add(x: i32, y: i32) -> i32 { x + y }\n")
        .unwrap();
    let g = build(dir.path(), &Config::default()).unwrap();
    let e = g.entries.get("lib.rs").expect("lib.rs entry");
    assert_eq!(e.path, "lib.rs");
    assert_eq!(e.signatures, vec!["pub fn add(x: i32, y: i32) -> i32".to_string()]);
    assert_eq!(e.doc_summary, Some("A tiny module.".to_string()));
    assert_eq!(e.loc, 2);
    assert!(e.canonical_form.as_ref().unwrap().contains("pub fn add(x: i32, y: i32) -> i32 { todo!() }"));
    assert!(e.semantic_findings.is_empty(), "{:?}", e.semantic_findings);
}

#[test]
fn build_skips_canonical_form_for_non_rust_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.py"), "def f(x):\n    pass\n").unwrap();
    let g = build(dir.path(), &Config::default()).unwrap();
    let e = g.entries.get("a.py").expect("a.py entry");
    assert_eq!(e.canonical_form, None);
    assert!(e.semantic_findings.is_empty());
}

#[test]
fn graph_persists_as_toon_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "pub fn f(x: i32) -> i32 { x }\n").unwrap();
    let g = build(dir.path(), &Config::default()).unwrap();
    store(dir.path(), &g).unwrap();
    let toon = dir.path().join(".veneer/graph.toon");
    assert!(toon.exists());
    let body = std::fs::read_to_string(&toon).unwrap();
    assert!(!body.trim_start().starts_with('{'), "on-disk graph is TOON, not JSON: {body}");
    assert_eq!(load(dir.path()).unwrap(), g);
}

#[test]
fn graph_with_real_semantic_findings_persists_and_round_trips() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // A real Oxidation finding carries a nested `location: {path, line}` —
    // toon_rust's tabular-array encoder previously choked on this.
    std::fs::write(dir.path().join("a.rs"), "pub fn pick(x: &Foo, y: &Foo) -> &Foo { x }\n").unwrap();
    let g = build(dir.path(), &Config::default()).unwrap();
    assert!(!g.entries["a.rs"].semantic_findings.is_empty(), "fixture should have a real finding");
    store(dir.path(), &g).unwrap();
    assert_eq!(load(dir.path()).unwrap(), g);
}

#[test]
fn graph_with_non_ascii_doc_comments_persists_and_round_trips() {
    // toon-rust 0.1.3's decoder miscomputes offsets across multi-byte UTF-8
    // characters: a single em dash in a doc comment corrupts parsing of
    // every subsequent line in the document (confirmed against the real
    // graph.toon this build produces for this very repo, which has em
    // dashes throughout its own doc comments).
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("a.rs"),
        "//! Heuristic — not an AST — honest about café, naïve, and 日本語 text.\npub fn f() {}\n",
    )
    .unwrap();
    let g = build(dir.path(), &Config::default()).unwrap();
    assert!(g.entries["a.rs"].doc_summary.as_ref().unwrap().contains('—'), "fixture must contain non-ASCII");
    store(dir.path(), &g).unwrap();
    assert_eq!(load(dir.path()).unwrap(), g);
}

#[test]
fn missing_graph_loads_as_empty_default() {
    let dir = tempfile::tempdir().unwrap();
    let g = load(dir.path()).unwrap();
    assert!(g.entries.is_empty());
}

#[test]
fn tampered_graph_toon_is_a_protocol_finding() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "pub fn f() {}\n").unwrap();
    let g = build(dir.path(), &Config::default()).unwrap();
    store(dir.path(), &g).unwrap();
    let p = dir.path().join(".veneer/graph.toon");
    let body = std::fs::read_to_string(&p).unwrap();
    assert!(body.contains("fnv:"));
    std::fs::write(&p, body.replacen("fnv:", "fnv:ff", 1)).unwrap();
    assert_eq!(load(dir.path()).unwrap_err().law, veneer::laws::Law::Protocol);
}

#[test]
fn freshly_built_graph_is_not_stale_but_a_source_edit_makes_it_stale() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "pub fn f() {}\n").unwrap();
    let g = build(dir.path(), &Config::default()).unwrap();
    assert!(!is_stale(&g, dir.path()));
    std::fs::write(dir.path().join("a.rs"), "pub fn f() { /* changed */ }\n").unwrap();
    assert!(is_stale(&g, dir.path()));
}

#[test]
fn entry_json_compact_strips_suggested_fix_from_semantic_findings() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "pub fn pick(x: &Foo, y: &Foo) -> &Foo { x }\n").unwrap();
    let g = build(dir.path(), &Config::default()).unwrap();
    let entry = query(&g, "a.rs").unwrap();
    assert!(!entry.semantic_findings.is_empty(), "expected a real finding to strip");
    let full = serde_json::to_value(Some(entry)).unwrap();
    assert!(full["semantic_findings"][0]["suggested_fix"].is_string(), "fixture should have a fix to strip");

    let compact = entry_json_compact(Some(entry));
    assert!(compact["semantic_findings"][0].get("suggested_fix").is_none(), "got: {compact}");
    assert_eq!(compact["path"], "a.rs");
}

#[test]
fn entry_json_compact_handles_none() {
    assert!(entry_json_compact(None).is_null());
}

#[test]
fn query_looks_up_an_entry_by_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "pub fn f() {}\n").unwrap();
    let g: Graph = build(dir.path(), &Config::default()).unwrap();
    assert!(query(&g, "a.rs").is_some());
    assert!(query(&g, "missing.rs").is_none());
}

#[test]
fn build_twice_on_unchanged_tree_is_byte_identical() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "//! Doc.\npub fn f(x: Foo) -> Foo { todo!() }\n").unwrap();
    std::fs::write(dir.path().join("a.py"), "def g(x):\n    pass\n").unwrap();
    let g1 = build(dir.path(), &Config::default()).unwrap();
    store(dir.path(), &g1).unwrap();
    let bytes1 = std::fs::read(dir.path().join(".veneer/graph.toon")).unwrap();

    let g2 = build(dir.path(), &Config::default()).unwrap();
    store(dir.path(), &g2).unwrap();
    let bytes2 = std::fs::read(dir.path().join(".veneer/graph.toon")).unwrap();

    assert_eq!(bytes1, bytes2, "re-running build+store on an unchanged tree must be a no-op");
}
