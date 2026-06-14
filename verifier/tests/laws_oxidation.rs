use veneer::laws::{Config, Law};
use veneer::oxidize::OxidizeConfig;

#[test]
fn oxidation_law_serializes_snake_case() {
    let j = serde_json::to_string(&Law::Oxidation).unwrap();
    assert_eq!(j, "\"oxidation\"");
}

#[test]
fn oxidize_config_defaults() {
    let c = OxidizeConfig::default();
    assert_eq!(c.edition, "2021");
    assert_eq!(c.steady_timeout_ms, 2000);
    assert_eq!(c.cold_timeout_ms, 30000);
}

#[test]
fn config_parses_oxidize_section() {
    let toml = "loc_soft = 500\n[oxidize]\nsteady_timeout_ms = 1500\n";
    let c: Config = toml::from_str(toml).unwrap();
    assert_eq!(c.oxidize.steady_timeout_ms, 1500);
    assert_eq!(c.oxidize.cold_timeout_ms, 30000); // default fills the gap
}

#[test]
fn config_without_oxidize_section_uses_defaults() {
    let c: Config = toml::from_str("loc_soft = 500\n").unwrap();
    assert_eq!(c.oxidize.steady_timeout_ms, 2000);
}
