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
