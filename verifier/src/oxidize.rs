//! Oxidation: transient Rust type-check of an agent-authored shadow skeleton.
//! The shadow is compiled against a persistent scratch crate (.veneer/oxidize/);
//! rustc diagnostics become Law::Oxidation findings; the shadow is never
//! retained as an artifact (it is overwritten on the next run, and the scratch
//! crate is gitignored and skipped by the walker). A second verifier beside the
//! CTT kernel — rustc judges type and
//! ownership (affine) coherence (basis §VII). Errors are data: every failure is
//! a Finding, never a panic.

use crate::laws::{Finding, Law, Severity};
use serde::Deserialize;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SHADOW_LABEL: &str = "<shadow>";
const OX_FIX: &str =
    "fix the type/ownership story of the proposed code (keep the shadow faithful to it), then re-oxidize";

/// Oxidation settings (the `[oxidize]` TOML section, surfaced on `Config`).
/// Timeouts are wall-clock caps on the scratch-crate cargo run; `edition`
/// selects the scratch crate's Rust edition. (A `deps` pass-through is reserved
/// for a later iteration and is not yet a field.)
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OxidizeConfig {
    #[serde(default = "default_edition")]
    pub edition: String,
    #[serde(default = "default_steady")]
    pub steady_timeout_ms: u64,
    #[serde(default = "default_cold")]
    pub cold_timeout_ms: u64,
}

fn default_edition() -> String { "2021".into() }
fn default_steady() -> u64 { 2000 }
fn default_cold() -> u64 { 30000 }

impl Default for OxidizeConfig {
    fn default() -> OxidizeConfig {
        OxidizeConfig {
            edition: default_edition(),
            steady_timeout_ms: default_steady(),
            cold_timeout_ms: default_cold(),
        }
    }
}

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

const SCRATCH_DIR: &str = ".veneer/oxidize";

/// Why a cargo run did not produce diagnostics. Errors as data.
enum RunErr {
    Spawn(String),
    Timeout(u64),
}

fn cargo_toml(edition: &str) -> String {
    format!(
        "[package]\nname = \"veneer_oxidize_scratch\"\nversion = \"0.0.0\"\nedition = \"{edition}\"\n\n[lib]\npath = \"src/lib.rs\"\n"
    )
}

/// Ensure the scratch crate exists; on first creation, cold-prime it (one-time
/// generous budget) so the first real oxidation is warm. Idempotent.
fn scaffold(root: &Path, cfg: &OxidizeConfig) -> Result<(), RunErr> {
    let dir = root.join(SCRATCH_DIR);
    let manifest = dir.join("Cargo.toml");
    if manifest.exists() {
        return Ok(());
    }
    let src = dir.join("src");
    std::fs::create_dir_all(&src).map_err(|e| RunErr::Spawn(e.to_string()))?;
    // Write the source first; the manifest is written last so that its presence
    // (the idempotency sentinel above) implies the source already exists — a
    // kill mid-scaffold cannot leave a manifest without its lib.rs.
    std::fs::write(src.join("lib.rs"), "").map_err(|e| RunErr::Spawn(e.to_string()))?;
    std::fs::write(&manifest, cargo_toml(&cfg.edition)).map_err(|e| RunErr::Spawn(e.to_string()))?;
    run_check(&manifest, cfg.cold_timeout_ms)?; // cold prime; discard output
    Ok(())
}

fn write_shadow(root: &Path, shadow: &str) -> Result<(), RunErr> {
    let p = root.join(SCRATCH_DIR).join("src/lib.rs");
    std::fs::write(p, shadow).map_err(|e| RunErr::Spawn(e.to_string()))
}

/// Run `cargo check --message-format=json` with a wall-clock cap. Captures
/// stdout (the JSON stream); cargo's human progress on stderr is discarded.
/// A reader thread drains stdout so a full pipe can never deadlock the wait
/// loop.
fn run_check(manifest: &Path, timeout_ms: u64) -> Result<String, RunErr> {
    let mut child = Command::new("cargo")
        .arg("check")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--message-format=json")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| RunErr::Spawn(e.to_string()))?;
    let mut out = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err(RunErr::Timeout(timeout_ms));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(RunErr::Spawn(e.to_string())),
        }
    }
    Ok(reader.join().unwrap_or_default())
}

fn run_err_finding(e: RunErr) -> Finding {
    match e {
        RunErr::Spawn(msg) => Finding::error(
            Law::Protocol,
            SHADOW_LABEL,
            None,
            &format!("oxidation could not run cargo: {msg}"),
            Some("ensure cargo is installed and .veneer/oxidize is writable"),
        ),
        RunErr::Timeout(ms) => Finding::error(
            Law::Protocol,
            SHADOW_LABEL,
            None,
            &format!("oxidation timed out after {ms}ms"),
            Some("simplify the shadow or raise [oxidize] steady_timeout_ms"),
        ),
    }
}

/// Oxidize: compile the agent-authored shadow against the scratch crate and
/// return Oxidation findings (empty = type-coherent). Run failures (cargo
/// missing, timeout, unwritable scratch) are Protocol findings — outside the
/// deterministic envelope.
pub fn oxidize(root: &Path, shadow: &str, cfg: &OxidizeConfig) -> Vec<Finding> {
    if let Err(e) = scaffold(root, cfg) {
        return vec![run_err_finding(e)];
    }
    if let Err(e) = write_shadow(root, shadow) {
        return vec![run_err_finding(e)];
    }
    let manifest = root.join(SCRATCH_DIR).join("Cargo.toml");
    match run_check(&manifest, cfg.steady_timeout_ms) {
        Ok(stdout) => parse_diagnostics(&stdout),
        Err(e) => vec![run_err_finding(e)],
    }
}
