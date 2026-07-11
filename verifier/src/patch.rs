//! Unified-diff patch machinery: parse agent-proposed patches and apply them
//! to an in-memory tree (path → contents). Pure value transformations — no
//! filesystem access. `laws::check_idempotency` is the judgement built on
//! top; any parse/apply deviation is an `Err(String)` the caller wraps as a
//! Protocol finding.

use std::collections::BTreeMap;

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
    pub is_delete: bool,
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
        if let Some(old_raw) = line.strip_prefix("--- ") {
            let new_raw = lines
                .next()
                .and_then(|l| l.strip_prefix("+++ "))
                .ok_or("expected +++ after ---")?;
            let new_stripped = strip_prefix_ab(new_raw.trim());
            // Detect deletion: +++ /dev/null (with or without leading a/b/)
            let is_delete = new_stripped == "/dev/null" || new_stripped == "dev/null";
            let path = if is_delete {
                strip_prefix_ab(old_raw.trim())
            } else {
                new_stripped
            };
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
            if is_delete && hunks.iter().any(|h| h.lines.iter().any(|l| matches!(l, DiffLine::Add(_)))) {
                return Err(format!("{path}: deletion patch must not contain added lines"));
            }
            files.push(FilePatch { path, hunks, is_delete });
        }
    }
    if files.is_empty() {
        return Err("no file patches found".into());
    }
    Ok(Patch { files })
}

/// Apply a patch to an in-memory tree (path → contents). Pure: returns a new
/// tree. Strict context matching; any mismatch is a clean Err.
///
/// The applier normalizes output to end with a trailing newline; the
/// `\\ No newline at end of file` marker is accepted but not preserved. Within
/// check_idempotency both applications share this normalization, so verdicts
/// are unaffected.
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
        if fp.is_delete {
            // Deletion verified above via the cursor loop (Del lines matched);
            // remove the file from the tree instead of inserting new content.
            out.remove(&fp.path);
        } else {
            new_lines.extend_from_slice(&old[cursor..]);
            let mut contents = new_lines.join("\n");
            if !contents.is_empty() {
                contents.push('\n');
            }
            out.insert(fp.path.clone(), contents);
        }
    }
    Ok(out)
}
