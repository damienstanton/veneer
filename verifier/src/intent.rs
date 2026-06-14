//! The AgentIntent ADT (the veneer interaction protocol): the single typed
//! entry path shared by the CLI and MCP surfaces.

use crate::laws::{loc, run_checks, Config, Finding, Law};
use crate::state::{set_phase, Phase};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum AgentIntent {
    /// Read a signature/interface; rejected if it would blow the context budget.
    ExpandContext { query: String },
    /// Propose a unified diff; runs the laws against it.
    ProposeDiff { patch: String },
    /// Request the ship gate.
    Conclude { summary: String },
    /// Type-check an agent-authored Rust shadow skeleton (oxidation).
    Oxidize { shadow: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Context(String),
    Findings(Vec<Finding>),
    Concluded(String),
}

/// Parse an intent; out-of-protocol input is a Protocol finding, never a crash.
pub fn parse_intent(s: &str) -> Result<AgentIntent, Finding> {
    serde_json::from_str(s).map_err(|e| {
        Finding::error(
            Law::Protocol,
            "<intent>",
            None,
            &format!("malformed intent: {e}"),
            Some(r#"emit {"intent":"expand_context"|"propose_diff"|"conclude"|"oxidize", ...}"#),
        )
    })
}

/// Execute an intent against the project at root.
pub fn execute(root: &Path, intent: AgentIntent, cfg: &Config) -> Outcome {
    match intent {
        AgentIntent::ExpandContext { query } => {
            let p = root.join(&query);
            match std::fs::read_to_string(&p) {
                Err(e) => Outcome::Findings(vec![Finding::error(
                    Law::Protocol,
                    &query,
                    None,
                    &format!("cannot read: {e}"),
                    None,
                )]),
                Ok(text) if loc(&text) > cfg.loc_hard => Outcome::Findings(vec![Finding::error(
                    Law::Protocol,
                    &query,
                    None,
                    &format!(
                        "file is {} LoC, exceeds the context budget ({})",
                        loc(&text),
                        cfg.loc_hard
                    ),
                    Some("read the module's signature/public surface instead of its implementation"),
                )]),
                Ok(text) => Outcome::Context(text),
            }
        }
        AgentIntent::ProposeDiff { patch } => {
            Outcome::Findings(run_checks(root, &[], Some(&patch), cfg))
        }
        AgentIntent::Conclude { summary } => match set_phase(root, Phase::Ship, &[]) {
            Ok(_) => Outcome::Concluded(summary),
            Err(f) => Outcome::Findings(vec![f]),
        },
        AgentIntent::Oxidize { shadow } => {
            Outcome::Findings(crate::oxidize::oxidize(root, &shadow, &cfg.oxidize))
        }
    }
}
