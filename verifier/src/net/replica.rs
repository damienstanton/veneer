//! One replica's view of the hub: emit ops with a monotonic clock and read the
//! folded state. Every command (CLI, MCP, worker) goes through this facade.

use super::error::NetError;
use super::hlc::{Clock, now_ms};
use super::memory::Memory;
use super::op::{self, Body, MemKind, Op};
use super::tasks::{self, Status, TaskState};
use crate::net::envelope::{Task, TaskResult};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_LEASE_MS: u64 = 10 * 60 * 1000;

pub struct Replica {
    root: PathBuf,
    clock: Clock,
    /// Fixed time source for tests; `None` uses the system clock.
    fixed_now: Option<u64>,
}

#[derive(Debug)]
pub struct Snapshot {
    pub tasks: BTreeMap<String, TaskState>,
    pub memory: Memory,
    /// Unreadable lines, orphan ops: reported, never fatal.
    pub problems: Vec<String>,
}

/// What a machine can run, for deciding which queued tasks it may claim.
#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    pub machine: String,
    pub tags: Vec<String>,
    pub harnesses: Vec<String>,
    pub local: bool,
    /// This machine can dispatch Antigravity Remote Agents (an agy remote
    /// project is configured), so it takes `agy` cloud tasks on their behalf.
    pub remote_agy: bool,
}

impl Replica {
    pub fn open(root: &Path, name: &str) -> Result<Replica, NetError> {
        if !op::valid_replica(name) {
            return Err(NetError::Invalid(format!("bad replica name {name:?}")));
        }
        let mut clock = Clock::new(name);
        for o in op::load(root)?.ops {
            clock.observe(&o.at);
        }
        Ok(Replica { root: root.to_path_buf(), clock, fixed_now: None })
    }

    pub fn with_time(mut self, ms: u64) -> Replica {
        self.fixed_now = Some(ms);
        self
    }

    pub fn set_time(&mut self, ms: u64) {
        self.fixed_now = Some(ms);
    }

    pub fn now(&self) -> u64 {
        self.fixed_now.unwrap_or_else(now_ms)
    }

