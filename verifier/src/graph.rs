//! Codebase knowledge graph: a cached, token-cheap index of per-file public
//! signatures, doc summaries, LoC, complexity, and (Rust files only) real
//! semantic findings lifted through the existing oxidize pipeline. Heuristic,
//! not an AST — honest about being structural, not a parser.

use crate::laws::{self, Config, Finding, Law};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// One file's extracted facts. `canonical_form` and `semantic_findings` are
/// populated for Rust files only: the generic-erased shadow lifted from
/// `signatures` (see `lift_shadow`) and the real rustc-grade findings from
/// running it through the existing `oxidize::oxidize()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEntry {
    pub path: String,
    pub signatures: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_summary: Option<String>,
    pub loc: u32,
    pub complexity: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_form: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic_findings: Vec<Finding>,
}

/// The full graph: every walked file's entry, keyed by root-relative path,
/// plus the tree hash it was built from — the staleness witness `is_stale`
/// compares against the current tree. Deliberately independent of
/// `laws::clean_hash`/the ship gate: a stale graph is a query-time warning,
/// never a build failure or a blocked transition.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Graph {
    pub entries: BTreeMap<String, GraphEntry>,
    pub built_from: u64,
}

/// True when the source tree has changed since `g` was built.
pub fn is_stale(g: &Graph, root: &Path) -> bool {
    g.built_from != laws::tree_hash(&laws::read_tree(root))
}

/// Look up one file's entry by its root-relative path.
pub fn query<'a>(g: &'a Graph, target: &str) -> Option<&'a GraphEntry> {
    g.entries.get(target)
}

/// `entry` rendered as JSON with `suggested_fix` stripped from any nested
/// `semantic_findings` — the same token-lean trim `--compact`/MCP already
/// apply to a top-level findings array, applied one level deeper. `None`
/// renders as JSON `null`.
pub fn entry_json_compact(entry: Option<&GraphEntry>) -> serde_json::Value {
    let mut v = serde_json::to_value(entry).expect("graph entry serialization is infallible");
    if let Some(findings) = v.get_mut("semantic_findings").and_then(|f| f.as_array_mut()) {
        for f in findings {
            if let Some(m) = f.as_object_mut() {
                m.remove("suggested_fix");
            }
        }
    }
    v
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

/// Build the graph by walking the tree (the same walker `veneer check` uses,
/// so the same skip list applies — `.veneer/`, `target/`, etc. are never
/// entered) and extracting each file's facts. Non-UTF8 files are skipped,
/// consistent with the module-budget check. One `cargo check` invocation per
/// Rust file (via `oxidize::oxidize`) — deliberately off the hot path; this
/// runs only on explicit `veneer graph build`, never inside `veneer check`.
pub fn build(root: &Path, cfg: &Config) -> Result<Graph, Finding> {
    let mut entries = BTreeMap::new();
    for f in laws::walk_files(root) {
        let path = rel(root, &f);
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let signatures = extract_signatures(&path, &text);
        let doc_summary = extract_doc_summary(&path, &text);
        let loc = laws::loc(&text);
        let complexity = complexity_score(&text);
        let (canonical_form, semantic_findings) = if path.ends_with(".rs") {
            let shadow = lift_shadow(&signatures);
            // oxidize labels every finding's location with the generic
            // "<shadow>" (it has no notion of the real source file); rewrite
            // it to this entry's path so a finding is attributable on its
            // own, not just by which entry happens to hold it. The line
            // number is cleared, not carried over: it indexes the synthetic
            // shadow's own layout, which has no correspondence to line
            // numbers in the real file — keeping it would pair a real path
            // with an unrelated line, false precision worse than none.
            let mut findings = crate::oxidize::oxidize(root, &shadow, &cfg.oxidize);
            for f in &mut findings {
                f.location.path = path.clone();
                f.location.line = None;
            }
            (Some(shadow), findings)
        } else {
            (None, Vec::new())
        };
        entries.insert(path.clone(), GraphEntry { path, signatures, doc_summary, loc, complexity, canonical_form, semantic_findings });
    }
    let built_from = laws::tree_hash(&laws::read_tree(root));
    Ok(Graph { entries, built_from })
}

fn graph_path(root: &Path) -> std::path::PathBuf {
    root.join(".veneer/graph.toon")
}

/// On-disk document: the logical graph plus its integrity hash. `built_from`
/// is encoded as a decimal string (not a bare TOON number) because it's a
/// full-width FNV-1a u64, frequently above `i64::MAX` — the same fix applied
/// to `state.rs`'s `last_clean_check` after it broke the ship gate.
#[derive(Serialize, Deserialize)]
struct OnDisk {
    entries: BTreeMap<String, OnDiskEntry>,
    #[serde(with = "hash_as_string")]
    built_from: u64,
    hash: String,
}

/// Percent-encodes any byte ≥ 0x80 (plus the `%` marker itself, for an
/// unambiguous round-trip) so free text is pure ASCII before it reaches
/// toon_rust. Necessary: toon-rust 0.1.3's decoder miscomputes offsets across
/// multi-byte UTF-8 characters — a single em dash in a doc comment corrupts
/// parsing of every subsequent line in the document, and some multi-byte
/// sequences (e.g. CJK text) make it panic on a char-boundary slice outright.
/// Plain ASCII (including embedded `\n`, which toon_rust's own escaping
/// already round-trips correctly) is left untouched. Confined to storage —
/// `GraphEntry`'s public shape, and all JSON the agent sees, carry normal
/// UTF-8 text; only the wire format is percent-encoded.
mod ascii_safe {
    pub fn encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for b in s.bytes() {
            if b == b'%' || b >= 0x80 {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            } else {
                out.push(b as char);
            }
        }
        out
    }

    pub fn decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            let hex_byte = if bytes[i] == b'%' && i + 3 <= bytes.len() {
                std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
            } else {
                None
            };
            match hex_byte {
                Some(b) => {
                    out.push(b);
                    i += 3;
                }
                None => {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
        }
        String::from_utf8(out).unwrap_or_default()
    }
}

