//! Shared agent memory: a last-writer-wins map keyed by (scope, key), with the
//! full write history kept for audit. Any harness reads and writes it through
//! the same CLI or MCP tool, so what one leaf learns the next one can use.

use super::op::{Body, MemKind, Op};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub scope: String,
    pub key: String,
    pub kind: MemKind,
    pub value: String,
    pub by: String,
    pub at: String,
    /// Earlier values this one replaced, oldest first.
    pub history: Vec<String>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Memory {
    live: BTreeMap<(String, String), Entry>,
    /// Keys deleted, with their last value, for audit.
    pub tombstones: BTreeMap<(String, String), String>,
}

impl Memory {
    /// Fold memory ops (already in stamp order). Later stamps win.
    pub fn fold(ops: &[Op]) -> Memory {
        let mut m = Memory::default();
        for op in ops {
            match &op.body {
                Body::MemPut { scope, key, kind, value } => {
                    let k = (scope.clone(), key.clone());
                    let mut history = m.live.remove(&k).map(|e| {
                        let mut h = e.history;
                        h.push(e.value);
                        h
                    });
                    if history.is_none() {
                        history = m.tombstones.remove(&k).map(|v| vec![v]);
                    }
                    m.live.insert(
                        k,
                        Entry {
                            scope: scope.clone(),
                            key: key.clone(),
                            kind: *kind,
                            value: value.clone(),
                            by: op.at.replica.clone(),
                            at: op.at.text(),
                            history: history.unwrap_or_default(),
                        },
                    );
                }
                Body::MemDel { scope, key } => {
                    let k = (scope.clone(), key.clone());
                    if let Some(e) = m.live.remove(&k) {
                        m.tombstones.insert(k, e.value);
                    }
                }
                _ => {}
            }
        }
        m
    }

    pub fn get(&self, scope: &str, key: &str) -> Option<&Entry> {
        self.live.get(&(scope.to_string(), key.to_string()))
    }

    /// Entries visible from a task in `repo`: global, then that repo, then the
    /// task itself, filtered by optional kind and key prefix.
    pub fn recall(&self, repo: Option<&str>, task: Option<&str>, kind: Option<MemKind>, prefix: &str) -> Vec<&Entry> {
        let scopes: Vec<String> = std::iter::once("global".to_string())
            .chain(repo.map(|r| format!("repo:{r}")))
            .chain(task.map(|t| format!("task:{t}")))
            .collect();
        scopes
            .iter()
            .flat_map(|s| self.live.range((s.clone(), String::new())..).take_while(move |((sc, _), _)| sc == s))
            .map(|(_, e)| e)
            .filter(|e| kind.is_none_or(|k| e.kind == k) && e.key.starts_with(prefix))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::hlc::Stamp;

    fn put(ms: u64, r: &str, scope: &str, key: &str, kind: MemKind, v: &str) -> Op {
        Op {
            at: Stamp { wall_ms: ms, counter: 0, replica: r.into() },
            body: Body::MemPut { scope: scope.into(), key: key.into(), kind, value: v.into() },
        }
    }
    fn del(ms: u64, scope: &str, key: &str) -> Op {
        Op {
            at: Stamp { wall_ms: ms, counter: 0, replica: "x".into() },
            body: Body::MemDel { scope: scope.into(), key: key.into() },
        }
    }

    #[test]
    fn last_writer_wins_with_history() {
        let m = Memory::fold(&[
            put(1, "mac", "global", "style", MemKind::Preference, "tabs"),
            put(2, "cloud", "global", "style", MemKind::Preference, "spaces"),
        ]);
        let e = m.get("global", "style").unwrap();
        assert_eq!(e.value, "spaces");
        assert_eq!(e.by, "cloud");
        assert_eq!(e.history, vec!["tabs"]);
    }

    #[test]
    fn concurrent_writes_converge_by_stamp_not_arrival() {
        let a = put(5, "a", "global", "k", MemKind::Fact, "from-a");
        let b = put(5, "b", "global", "k", MemKind::Fact, "from-b");
        let mut x = vec![a.clone(), b.clone()];
        let mut y = vec![b, a];
        x.sort_by(|p, q| p.at.cmp(&q.at));
        y.sort_by(|p, q| p.at.cmp(&q.at));
        assert_eq!(Memory::fold(&x), Memory::fold(&y));
        assert_eq!(Memory::fold(&x).get("global", "k").unwrap().value, "from-b");
    }

    #[test]
    fn delete_then_rewrite_keeps_audit_trail() {
        let m = Memory::fold(&[
            put(1, "a", "repo:o/r", "db", MemKind::Fact, "postgres"),
            del(2, "repo:o/r", "db"),
        ]);
        assert!(m.get("repo:o/r", "db").is_none());
        assert_eq!(m.tombstones.len(), 1);
        let m = Memory::fold(&[
            put(1, "a", "repo:o/r", "db", MemKind::Fact, "postgres"),
            del(2, "repo:o/r", "db"),
            put(3, "a", "repo:o/r", "db", MemKind::Fact, "sqlite"),
        ]);
        assert_eq!(m.get("repo:o/r", "db").unwrap().history, vec!["postgres"]);
    }

    #[test]
    fn recall_scopes_kind_and_prefix() {
        let m = Memory::fold(&[
            put(1, "a", "global", "lang", MemKind::Preference, "rust"),
            put(2, "a", "repo:o/r", "build.cmd", MemKind::Fact, "cargo make"),
            put(3, "a", "repo:o/other", "build.cmd", MemKind::Fact, "make"),
            put(4, "a", "task:t1", "handoff", MemKind::Handoff, "left off at parser"),
            put(5, "a", "repo:o/r", "why-sqlite", MemKind::Decision, "single file"),
        ]);
        let all: Vec<_> = m.recall(Some("o/r"), Some("t1"), None, "").iter().map(|e| e.value.as_str()).collect();
        assert_eq!(all, ["rust", "cargo make", "single file", "left off at parser"]);
        let facts = m.recall(Some("o/r"), None, Some(MemKind::Fact), "build.");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].value, "cargo make");
        assert_eq!(m.recall(None, None, None, "").len(), 1);
    }
}