    pub fn name(&self) -> &str {
        self.clock.replica()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn emit(&mut self, body: Body) -> Result<Op, NetError> {
        for s in op::own_stamps(&self.root, self.clock.replica()) {
            self.clock.observe(&s);
        }
        let now = self.now();
        let o = Op { at: self.clock.tick(now), body };
        op::append(&self.root, &o)?;
        Ok(o)
    }

    pub fn snapshot(&self) -> Result<Snapshot, NetError> {
        let loaded = op::load(&self.root)?;
        let (tasks, orphans) = tasks::fold(&loaded.ops);
        let mut problems = loaded.problems;
        problems.extend(orphans);
        Ok(Snapshot { tasks, memory: Memory::fold(&loaded.ops), problems })
    }

    pub fn task(&self, id: &str) -> Result<TaskState, NetError> {
        self.snapshot()?
            .tasks
            .remove(id)
            .ok_or_else(|| NetError::Invalid(format!("no task {id}")))
    }

    pub fn create(&mut self, task: Task) -> Result<String, NetError> {
        task.validate()?;
        let id = op::task_id(&self.clock.clone().tick(self.now()), &task);
        // The id is a pure function of the envelope and a stamp; it need not be
        // the create op's own stamp, only unique, which the stamp guarantees.
        self.emit(Body::TaskCreate { id: id.clone(), task })?;
        Ok(id)
    }

    /// Claim a queued task. Returns the run id. The claim is provisional until
    /// the next sync: if another replica claimed first, the fold will show its
    /// run as holder and this one must stop (see [`Replica::still_holds`]).
    pub fn claim(&mut self, id: &str, harness: &str, lease_ms: u64) -> Result<String, NetError> {
        let t = self.task(id)?;
        let status = t.status_at(self.now());
        if status != Status::Queued {
            return Err(NetError::Rejected(format!("task {id} is {}", status.name())));
        }
        let run = format!("{}-{}", self.name(), self.now());
        self.emit(Body::TaskClaim { id: id.into(), run: run.clone(), harness: harness.into(), lease_ms })?;
        if self.still_holds(id, &run)? {
            Ok(run)
        } else {
            Err(NetError::Rejected(format!("claim on {id} lost")))
        }
    }

    pub fn still_holds(&self, id: &str, run: &str) -> Result<bool, NetError> {
        let t = self.task(id)?;
        Ok(t.status_at(self.now()) == Status::Running && t.holder.is_some_and(|h| h.run == run))
    }

    pub fn heartbeat(&mut self, id: &str, run: &str) -> Result<(), NetError> {
        self.emit(Body::TaskHeartbeat { id: id.into(), run: run.into() }).map(|_| ())
    }

    pub fn progress(&mut self, id: &str, run: &str, text: &str) -> Result<(), NetError> {
        self.emit(Body::TaskProgress { id: id.into(), run: run.into(), text: text.into() }).map(|_| ())
    }

    pub fn ask(&mut self, id: &str, run: &str, question: &str, session_id: Option<String>) -> Result<(), NetError> {
        self.emit(Body::TaskAsk { id: id.into(), run: run.into(), question: question.into(), session_id })
            .map(|_| ())
    }

    pub fn answer(&mut self, id: &str, text: &str) -> Result<(), NetError> {
        let t = self.task(id)?;
        if t.status_at(self.now()) != Status::NeedsInput {
            return Err(NetError::Rejected(format!("task {id} has no open question")));
        }
        self.emit(Body::TaskAnswer { id: id.into(), text: text.into() }).map(|_| ())
    }

    /// Report a result. Validated here first so a bad report fails loudly at
    /// the leaf instead of being silently ignored by every replica's fold.
    pub fn complete(&mut self, id: &str, run: &str, result: TaskResult) -> Result<(), NetError> {
        let t = self.task(id)?;
        result.validate_for(&t.task)?;
        if !self.still_holds(id, run)? {
            return Err(NetError::Rejected(format!("run {run} does not hold task {id}")));
        }
        self.emit(Body::TaskComplete { id: id.into(), run: run.into(), result }).map(|_| ())
    }

    pub fn cancel(&mut self, id: &str, reason: &str) -> Result<(), NetError> {
        self.task(id)?;
        self.emit(Body::TaskCancel { id: id.into(), reason: reason.into() }).map(|_| ())
    }

    pub fn remember(&mut self, scope: &str, key: &str, kind: MemKind, value: &str) -> Result<(), NetError> {
        if !op::valid_scope(scope) {
            return Err(NetError::Invalid(format!("bad scope {scope:?}; use global, repo:<owner/name>, or task:<id>")));
        }
        if key.trim().is_empty() {
            return Err(NetError::Invalid("empty key".into()));
        }
        self.emit(Body::MemPut { scope: scope.into(), key: key.into(), kind, value: value.into() }).map(|_| ())
    }

    pub fn forget(&mut self, scope: &str, key: &str) -> Result<(), NetError> {
        self.emit(Body::MemDel { scope: scope.into(), key: key.into() }).map(|_| ())
    }

    /// Queued tasks this machine may claim, oldest first.
    pub fn claimable(&self, caps: &Capabilities) -> Result<Vec<TaskState>, NetError> {
        let snap = self.snapshot()?;
        let now = self.now();
        let mut out: Vec<TaskState> = snap
            .tasks
            .values()
            .filter(|t| t.status_at(now) == Status::Queued)
            .filter(|t| fits(&t.task, caps))
            .filter(|t| tasks::deps_done(&snap.tasks, &t.task, now))
            .cloned()
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }
}

pub fn fits(t: &Task, caps: &Capabilities) -> bool {
    use crate::net::envelope::{HarnessPref, Where};
    let common = t.machine.as_ref().is_none_or(|m| *m == caps.machine) && t.needs.iter().all(|n| caps.tags.contains(n));
    if dispatches_remote_agy(t, caps) {
        return common;
    }
    let harness_ok = match t.harness {
        HarnessPref::Any => !caps.harnesses.is_empty(),
        h => caps.harnesses.iter().any(|x| x == h.name()),
    };
    let where_ok = match t.location {
        Where::Any => true,
        Where::Local => caps.local,
        Where::Cloud => !caps.local,
    };
    harness_ok && where_ok && common
}

/// A local machine with an agy remote project takes `where = cloud` tasks
/// that Antigravity may run, and launches them as Remote Agents. `needs` tags
/// still apply to the machine, which is the honest reading when it is the
/// dispatcher; a task that needs a tag in the sandbox should not set it.
pub fn dispatches_remote_agy(t: &Task, caps: &Capabilities) -> bool {
    use crate::net::envelope::{HarnessPref, Where};
    caps.local && caps.remote_agy && t.location == Where::Cloud && matches!(t.harness, HarnessPref::Agy | HarnessPref::Any)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::envelope::tests::{task, witness};
    use crate::net::envelope::{HarnessPref, Outcome};

    fn caps() -> Capabilities {
        Capabilities {
            machine: "mac".into(),
            tags: vec!["macos".into()],
            harnesses: vec!["claude".into(), "agy".into()],
            local: true,
            remote_agy: false,
        }
    }

    #[test]
    fn a_remote_agy_dispatcher_takes_agy_cloud_tasks_only() {
        use crate::net::envelope::Where;
        let mut t = task();
        t.location = Where::Cloud;
        t.harness = HarnessPref::Agy;
        assert!(!fits(&t, &caps()), "a plain local machine takes no cloud task");
        let disp = Capabilities { remote_agy: true, ..caps() };
        assert!(fits(&t, &disp));
        t.harness = HarnessPref::Claude;
        assert!(!fits(&t, &disp), "Claude cloud tasks are for Claude cloud sessions");
        t.harness = HarnessPref::Any;
        assert!(fits(&t, &disp));
    }

    #[test]
    fn end_to_end_on_one_hub_dir() {
        let d = tempfile::tempdir().unwrap();
        let mut orch = Replica::open(d.path(), "orch").unwrap().with_time(1_000);
        let id = orch.create(task()).unwrap();

        let mut mac = Replica::open(d.path(), "mac").unwrap().with_time(2_000);
        assert_eq!(mac.claimable(&caps()).unwrap().len(), 1);
        let run = mac.claim(&id, "claude", DEFAULT_LEASE_MS).unwrap();
        assert!(mac.claimable(&caps()).unwrap().is_empty());
        mac.remember(&format!("task:{id}"), "handoff", MemKind::Handoff, "backoff in fetch.rs").unwrap();
        mac.progress(&id, &run, "tests written").unwrap();

        // A result without the gate witness is refused at the source.
        let r = TaskResult::new(Outcome::Done, "done");
        assert!(matches!(mac.complete(&id, &run, r), Err(NetError::Rejected(_))));
        let mut r = TaskResult::new(Outcome::Done, "done");
        r.witness = Some(witness());
        r.acceptance = vec![crate::net::envelope::Check { criterion: "cargo test passes".into(), pass: true, note: None }];
        mac.complete(&id, &run, r).unwrap();

        let s = orch.snapshot().unwrap();
        assert_eq!(s.tasks[&id].status_at(3_000), Status::Done);
        assert_eq!(s.memory.recall(None, Some(&id), None, "").len(), 1);
        assert!(s.problems.is_empty());
    }

    #[test]
    fn second_claimer_is_refused_and_clock_survives_reopen() {
        let d = tempfile::tempdir().unwrap();
        let mut orch = Replica::open(d.path(), "orch").unwrap().with_time(1_000);
        let id = orch.create(task()).unwrap();
        let mut a = Replica::open(d.path(), "a").unwrap().with_time(2_000);
        let mut b = Replica::open(d.path(), "b").unwrap().with_time(2_000);
        a.claim(&id, "claude", 60_000).unwrap();
        assert!(b.claim(&id, "agy", 60_000).is_err());
        // Reopening with a wall clock behind the log still issues later stamps.
        let mut late = Replica::open(d.path(), "orch").unwrap().with_time(5);
        late.cancel(&id, "changed plans").unwrap();
        assert_eq!(late.task(&id).unwrap().status_at(5), Status::Canceled);
    }

    #[test]
    fn ask_and_answer_through_the_facade() {
        let d = tempfile::tempdir().unwrap();
        let mut orch = Replica::open(d.path(), "orch").unwrap().with_time(1_000);
        let id = orch.create(task()).unwrap();
        let mut mac = Replica::open(d.path(), "mac").unwrap().with_time(2_000);
        let run = mac.claim(&id, "agy", 60_000).unwrap();
        assert!(orch.answer(&id, "early").is_err());
        mac.ask(&id, &run, "v1 or v2?", Some("conv-1".into())).unwrap();
        orch.set_time(3_000);
        orch.answer(&id, "v2").unwrap();
        mac.set_time(4_000);
        assert_eq!(mac.claimable(&caps()).unwrap()[0].resume_session.as_deref(), Some("conv-1"));
    }

    #[test]
    fn fit_rules() {
        let mut t = task();
        assert!(fits(&t, &caps()));
        t.needs = vec!["xcode".into()];
        assert!(!fits(&t, &caps()));
        let mut t = task();
        t.harness = HarnessPref::Agy;
        let claude_only = Capabilities { harnesses: vec!["claude".into()], ..caps() };
        assert!(!fits(&t, &claude_only));
        let mut t = task();
        t.machine = Some("mini".into());
        assert!(!fits(&t, &caps()));
        let mut t = task();
        t.location = crate::net::envelope::Where::Cloud;
        assert!(!fits(&t, &caps()));
        assert!(!fits(&task(), &Capabilities { harnesses: vec![], ..caps() }));
        assert!(matches!(
            Replica::open(std::path::Path::new("/nonexistent"), "../bad"),
            Err(NetError::Invalid(_))
        ));
    }
}