/// `Finding` with `location` flattened to two scalar fields, used only for
/// storage. TOON's tabular-array encoder requires every field of a uniformly-
/// keyed object array to be a primitive scalar; `Finding`'s nested
/// `location: {path, line}` is not, so encoding `Vec<Finding>` as-is fails
/// whenever a file has any real findings (the common, non-empty case — see
/// toon-rust#`encode_tabular_array_rows`). Confined to the wire format:
/// `GraphEntry`'s public shape, and all JSON the agent sees, keep `Finding`'s
/// normal nested `location`.
#[derive(Serialize, Deserialize)]
struct FlatFinding {
    law: Law,
    severity: crate::laws::Severity,
    location_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    location_line: Option<u32>,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    suggested_fix: Option<String>,
}

impl From<&Finding> for FlatFinding {
    fn from(f: &Finding) -> FlatFinding {
        FlatFinding {
            law: f.law.clone(),
            severity: f.severity.clone(),
            location_path: ascii_safe::encode(&f.location.path),
            location_line: f.location.line,
            message: ascii_safe::encode(&f.message),
            suggested_fix: f.suggested_fix.as_deref().map(ascii_safe::encode),
        }
    }
}

impl From<FlatFinding> for Finding {
    fn from(f: FlatFinding) -> Finding {
        Finding {
            law: f.law,
            severity: f.severity,
            location: crate::laws::Location {
                path: ascii_safe::decode(&f.location_path),
                line: f.location_line,
            },
            message: ascii_safe::decode(&f.message),
            suggested_fix: f.suggested_fix.as_deref().map(ascii_safe::decode),
        }
    }
}

/// `GraphEntry` with `semantic_findings` stored as `FlatFinding`, for the
/// same reason — see `FlatFinding`.
#[derive(Serialize, Deserialize)]
struct OnDiskEntry {
    path: String,
    signatures: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    doc_summary: Option<String>,
    loc: u32,
    complexity: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    canonical_form: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    semantic_findings: Vec<FlatFinding>,
}

