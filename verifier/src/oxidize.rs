//! Oxidation: transient Rust type-check of an agent-authored shadow skeleton.
//! The shadow is compiled against a persistent scratch crate (.veneer/oxidize/);
//! rustc diagnostics become Law::Oxidation findings, then the shadow is
//! discarded. A second verifier beside the CTT kernel — rustc judges type and
//! ownership (affine) coherence (basis §VII). Errors are data: every failure is
//! a Finding, never a panic.

use crate::laws::{Finding, Law, Severity};
use serde::Deserialize;

const SHADOW_LABEL: &str = "<shadow>";
const OX_FIX: &str =
    "fix the type/ownership story of the proposed code (keep the shadow faithful to it), then re-oxidize";

/// One line of `cargo check --message-format=json`. Unknown fields ignored.
#[derive(Deserialize)]
struct CargoLine {
    reason: String,
    message: Option<Diag>,
}

#[derive(Deserialize)]
struct Diag {
    message: String,
    level: String,
    #[serde(default)]
    spans: Vec<Span>,
}

#[derive(Deserialize)]
struct Span {
    line_start: u32,
    #[serde(default)]
    is_primary: bool,
}

/// Map a cargo JSON stream to sorted, deterministic Oxidation findings.
/// Only error/warning diagnostics survive; notes/help/failure-notes and the
/// "aborting due to …" summary are dropped.
pub fn parse_diagnostics(stdout: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(cl) = serde_json::from_str::<CargoLine>(line) else { continue };
        if cl.reason != "compiler-message" {
            continue;
        }
        let Some(diag) = cl.message else { continue };
        let severity = match diag.level.as_str() {
            "warning" => Severity::Warning,
            l if l.starts_with("error") => Severity::Error,
            _ => continue,
        };
        if diag.spans.is_empty() && diag.message.starts_with("aborting due to") {
            continue;
        }
        let line_no = diag
            .spans
            .iter()
            .find(|s| s.is_primary)
            .or_else(|| diag.spans.first())
            .map(|s| s.line_start);
        let f = match severity {
            Severity::Error => {
                Finding::error(Law::Oxidation, SHADOW_LABEL, line_no, &diag.message, Some(OX_FIX))
            }
            Severity::Warning => {
                Finding::warning(Law::Oxidation, SHADOW_LABEL, line_no, &diag.message, Some(OX_FIX))
            }
        };
        findings.push(f);
    }
    findings.sort_by(|a, b| {
        a.location.line.cmp(&b.location.line).then_with(|| a.message.cmp(&b.message))
    });
    findings
}
