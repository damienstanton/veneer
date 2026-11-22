//! CLI for `veneer net` (setup, sync, worker) and `veneer memory`.

use super::cli::{fail, flag, print, usage};
use super::config::{NetConfig, config_path, default_replica};
use super::error::NetError;
use super::hub::{Hub, LeafContext};
use super::op::MemKind;
use serde_json::json;
use std::path::Path;

pub fn cmd_net(args: &[String]) -> i32 {
    let rest = args.get(1..).unwrap_or(&[]);
    match args.first().map(String::as_str) {
        Some("init") => init(rest),
        Some("sync") => match Hub::load() {
            Ok(h) => match &h.offline {
                None => print(json!({"synced": true, "replica": h.cfg.replica})),
                Some(e) => fail(NetError::Io(e.clone())),
            },
            Err(e) => fail(e),
        },
        Some("status") => match Hub::load().and_then(|h| {
            let snap = h.replica.snapshot()?;
            let now = h.replica.now();
            let mut counts = std::collections::BTreeMap::<&str, usize>::new();
            for t in snap.tasks.values() {
                *counts.entry(t.status_at(now).name()).or_default() += 1;
            }
            Ok(json!({
                "replica": h.cfg.replica, "hub": h.cfg.hub, "branch": h.cfg.branch,
                "online": h.offline.is_none(), "capabilities": {
                    "tags": h.cfg.tags, "harnesses": h.cfg.capabilities().harnesses, "local": h.cfg.local },
                "tasks": counts, "memory_entries": snap.memory.len(), "problems": snap.problems,
            }))
        }) {
            Ok(v) => print(v),
            Err(e) => fail(e),
        },
        Some("worker") => match Hub::load() {
            Ok(mut h) => match super::worker::run(&mut h, rest.iter().any(|a| a == "--once")) {
                Ok(()) => 0,
                Err(e) => fail(e),
            },
            Err(e) => fail(e),
        },
        _ => usage(),
    }
}

fn init(rest: &[String]) -> i32 {
    let Some(hub) = flag(rest, "--hub") else { return usage() };
    let path = config_path();
    let mut cfg = NetConfig::load_from(&path).unwrap_or_else(|_| NetConfig::new(default_replica(), hub.into()));
    cfg.hub = hub.into();
    if let Some(n) = flag(rest, "--name") {
        cfg.replica = n.into();
    }
    let list = |s: &str| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect::<Vec<_>>();
    if let Some(t) = flag(rest, "--tags") {
        cfg.tags = list(t);
    }
    if let Some(h) = flag(rest, "--harness") {
        cfg.harnesses = list(h);
    }
    if rest.iter().any(|a| a == "--cloud") {
        cfg.local = false;
    }
    if let Err(e) = cfg.store_to(&path) {
        return fail(e);
    }
    match Hub::open(cfg) {
        Ok(h) => print(json!({
            "config": path, "replica": h.cfg.replica, "hub_dir": h.cfg.hub_dir(),
            "harnesses": h.cfg.capabilities().harnesses, "online": h.offline.is_none(),
        })),
        Err(e) => fail(e),
    }
}

fn kind(s: &str) -> Result<MemKind, NetError> {
    match s {
        "fact" => Ok(MemKind::Fact),
        "decision" => Ok(MemKind::Decision),
        "preference" => Ok(MemKind::Preference),
        "handoff" => Ok(MemKind::Handoff),
        _ => Err(NetError::Invalid(format!("kind {s:?}: use fact|decision|preference|handoff"))),
    }
}

pub fn cmd_memory(root: &Path, args: &[String]) -> i32 {
    let mut hub = match Hub::load() {
        Ok(h) => h,
        Err(e) => return fail(e),
    };
    let rest = args.get(1..).unwrap_or(&[]);
    let r = match args.first().map(String::as_str) {
        Some("put") if rest.len() >= 4 => kind(&rest[2]).and_then(|k| {
            let value = rest[3..].join(" ");
            hub.write(|r| r.remember(&rest[0], &rest[1], k, &value)).map(|_| json!({"ok": true}))
        }),
        Some("del") if rest.len() == 2 => hub.write(|r| r.forget(&rest[0], &rest[1])).map(|_| json!({"ok": true})),
        Some("get") if rest.len() == 2 => hub
            .replica
            .snapshot()
            .map(|s| s.memory.get(&rest[0], &rest[1]).map(entry_json).unwrap_or(json!(null))),
        Some("recall") => recall(&hub, root, rest),
        _ => return usage(),
    };
    match r {
        Ok(v) => print(v),
        Err(e) => fail(e),
    }
}

fn entry_json(e: &super::memory::Entry) -> serde_json::Value {
    json!({"scope": e.scope, "key": e.key, "kind": e.kind, "value": e.value, "by": e.by, "at": e.at})
}

fn recall(hub: &Hub, root: &Path, rest: &[String]) -> Result<serde_json::Value, NetError> {
    // Inside a task, default to that task and its repo.
    let leaf = LeafContext::find(root);
    let task = flag(rest, "--task").map(String::from).or_else(|| leaf.as_ref().map(|c| c.task.clone()));
    let snap = hub.replica.snapshot()?;
    let repo = flag(rest, "--repo")
        .map(String::from)
        .or_else(|| task.as_ref().and_then(|t| snap.tasks.get(t)).map(|t| t.task.repo.clone()));
    let k = flag(rest, "--kind").map(kind).transpose()?;
    let prefix = flag(rest, "--prefix").unwrap_or("");
    let entries: Vec<_> = snap
        .memory
        .recall(repo.as_deref(), task.as_deref(), k, prefix)
        .into_iter()
        .map(entry_json)
        .collect();
    Ok(json!({"repo": repo, "task": task, "entries": entries}))
}