impl From<&GraphEntry> for OnDiskEntry {
    fn from(e: &GraphEntry) -> OnDiskEntry {
        OnDiskEntry {
            path: ascii_safe::encode(&e.path),
            signatures: e.signatures.iter().map(|s| ascii_safe::encode(s)).collect(),
            doc_summary: e.doc_summary.as_deref().map(ascii_safe::encode),
            loc: e.loc,
            complexity: e.complexity,
            canonical_form: e.canonical_form.as_deref().map(ascii_safe::encode),
            semantic_findings: e.semantic_findings.iter().map(FlatFinding::from).collect(),
        }
    }
}

impl From<OnDiskEntry> for GraphEntry {
    fn from(e: OnDiskEntry) -> GraphEntry {
        GraphEntry {
            path: ascii_safe::decode(&e.path),
            signatures: e.signatures.iter().map(|s| ascii_safe::decode(s)).collect(),
            doc_summary: e.doc_summary.as_deref().map(ascii_safe::decode),
            loc: e.loc,
            complexity: e.complexity,
            canonical_form: e.canonical_form.as_deref().map(ascii_safe::decode),
            semantic_findings: e.semantic_findings.into_iter().map(Finding::from).collect(),
        }
    }
}

mod hash_as_string {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::String(s) => s.parse::<u64>().map_err(|_| D::Error::custom("invalid hash")),
            serde_json::Value::Number(n) => n.as_u64().ok_or_else(|| D::Error::custom("hash out of range")),
            _ => Err(D::Error::custom("hash must be a number or string")),
        }
    }
}

fn canonical_bytes(entries: &BTreeMap<String, GraphEntry>, built_from: u64) -> Vec<u8> {
    let mut buf = serde_json::to_vec(entries).expect("graph serialization is infallible");
    buf.extend_from_slice(&built_from.to_be_bytes());
    buf
}

/// Persist the graph as TOON with an embedded content-hash witness — its own
/// integrity check, independent of `.veneer/state.toon`. Crash-atomic
/// (temp-write then rename), mirroring `state::store`.
pub fn store(root: &Path, g: &Graph) -> std::io::Result<()> {
    std::fs::create_dir_all(root.join(".veneer"))?;
    let hash = format!("fnv:{:016x}", laws::fnv64(&canonical_bytes(&g.entries, g.built_from)));
    let entries: BTreeMap<String, OnDiskEntry> =
        g.entries.iter().map(|(k, v)| (ascii_safe::encode(k), v.into())).collect();
    let od = OnDisk { entries, built_from: g.built_from, hash };
    let body = toon_rust::to_string(&od).expect("graph serialization is infallible") + "\n";
    let tmp = graph_path(root).with_extension("toon.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, graph_path(root))
}

/// Load the graph; a genuinely missing file (`NotFound`) is the empty
/// default (never built yet — not corruption). Any other read error
/// (permission denied, etc.) surfaces as a Protocol finding rather than
/// being silently treated as "never built" — masking a real IO problem as
/// an empty graph would make it harder to diagnose, not easier. A malformed
/// file or content-hash mismatch is also a Protocol finding, never a panic.
pub fn load(root: &Path) -> Result<Graph, Finding> {
    let corrupt = |msg: &str| {
        Finding::error(Law::Protocol, ".veneer/graph.toon", None, msg, Some("run `veneer graph build` to regenerate"))
    };
    let raw = match std::fs::read_to_string(graph_path(root)) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Graph::default()),
        Err(e) => return Err(corrupt(&format!("cannot read graph file: {e}"))),
    };
    let od: OnDisk = toon_rust::from_str(&raw).map_err(|_| corrupt("graph file is not valid TOON"))?;
    let entries: BTreeMap<String, GraphEntry> =
        od.entries.into_iter().map(|(k, v)| (ascii_safe::decode(&k), v.into())).collect();
    let expect = format!("fnv:{:016x}", laws::fnv64(&canonical_bytes(&entries, od.built_from)));
    if od.hash != expect {
        return Err(corrupt("graph file content hash mismatch"));
    }
    Ok(Graph { entries, built_from: od.built_from })
}

