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
    Oxidation,
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
    /// LoC-budget exclusions: entries ending in '/' are root-relative
    /// directory path-prefixes ("docs/", ".lake/" — checked before the dot
    /// rule, so dot-prefixed directories work as prefixes, not suffixes);
    /// entries starting with '.' (and not ending in '/') are extension
    /// suffixes (".json"); all other entries are plain path prefixes.
    /// Excluded files still participate in sealing, idempotency, and the
    /// tree hash — only the budget check skips them. Note: non-'/' prefix
    /// matching is a plain string prefix, so "docs" also matches "docsmore/";
    /// include the trailing '/' for directory entries.
    #[serde(default)]
    pub loc_exclude: Vec<String>,
    #[serde(default)]
    pub oxidize: crate::oxidize::OxidizeConfig,
}

fn default_soft() -> u32 { 500 }
fn default_hard() -> u32 { 1000 }

impl Default for Config {
    fn default() -> Config {
        Config {
            loc_soft: 500, loc_hard: 1000, modules: Vec::new(), loc_exclude: Vec::new(),
            oxidize: crate::oxidize::OxidizeConfig::default(),
        }
    }
}

/// Load `.veneer/config.toml` under `root`. Absent file → defaults.
/// Malformed TOML → a Protocol finding (silent fallback would undermine
/// the determinism contract).
pub fn load_config(root: &Path) -> Result<Config, Finding> {
    let p = root.join(".veneer/config.toml");
    let Ok(raw) = std::fs::read_to_string(p) else {
        return Ok(Config::default());
    };
    toml::from_str(&raw).map_err(|e| {
        Finding::error(
            Law::Protocol,
            ".veneer/config.toml",
            None,
            &format!("malformed config: {e}"),
            Some("fix the TOML or delete the file to use defaults"),
        )
    })
}

use std::collections::BTreeMap;
use std::path::PathBuf;

const SKIP_DIRS: &[&str] = &[".git", ".veneer", "target", "node_modules", ".claude", ".agents"];
/// Generated lockfiles are not first-principles modules; skip them from the
/// walk. Suffix match covers Cargo.lock, yarn.lock, Gemfile.lock, etc.; the
/// exact names cover lockfiles that don't end in ".lock".
const SKIP_FILE_SUFFIXES: &[&str] = &[".lock"];
const SKIP_FILE_NAMES: &[&str] = &["package-lock.json", "pnpm-lock.yaml"];

