//! The operation alphabet and its on-disk form. Each replica appends ops to its
//! own `ops/<replica>.jsonl`; no file is ever written by two replicas, so a git
//! merge of the op directory can never conflict.

use super::error::{NetError, fnv64};
use super::hlc::Stamp;
use crate::net::envelope::{Task, TaskResult};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemKind {
    Fact,
    Decision,
    Preference,
    Handoff,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Body {
    TaskCreate { id: String, task: Task },
    TaskClaim { id: String, run: String, harness: String, lease_ms: u64 },
    TaskHeartbeat { id: String, run: String },
    TaskProgress { id: String, run: String, text: String },
    TaskAsk { id: String, run: String, question: String, session_id: Option<String> },
    TaskAnswer { id: String, text: String },
    TaskComplete { id: String, run: String, result: TaskResult },
    TaskCancel { id: String, reason: String },
    MemPut { scope: String, key: String, kind: MemKind, value: String },
    MemDel { scope: String, key: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Op {
    pub at: Stamp,
    #[serde(flatten)]
    pub body: Body,
}

/// Deterministic task id from the creating stamp and the envelope.
pub fn task_id(at: &Stamp, task: &Task) -> String {
    let mut bytes = at.text().into_bytes();
    bytes.extend(serde_json::to_vec(task).unwrap_or_default());
    format!("t{:010x}", fnv64(&bytes) & 0xff_ffff_ffff)
}

/// Memory scopes: `global`, `repo:<owner/name>`, `task:<id>`.
pub fn valid_scope(scope: &str) -> bool {
    scope == "global"
        || scope.strip_prefix("repo:").is_some_and(|r| r.split('/').count() == 2 && !r.contains(' '))
        || scope.strip_prefix("task:").is_some_and(|t| !t.is_empty() && !t.contains(' '))
}

/// Replica names become file names: keep them to a safe alphabet.
pub fn valid_replica(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        && !name.starts_with('.')
}

pub fn ops_dir(root: &Path) -> PathBuf {
    root.join("ops")
}

/// Stamps already in one replica's own file. Several processes on one machine
/// (the worker and the leaf it launched) share a replica name, so each one
/// observes the file before stamping to keep its stamps strictly increasing.
pub fn own_stamps(root: &Path, replica: &str) -> Vec<Stamp> {
    std::fs::read_to_string(ops_dir(root).join(format!("{replica}.jsonl")))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Op>(l).ok())
        .map(|o| o.at)
        .collect()
}

/// Append one op to this replica's log, flushed to disk before returning.
pub fn append(root: &Path, op: &Op) -> Result<(), NetError> {
    if !valid_replica(&op.at.replica) {
        return Err(NetError::Invalid(format!("bad replica name {:?}", op.at.replica)));
    }
    let dir = ops_dir(root);
    std::fs::create_dir_all(&dir)?;
    let mut line = serde_json::to_string(op).map_err(|e| NetError::Invalid(e.to_string()))?;
    line.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("{}.jsonl", op.at.replica)))?;
    f.write_all(line.as_bytes())?;
    f.sync_all()?;
    Ok(())
}

/// Everything read from the op directory: the ops, plus a problem per line
/// that could not be used (reported, never fatal).
#[derive(Debug, Default)]
pub struct Loaded {
    pub ops: Vec<Op>,
    pub problems: Vec<String>,
}

pub fn load(root: &Path) -> Result<Loaded, NetError> {
    let dir = ops_dir(root);
    let mut out = Loaded::default();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    for path in files {
        let owner = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let text = std::fs::read_to_string(&path)?;
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let at = format!("{}:{}", path.display(), i + 1);
            match serde_json::from_str::<Op>(line) {
                // A replica may only speak in its own file.
                Ok(op) if op.at.replica == owner => out.ops.push(op),
                Ok(_) => out.problems.push(format!("{at}: op stamped by another replica; ignored")),
                // A torn final line from a crash mid-append is expected; anything else is reported too.
                Err(e) => out.problems.push(format!("{at}: unreadable op ({e}); ignored")),
            }
        }
    }
    out.ops.sort_by(|a, b| a.at.cmp(&b.at));
    // Identical lines (e.g. a replayed append) collapse; distinct ops never do.
    out.ops.dedup_by(|a, b| a == b);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::envelope::tests::task;

    fn st(ms: u64, r: &str) -> Stamp {
        Stamp { wall_ms: ms, counter: 0, replica: r.into() }
    }

    #[test]
    fn append_then_load_merges_replicas_in_clock_order() {
        let d = tempfile::tempdir().unwrap();
        let put = |ms, r: &str, k: &str| Op {
            at: st(ms, r),
            body: Body::MemPut { scope: "global".into(), key: k.into(), kind: MemKind::Fact, value: "v".into() },
        };
        append(d.path(), &put(30, "mac", "c")).unwrap();
        append(d.path(), &put(10, "cloud-1", "a")).unwrap();
        append(d.path(), &put(20, "mac", "b")).unwrap();
        let l = load(d.path()).unwrap();
        assert!(l.problems.is_empty());
        let keys: Vec<_> = l
            .ops
            .iter()
            .map(|o| match &o.body {
                Body::MemPut { key, .. } => key.as_str(),
                _ => "",
            })
            .collect();
        assert_eq!(keys, ["a", "b", "c"]);
    }

    #[test]
    fn torn_lines_and_forged_replicas_are_reported_not_fatal() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(ops_dir(d.path())).unwrap();
        let good = Op { at: st(1, "mac"), body: Body::TaskCancel { id: "t1".into(), reason: "r".into() } };
        let forged = Op { at: st(2, "other"), body: Body::TaskCancel { id: "t1".into(), reason: "r".into() } };
        let text = format!(
            "{}\n{}\n{{\"at\":",
            serde_json::to_string(&good).unwrap(),
            serde_json::to_string(&forged).unwrap()
        );
        std::fs::write(ops_dir(d.path()).join("mac.jsonl"), text).unwrap();
        let l = load(d.path()).unwrap();
        assert_eq!(l.ops, vec![good]);
        assert_eq!(l.problems.len(), 2);
    }

    #[test]
    fn missing_dir_is_empty() {
        let d = tempfile::tempdir().unwrap();
        assert!(load(d.path()).unwrap().ops.is_empty());
    }

    #[test]
    fn ids_and_names() {
        let t = task();
        let a = task_id(&st(1, "mac"), &t);
        assert_eq!(a, task_id(&st(1, "mac"), &t));
        assert_ne!(a, task_id(&st(2, "mac"), &t));
        assert_eq!(a.len(), 11);
        assert!(valid_scope("global") && valid_scope("repo:o/r") && valid_scope("task:t1"));
        assert!(!valid_scope("repo:nope") && !valid_scope("task:") && !valid_scope("x"));
        assert!(valid_replica("dim2-643-local") && !valid_replica("../x") && !valid_replica(".hidden"));
    }

    #[test]
    fn op_json_shape_is_flat_and_tagged() {
        let op = Op { at: st(5, "mac"), body: Body::TaskAnswer { id: "t1".into(), text: "v2".into() } };
        let v = serde_json::to_value(&op).unwrap();
        assert_eq!(v["op"], "task_answer");
        assert_eq!(v["at"]["replica"], "mac");
        assert_eq!(v["text"], "v2");
    }
}