/// Per-extension markers for "this line declares a public item." Heuristic:
/// a line is a signature candidate if it starts with one of these markers
/// (after trimming indentation). No marker table for an extension ⇒ no
/// signatures are extracted for that file (loc/complexity still apply).
fn markers_for(path: &str) -> &'static [&'static str] {
    if path.ends_with(".rs") {
        &["pub fn ", "pub struct ", "pub enum ", "pub trait ", "pub type "]
    } else if path.ends_with(".py") {
        &["def ", "class "]
    } else if path.ends_with(".ts") || path.ends_with(".tsx") || path.ends_with(".js") || path.ends_with(".jsx") {
        &["export function ", "export class ", "export const ", "export interface ", "export type "]
    } else {
        &[]
    }
}

/// Truncate a candidate line at its first `{`, trimming trailing whitespace —
/// the declaration without its body. Lines with no `{` (e.g. `pub struct Foo;`)
/// are returned trimmed as-is.
fn signature_only(line: &str) -> String {
    match line.find('{') {
        Some(i) => line[..i].trim_end().to_string(),
        None => line.trim_end().to_string(),
    }
}

pub use crate::graph_lift::lift_shadow;

const BRANCH_WORDS: &[&str] = &["if", "else", "elif", "for", "while", "match", "switch", "case", "catch"];

/// Cross-language branch-complexity heuristic: counts whole-word branch
/// keywords plus `&&`/`||` occurrences. Word-bounded (splits on non-
/// alphanumeric characters), so "forest" or "matches" are not mistaken for
/// "for"/"match". A presence count, not a real cyclomatic-complexity proof.
pub fn complexity_score(text: &str) -> u32 {
    let words = text.split(|c: char| !c.is_alphanumeric() && c != '_').filter(|w| BRANCH_WORDS.contains(w)).count();
    let ops = text.matches("&&").count() + text.matches("||").count();
    (words + ops) as u32
}

/// Leading file-level doc comment, by source convention: Rust `//!` lines,
/// a Python leading triple-quoted docstring, or a TS/JS leading `/** */`
/// block. None if the file doesn't open with one of these.
pub fn extract_doc_summary(path: &str, text: &str) -> Option<String> {
    if path.ends_with(".rs") {
        let lines: Vec<&str> = text
            .lines()
            .take_while(|l| l.trim_start().starts_with("//!"))
            .map(|l| l.trim_start().trim_start_matches("//!").trim_start())
            .collect();
        if lines.is_empty() { None } else { Some(lines.join("\n")) }
    } else if path.ends_with(".py") {
        let trimmed = text.trim_start();
        let quote = trimmed.get(0..3)?;
        if quote != "\"\"\"" && quote != "'''" {
            return None;
        }
        let rest = &trimmed[3..];
        let end = rest.find(quote)?;
        Some(rest[..end].trim().to_string())
    } else if path.ends_with(".ts") || path.ends_with(".tsx") || path.ends_with(".js") || path.ends_with(".jsx") {
        let trimmed = text.trim_start();
        if !trimmed.starts_with("/**") {
            return None;
        }
        let end = trimmed.find("*/")?;
        let body = &trimmed[2..end];
        let lines: Vec<String> = body
            .lines()
            .map(|l| l.trim().trim_start_matches('*').trim())
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        if lines.is_empty() { None } else { Some(lines.join("\n")) }
    } else {
        None
    }
}

/// Heuristic public-signature extraction. Top-level only (no leading
/// whitespace) — the same convention this codebase's own public surface
/// follows (`^pub fn`, anchored at column 0) — which also correctly excludes
/// nested declarations (e.g. a Python `def` indented inside another `def`).
/// Best-effort, single-line: a multi-line declaration is captured up to
/// wherever its opening line ends.
pub fn extract_signatures(path: &str, text: &str) -> Vec<String> {
    let markers = markers_for(path);
    text.lines()
        .filter(|l| markers.iter().any(|m| l.starts_with(m)))
        .map(signature_only)
        .collect()
}