/// All regular files under root, sorted, skipping VCS/build/harness dirs and
/// generated lockfiles. Deterministic order ⇒ deterministic findings and tree
/// hashes. Symlinks are not followed: symlinked directories and symlinked files
/// are both skipped, preventing cycles caused by symlinks to ancestor directories.
pub fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let ft = entry.file_type().ok();
            if ft.map_or(false, |t| t.is_dir()) && !SKIP_DIRS.contains(&name.as_str()) {
                stack.push(p);
            } else if ft.map_or(false, |t| t.is_file())
                && !SKIP_FILE_SUFFIXES.iter().any(|s| name.ends_with(s))
                && !SKIP_FILE_NAMES.contains(&name.as_str())
            {
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

/// True when `path` (root-relative, '/'-separated) matches a `loc_exclude`
/// entry. Entries ending in '/' are directory path-prefixes (checked first,
/// so a dot-prefixed directory like ".lake/" is a prefix match, not an
/// extension suffix); entries starting with '.' are extension suffixes; all
/// other entries are path prefixes. Blank entries are inert.
fn is_loc_excluded(path: &str, cfg: &Config) -> bool {
    cfg.loc_exclude.iter().any(|pat| {
        let pat = pat.trim();
        if pat.is_empty() {
            false
        } else if pat.ends_with('/') {
            path.starts_with(pat)
        } else if pat.starts_with('.') {
            path.ends_with(pat)
        } else {
            path.starts_with(pat)
        }
    })
}

/// Law 2 (first-principles modules): a module is a file; Warning above the
/// soft bound, Error above the hard bound. Non-UTF8 files are not modules.
pub fn check_module_budget(root: &Path, files: &[PathBuf], cfg: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    for f in files {
        let path = rel(root, f);
        if is_loc_excluded(&path, cfg) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        let n = loc(&text);
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

pub use crate::patch::{apply_patch, parse_patch, DiffLine, FilePatch, Hunk, Patch};

use crate::kernel;

/// FNV-1a 64-bit. Deterministic content hash — an equality witness, not a
/// security primitive.
pub fn fnv64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Canonical hash of a tree: hash over sorted (path NUL content NUL) pairs.
pub fn tree_hash(tree: &BTreeMap<String, String>) -> u64 {
    let mut buf = Vec::new();
    for (path, contents) in tree {
        buf.extend_from_slice(path.as_bytes());
        buf.push(0);
        buf.extend_from_slice(contents.as_bytes());
        buf.push(0);
    }
    fnv64(&buf)
}

/// Read the working tree (UTF-8 files only) into path → contents.
pub fn read_tree(root: &Path) -> BTreeMap<String, String> {
    let mut t = BTreeMap::new();
    for f in walk_files(root) {
        if let Ok(text) = std::fs::read_to_string(&f) {
            t.insert(rel(root, &f), text);
        }
    }
    t
}

/// The hash that witnesses a clean check: raw config bytes + tree hash.
/// Including the config means editing limits or module declarations
/// invalidates a recorded clean check — the verdict could differ under the
/// new rules. (The walker skips `.veneer/`, so the tree hash alone cannot
/// see config edits.)
pub fn clean_hash(root: &Path) -> u64 {
    let cfg_raw = std::fs::read_to_string(root.join(".veneer/config.toml")).unwrap_or_default();
    let mut buf = cfg_raw.into_bytes();
    buf.push(0);
    buf.extend_from_slice(&tree_hash(&read_tree(root)).to_be_bytes());
    fnv64(&buf)
}

/// Law 3 (idempotency): applying a patch twice must equal applying it once.
/// T1 = apply(T0, p); T2 = apply(T1, p) or T1 if re-application fails cleanly.
/// The judgement hash(T1) ≐ hash(T2) runs through the kernel: hashes are
/// lifted to canonical forms and compared with check_eq (basis §IV).
pub fn check_idempotency(tree0: &BTreeMap<String, String>, patch_text: &str) -> Vec<Finding> {
    let patch = match parse_patch(patch_text) {
        Ok(p) => p,
        Err(e) => {
            return vec![Finding::error(
                Law::Protocol,
                "<patch>",
                None,
                &format!("unparseable patch: {e}"),
                Some("emit a strict unified diff"),
            )]
        }
    };
    let t1 = match apply_patch(tree0, &patch) {
        Ok(t) => t,
        Err(e) => {
            return vec![Finding::error(
                Law::Protocol,
                "<patch>",
                None,
                &format!("patch does not apply: {e}"),
                Some("rebase the patch against the current tree"),
            )]
        }
    };
    let t2 = apply_patch(&t1, &patch).unwrap_or_else(|_| t1.clone());
    let h1 = tree_hash(&t1).to_be_bytes();
    let h2 = tree_hash(&t2).to_be_bytes();
    let mut gas = 1_000_000; // ~100x the worst case for two 8-byte encodings
    let equal = kernel::check_eq(
        &kernel::bytes_type(8),
        &kernel::from_bytes(&h1),
        &kernel::from_bytes(&h2),
        &mut gas,
    )
    .unwrap_or(false);
    if equal {
        Vec::new()
    } else {
        vec![Finding::error(
            Law::Idempotency,
            "<patch>",
            None,
            "patch is not idempotent: applying twice diverges from applying once",
            Some("anchor insertions to unique context so re-application fails cleanly"),
        )]
    }
}

/// Law 3 (sealing): files outside a declared module must not reference the
/// module's internal files. Language-agnostic textual check: a reference is
/// any occurrence of "<module-dir-name>/<internal-file-stem>". Modules are
/// declared in .veneer/config.toml; nothing declared, nothing to seal.
pub fn check_sealing(root: &Path, files: &[PathBuf], cfg: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    for m in &cfg.modules {
        let mod_dir = root.join(&m.path);
        let dir_name = Path::new(&m.path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| m.path.clone());
        let internal: Vec<String> = files
            .iter()
            .filter(|f| f.starts_with(&mod_dir))
            .filter_map(|f| f.file_name().map(|n| n.to_string_lossy().to_string()))
            .filter(|name| !m.public.iter().any(|p| p == name))
            .filter_map(|name| name.rsplit_once('.').map(|(stem, _)| stem.to_string()))
            .collect();
        for f in files.iter().filter(|f| !f.starts_with(&mod_dir)) {
            let Ok(text) = std::fs::read_to_string(f) else { continue };
            let lines: Vec<&str> = text.lines().collect();
            for stem in &internal {
                let needle = format!("{dir_name}/{stem}");
                if let Some(idx) = lines.iter().position(|l| l.contains(&needle)) {
                    findings.push(Finding::error(
                        Law::ModuleSealing,
                        &rel(root, f),
                        Some(idx as u32 + 1),
                        &format!(
                            "references internal file '{stem}' of sealed module '{}'",
                            m.path
                        ),
                        Some("depend on the module's declared public surface instead"),
                    ));
                }
            }
        }
    }
    findings
}

/// Findings serialized without `suggested_fix` — the token-lean trace for
/// agent consumption (per-law fixes are documented in the skill). The full
/// schema is unchanged; this is an output view, not a data-model change.
pub fn findings_json_compact(findings: &[Finding]) -> String {
    let mut v = serde_json::to_value(findings).expect("findings serialization is infallible");
    if let Some(arr) = v.as_array_mut() {
        for f in arr {
            if let Some(m) = f.as_object_mut() {
                m.remove("suggested_fix");
            }
        }
    }
    v.to_string()
}

/// The check orchestrator used by CLI, MCP, and intent execution.
/// paths: restrict budget check to these (empty = whole tree).
/// diff: also run the idempotency law against the on-disk tree.
pub fn run_checks(
    root: &Path,
    paths: &[PathBuf],
    diff: Option<&str>,
    cfg: &Config,
) -> Vec<Finding> {
    let all = walk_files(root);
    let budget_files: Vec<PathBuf> = if paths.is_empty() {
        all.clone()
    } else {
        all.iter().filter(|f| paths.iter().any(|p| f.starts_with(root.join(p)))).cloned().collect()
    };
    let mut findings = check_module_budget(root, &budget_files, cfg);
    findings.extend(check_sealing(root, &all, cfg));
    if let Some(d) = diff {
        findings.extend(check_idempotency(&read_tree(root), d));
    }
    findings
}
