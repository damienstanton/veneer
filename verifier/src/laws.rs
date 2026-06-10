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

use std::collections::BTreeMap;
use std::path::PathBuf;

const SKIP_DIRS: &[&str] = &[".git", ".veneer", "target", "node_modules", ".claude", ".agents"];

/// All regular files under root, sorted, skipping VCS/build/harness dirs.
/// Deterministic order ⇒ deterministic findings and tree hashes.
pub fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push(p);
                }
            } else if p.is_file() {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Non-blank line count: the measurable proxy for first-principles size.
pub fn loc(text: &str) -> u32 {
    text.lines().filter(|l| !l.trim().is_empty()).count() as u32
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

/// Law 2 (first-principles modules): a module is a file; Warning above the
/// soft bound, Error above the hard bound. Non-UTF8 files are not modules.
pub fn check_module_budget(root: &Path, files: &[PathBuf], cfg: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        let n = loc(&text);
        let path = rel(root, f);
        let fix = "split into first-principles modules (target ~500 LoC)";
        if n > cfg.loc_hard {
            findings.push(Finding::error(
                Law::ModuleBudget,
                &path,
                None,
                &format!("module is {n} LoC, exceeds hard bound {}", cfg.loc_hard),
                Some(fix),
            ));
        } else if n > cfg.loc_soft {
            findings.push(Finding::warning(
                Law::ModuleBudget,
                &path,
                None,
                &format!("module is {n} LoC, above soft bound {}", cfg.loc_soft),
                Some(fix),
            ));
        }
    }
    findings
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    Ctx(String),
    Add(String),
    Del(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: usize, // 1-based; 0 for new files
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    pub path: String,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub files: Vec<FilePatch>,
}

fn strip_prefix_ab(p: &str) -> String {
    p.trim().trim_start_matches("a/").trim_start_matches("b/").to_string()
}

/// Parse a unified diff. Strict enough for agent-proposed patches; any
/// deviation is an Err(String) the caller wraps as a Protocol finding.
pub fn parse_patch(s: &str) -> Result<Patch, String> {
    let mut files: Vec<FilePatch> = Vec::new();
    let mut lines = s.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(_old) = line.strip_prefix("--- ") {
            let new = lines
                .next()
                .and_then(|l| l.strip_prefix("+++ "))
                .ok_or("expected +++ after ---")?;
            let path = strip_prefix_ab(new);
            let mut hunks = Vec::new();
            while let Some(h) = lines.peek().and_then(|l| l.strip_prefix("@@ ")) {
                let header = h.split(" @@").next().ok_or("malformed hunk header")?;
                let old_part = header.split(' ').next().ok_or("malformed hunk header")?;
                let old_start: usize = old_part
                    .trim_start_matches('-')
                    .split(',')
                    .next()
                    .ok_or("malformed hunk header")?
                    .parse()
                    .map_err(|_| "malformed hunk start".to_string())?;
                lines.next();
                let mut body = Vec::new();
                while let Some(l) = lines.peek() {
                    match l.chars().next() {
                        Some(' ') => body.push(DiffLine::Ctx(l[1..].to_string())),
                        Some('+') if !l.starts_with("+++") => body.push(DiffLine::Add(l[1..].to_string())),
                        Some('-') if !l.starts_with("---") => body.push(DiffLine::Del(l[1..].to_string())),
                        Some('\\') => {} // "\ No newline at end of file"
                        _ => break,
                    }
                    lines.next();
                }
                hunks.push(Hunk { old_start, lines: body });
            }
            if hunks.is_empty() {
                return Err(format!("no hunks for {path}"));
            }
            files.push(FilePatch { path, hunks });
        }
    }
    if files.is_empty() {
        return Err("no file patches found".into());
    }
    Ok(Patch { files })
}

/// Apply a patch to an in-memory tree (path → contents). Pure: returns a new
/// tree. Strict context matching; any mismatch is a clean Err.
pub fn apply_patch(
    tree: &BTreeMap<String, String>,
    patch: &Patch,
) -> Result<BTreeMap<String, String>, String> {
    let mut out = tree.clone();
    for fp in &patch.files {
        let old: Vec<String> = out
            .get(&fp.path)
            .map(|c| c.lines().map(String::from).collect())
            .unwrap_or_default();
        let mut new_lines: Vec<String> = Vec::new();
        let mut cursor = 0usize; // index into old
        for h in &fp.hunks {
            let start = h.old_start.saturating_sub(1);
            if start < cursor || start > old.len() {
                return Err(format!("{}: hunk start out of order", fp.path));
            }
            new_lines.extend_from_slice(&old[cursor..start]);
            cursor = start;
            for dl in &h.lines {
                match dl {
                    DiffLine::Ctx(s) | DiffLine::Del(s) => {
                        if old.get(cursor) != Some(s) {
                            return Err(format!(
                                "{}: context mismatch at line {}",
                                fp.path,
                                cursor + 1
                            ));
                        }
                        if matches!(dl, DiffLine::Ctx(_)) {
                            new_lines.push(s.clone());
                        }
                        cursor += 1;
                    }
                    DiffLine::Add(s) => new_lines.push(s.clone()),
                }
            }
        }
        new_lines.extend_from_slice(&old[cursor..]);
        let mut contents = new_lines.join("\n");
        if !contents.is_empty() {
            contents.push('\n');
        }
        out.insert(fp.path.clone(), contents);
    }
    Ok(out)
}
