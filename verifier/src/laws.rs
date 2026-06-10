//! The three laws as deterministic checks, plus the Finding value type that
//! every veneer surface (CLI, MCP, state, intent) reports through.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Law {
    ModuleBudget,
    ModuleSealing,
    Idempotency,
    Protocol,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// Errors are data: every veneer failure is a Finding, never a panic.
/// Structural equality; serialization is the machine-readable trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub law: Law,
    pub severity: Severity,
    pub location: Location,
    pub message: String,
    pub suggested_fix: Option<String>,
}

impl Finding {
    pub fn error(law: Law, path: &str, line: Option<u32>, msg: &str, fix: Option<&str>) -> Finding {
        Finding {
            law,
            severity: Severity::Error,
            location: Location { path: path.into(), line },
            message: msg.into(),
            suggested_fix: fix.map(Into::into),
        }
    }
    pub fn warning(law: Law, path: &str, line: Option<u32>, msg: &str, fix: Option<&str>) -> Finding {
        Finding {
            law,
            severity: Severity::Warning,
            location: Location { path: path.into(), line },
            message: msg.into(),
            suggested_fix: fix.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ModuleDecl {
    pub path: String,
    #[serde(default)]
    pub public: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Config {
    #[serde(default = "default_soft")]
    pub loc_soft: u32,
    #[serde(default = "default_hard")]
    pub loc_hard: u32,
    #[serde(default)]
    pub modules: Vec<ModuleDecl>,
}

fn default_soft() -> u32 { 500 }
fn default_hard() -> u32 { 1000 }

impl Default for Config {
    fn default() -> Config {
        Config { loc_soft: 500, loc_hard: 1000, modules: Vec::new() }
    }
}

/// Load `.veneer/config.toml` under `root`; absent or malformed → defaults.
/// (A malformed config is reported as a Protocol finding by the CLI layer.)
pub fn load_config(root: &Path) -> Config {
    let p = root.join(".veneer/config.toml");
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}
