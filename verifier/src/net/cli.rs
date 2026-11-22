//! CLI for `veneer task` and `veneer net`. Output is JSON on stdout; failures
//! are protocol findings. Exit codes: 0 ok, 1 finding, 2 usage.

use super::envelope::{Check, Outcome, TASK_SCHEMA, Task, TaskResult};
use super::error::NetError;
use super::hub::{Hub, LeafContext};
use super::replica::DEFAULT_LEASE_MS;
use super::tasks::TaskState;
use super::witness;
use serde_json::{Value, json};
use std::path::Path;

pub const USAGE: &str = "\
  veneer net init --hub <git-remote> [--name <replica>] [--tags a,b] [--harness claude,agy] [--cloud]
  veneer net sync | status | worker [--once]
      cloud sandboxes: detected (Claude Code) or VENEER_CLOUD=<kind>; replica
      named <kind>-<session>; with VENEER_NET_HUB set no config file is needed
  veneer task new <envelope.json | ->      # orchestrator: queue a task
  veneer task list [--all] | show [<id>] | wait <id>... [--timeout <secs>]
  veneer task answer <id> <text> | cancel <id> <reason>
  veneer task claim <id>                    # leaf without a worker (cloud sessions)
  veneer task progress <text> | ask <question> [--session <id>]
  veneer task complete --status done|failed --summary <s> [--pr <url>] [--branch <b>]
                       [--check '<criterion>=pass|fail' ...] [--session <id>]
  veneer memory put <scope> <key> <fact|decision|preference|handoff> <value>
  veneer memory get <scope> <key> | del <scope> <key>
  veneer memory recall [--repo <owner/name>] [--task <id>] [--kind <k>] [--prefix <p>]
";

pub(crate) fn fail(e: NetError) -> i32 {
    let f = e.finding("<net>");
    println!("{}", crate::laws::findings_json_compact(std::slice::from_ref(&f)));
    eprintln!("error [protocol]: {}", f.message);
    1
}

pub(crate) fn usage() -> i32 {
    eprintln!("USAGE:\n{USAGE}");
    2
}

pub(crate) fn print(v: Value) -> i32 {
    println!("{v}");
    0
}

/// `--flag value` lookups over a flat argument list; repeated flags collect.
pub(crate) fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

pub(crate) fn flags<'a>(args: &'a [String], name: &str) -> Vec<&'a str> {
    args.windows(2).filter(|w| w[0] == name).map(|w| w[1].as_str()).collect()
}

pub fn task_json(t: &TaskState, now: u64) -> Value {
    json!({
        "id": t.id,
        "title": t.task.title,
        "repo": t.task.repo,
        "status": t.status_at(now).name(),
        "harness": t.task.harness.name(),
        "holder": t.holder.as_ref().map(|h| json!({"replica": h.replica, "run": h.run, "harness": h.harness})),
        "depends_on": t.task.depends_on,
        "progress": t.progress.last(),
        "dialogue": t.dialogue,
        "result": t.result,
        "ignored": t.ignored,
    })
}

pub fn cmd_task(root: &Path, args: &[String]) -> i32 {
    let mut hub = match Hub::load() {
        Ok(h) => h,
        Err(e) => return fail(e),
    };
    let rest = args.get(1..).unwrap_or(&[]);
    let r = match args.first().map(String::as_str) {
        Some("new") => new(&mut hub, rest),
        Some("list") => list(&hub, rest.iter().any(|a| a == "--all")),
        Some("show") => show(&hub, root, rest.first().map(String::as_str)),
        Some("wait") => wait(&mut hub, rest),
        Some("answer") if rest.len() >= 2 => {
            let text = rest[1..].join(" ");
            hub.write(|r| r.answer(&rest[0], &text)).map(|_| json!({"answered": rest[0]}))
        }
        Some("cancel") if rest.len() >= 2 => {
            let reason = rest[1..].join(" ");
            hub.write(|r| r.cancel(&rest[0], &reason)).map(|_| json!({"canceled": rest[0]}))
        }
        Some("claim") if rest.len() == 1 => claim(&mut hub, root, &rest[0]),
        Some("progress") if !rest.is_empty() => LeafContext::require(root).and_then(|c| {
            let text = rest.join(" ");
            hub.write(|r| r.progress(&c.task, &c.run, &text)).map(|_| json!({"ok": true}))
        }),
        Some("ask") if !rest.is_empty() => ask(&mut hub, root, rest),
        Some("complete") => complete(&mut hub, root, rest),
        _ => return usage(),
    };
    match r {
        Ok(v) => {
            if let Some(off) = &hub.offline {
                eprintln!("warning: hub unreachable, change kept locally and will sync later: {off}");
            }
            print(v)
        }
        Err(e) => fail(e),
    }
}

fn new(hub: &mut Hub, rest: &[String]) -> Result<Value, NetError> {
    let text = match rest.first().map(String::as_str) {
        None | Some("-") => {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
            s
        }
        Some(path) => std::fs::read_to_string(path)?,
    };
    let mut v: Value = serde_json::from_str(&text).map_err(|e| NetError::Invalid(format!("envelope: {e}")))?;
    if v.get("schema").is_none() {
        v["schema"] = json!(TASK_SCHEMA);
    }
    let task: Task = serde_json::from_value(v).map_err(|e| NetError::Invalid(format!("envelope: {e}")))?;
    let id = hub.write(|r| r.create(task))?;
    Ok(json!({"id": id, "status": "queued"}))
}

fn list(hub: &Hub, all: bool) -> Result<Value, NetError> {
    let now = hub.replica.now();
    let snap = hub.replica.snapshot()?;
    let tasks: Vec<Value> = snap
        .tasks
        .values()
        .filter(|t| all || !t.status_at(now).terminal())
        .map(|t| task_json(t, now))
        .collect();
    Ok(json!({"tasks": tasks, "problems": snap.problems}))
}

fn show(hub: &Hub, root: &Path, id: Option<&str>) -> Result<Value, NetError> {
    let id = match id {
        Some(i) => i.to_string(),
        None => LeafContext::require(root)?.task,
    };
    let t = hub.replica.task(&id)?;
    let mut v = task_json(&t, hub.replica.now());
    v["task"] = serde_json::to_value(&t.task).map_err(|e| NetError::Invalid(e.to_string()))?;
    Ok(v)
}

fn claim(hub: &mut Hub, root: &Path, id: &str) -> Result<Value, NetError> {
    let t = hub.replica.task(id)?;
    let harness = std::env::var("VENEER_HARNESS").unwrap_or_else(|_| match t.task.harness.name() {
        "any" => "claude".into(),
        h => h.into(),
    });
    let lease = hub.cfg.lease_ms().max(DEFAULT_LEASE_MS);
    let run = hub.write(|r| r.claim(id, &harness, lease))?;
    if !hub.replica.still_holds(id, &run)? {
        return Err(NetError::Rejected(format!("lost the claim on {id} to an earlier replica")));
    }
    LeafContext { task: id.into(), run: run.clone() }.store(root)?;
    Ok(json!({"id": id, "run": run, "lease_minutes": lease / 60_000, "resume_session": t.resume_session}))
}

fn ask(hub: &mut Hub, root: &Path, rest: &[String]) -> Result<Value, NetError> {
    let c = LeafContext::require(root)?;
    let session = flag(rest, "--session").map(String::from);
    let q: Vec<&str> = rest
        .iter()
        .enumerate()
        .filter(|(i, a)| *a != "--session" && !(i > &0 && rest[i - 1] == "--session"))
        .map(|(_, a)| a.as_str())
        .collect();
    let question = q.join(" ");
    hub.write(|r| r.ask(&c.task, &c.run, &question, session))?;
    Ok(json!({"id": c.task, "status": "needs_input"}))
}

fn complete(hub: &mut Hub, root: &Path, rest: &[String]) -> Result<Value, NetError> {
    let c = LeafContext::require(root)?;
    let status = match flag(rest, "--status") {
        Some("done") => Outcome::Done,
        Some("failed") => Outcome::Failed,
        _ => return Err(NetError::Invalid("--status done|failed is required".into())),
    };
    let summary = flag(rest, "--summary").ok_or_else(|| NetError::Invalid("--summary is required".into()))?;
    let mut r = TaskResult::new(status, summary);
    r.pr = flag(rest, "--pr").map(String::from);
    r.branch = flag(rest, "--branch").map(String::from).or_else(|| witness::git(root, &["branch", "--show-current"]).ok());
    r.session_id = flag(rest, "--session").map(String::from);
    r.harness = std::env::var("VENEER_HARNESS").ok();
    for chk in flags(rest, "--check") {
        let (crit, verdict) = chk
            .rsplit_once('=')
            .ok_or_else(|| NetError::Invalid(format!("--check {chk:?}: use '<criterion>=pass|fail'")))?;
        r.acceptance.push(Check { criterion: crit.into(), pass: verdict == "pass", note: None });
    }
    if status == Outcome::Done {
        r.witness = Some(witness::gate_witness(root)?);
    }
    hub.write(|rep| rep.complete(&c.task, &c.run, r))?;
    Ok(json!({"id": c.task, "status": hub.replica.task(&c.task)?.status_at(hub.replica.now()).name()}))
}

fn wait(hub: &mut Hub, rest: &[String]) -> Result<Value, NetError> {
    let timeout = flag(rest, "--timeout").and_then(|t| t.parse::<u64>().ok()).unwrap_or(3600);
    let ids: Vec<&String> = rest
        .iter()
        .enumerate()
        .filter(|(i, a)| *a != "--timeout" && !(i > &0 && rest[i - 1] == "--timeout"))
        .map(|(_, a)| a)
        .collect();
    if ids.is_empty() {
        return Err(NetError::Invalid("wait needs at least one task id".into()));
    }
    let start = std::time::Instant::now();
    loop {
        let now = hub.replica.now();
        let states: Vec<TaskState> = ids.iter().map(|i| hub.replica.task(i)).collect::<Result<_, _>>()?;
        let settled = states.iter().all(|t| t.status_at(now).settled());
        if settled || start.elapsed().as_secs() >= timeout {
            let tasks: Vec<Value> = states.iter().map(|t| task_json(t, now)).collect();
            return Ok(json!({"settled": settled, "tasks": tasks}));
        }
        std::thread::sleep(std::time::Duration::from_secs(hub.cfg.poll_seconds.clamp(5, 60)));
        hub.sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn flag_parsing() {
        let a = s(&["--status", "done", "--check", "a b=pass", "--check", "c=fail", "--summary", "ok"]);
        assert_eq!(flag(&a, "--status"), Some("done"));
        assert_eq!(flags(&a, "--check"), vec!["a b=pass", "c=fail"]);
        assert_eq!(flag(&a, "--pr"), None);
    }
}
