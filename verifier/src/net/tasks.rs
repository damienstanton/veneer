//! Deterministic fold of task ops into task state. Every replica holding the
//! same ops computes the same result: ops are applied in stamp order, and every
//! rule depends only on the ops and their stamps, never on arrival order.
//!
//! Merge rules:
//! - A claim is granted only if no other run holds an unexpired lease at the
//!   claim's own stamp. Concurrent claims resolve to the earliest stamp; the
//!   loser sees its claim ignored after the next sync and stops.
//! - Only the holding run may heartbeat, report progress, ask, or complete.
//! - Terminal states (done, failed, canceled) absorb every later op.
//! - A `done` result is accepted only if it validates against the task
//!   (passing checks, and a veneer witness when the task is gated).

use super::op::{Body, Op};
use crate::net::envelope::{Outcome, Task, TaskResult};
use std::collections::BTreeMap;

/// After this many runs lose their lease, the task fails instead of re-queueing.
pub const MAX_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub run: String,
    pub replica: String,
    pub harness: String,
    pub lease_ms: u64,
    pub last_beat_ms: u64,
}

impl Holder {
    fn expired_at(&self, ms: u64) -> bool {
        ms > self.last_beat_ms.saturating_add(self.lease_ms)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Queued,
    Running,
    NeedsInput,
    Done,
    Failed,
    Canceled,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Queued => "queued",
            Status::Running => "running",
            Status::NeedsInput => "needs_input",
            Status::Done => "done",
            Status::Failed => "failed",
            Status::Canceled => "canceled",
        }
    }
    pub fn terminal(self) -> bool {
        matches!(self, Status::Done | Status::Failed | Status::Canceled)
    }
    /// States an orchestrator's wait returns on.
    pub fn settled(self) -> bool {
        self.terminal() || self == Status::NeedsInput
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskState {
    pub id: String,
    pub task: Task,
    pub created_by: String,
    status: Status,
    pub holder: Option<Holder>,
    /// Runs that lost their lease without reporting.
    pub lapsed: usize,
    pub progress: Vec<String>,
    /// Questions and answers, oldest first.
    pub dialogue: Vec<String>,
    /// Session to resume after an answer.
    pub resume_session: Option<String>,
    pub result: Option<TaskResult>,
    pub cancel_reason: Option<String>,
    /// Ops that were well-formed but not allowed, with the reason.
    pub ignored: Vec<String>,
}

impl TaskState {
    /// Status as seen at `now_ms`: a running task whose lease lapsed reads as
    /// queued (claimable), or failed once it has used up its attempts.
    pub fn status_at(&self, now_ms: u64) -> Status {
        match (&self.status, &self.holder) {
            (Status::Running, Some(h)) if h.expired_at(now_ms) => {
                if self.lapsed + 1 >= MAX_ATTEMPTS {
                    Status::Failed
                } else {
                    Status::Queued
                }
            }
            (s, _) => *s,
        }
    }

    fn holds(&self, run: &str) -> bool {
        self.status == Status::Running && self.holder.as_ref().is_some_and(|h| h.run == run)
    }

    fn apply(&mut self, op: &Op) {
        let ms = op.at.wall_ms;
        // Fold lease expiry in at the op's own time, so the rules are time-pure.
        if let (Status::Running, Some(h)) = (self.status, &self.holder)
            && h.expired_at(ms)
        {
            self.lapsed += 1;
            self.holder = None;
            self.status = if self.lapsed >= MAX_ATTEMPTS { Status::Failed } else { Status::Queued };
        }
        if self.status.terminal() {
            self.ignored.push(format!("{}: task already {}", op.at.text(), self.status.name()));
            return;
        }
        let denial: Option<String> = match &op.body {
            Body::TaskClaim { run, harness, lease_ms, .. } => {
                if self.status == Status::Queued {
                    self.holder = Some(Holder {
                        run: run.clone(),
                        replica: op.at.replica.clone(),
                        harness: harness.clone(),
                        lease_ms: *lease_ms,
                        last_beat_ms: ms,
                    });
                    self.status = Status::Running;
                    None
                } else {
                    Some("claim lost: task not queued".into())
                }
            }
            Body::TaskHeartbeat { run, .. } => {
                if self.holds(run) {
                    if let Some(h) = self.holder.as_mut() {
                        h.last_beat_ms = ms;
                    }
                    None
                } else {
                    Some("heartbeat from a run that does not hold the task".into())
                }
            }
            Body::TaskProgress { run, text, .. } => {
                if self.holds(run) {
                    self.progress.push(text.clone());
                    None
                } else {
                    Some("progress from a run that does not hold the task".into())
                }
            }
            Body::TaskAsk { run, question, session_id, .. } => {
                if self.holds(run) {
                    self.dialogue.push(format!("Q: {question}"));
                    self.resume_session = session_id.clone();
                    self.holder = None;
                    self.status = Status::NeedsInput;
                    None
                } else {
                    Some("question from a run that does not hold the task".into())
                }
            }
            Body::TaskAnswer { text, .. } => {
                if self.status == Status::NeedsInput {
                    self.dialogue.push(format!("A: {text}"));
                    self.status = Status::Queued;
                    None
                } else {
                    Some("answer while no question is open".into())
                }
            }
            Body::TaskComplete { run, result, .. } => {
                if !self.holds(run) {
                    Some("result from a run that does not hold the task".into())
                } else if let Err(e) = result.validate_for(&self.task) {
                    Some(e.to_string())
                } else {
                    self.status = match result.status {
                        Outcome::Done => Status::Done,
                        Outcome::Failed => Status::Failed,
                    };
                    self.result = Some(result.clone());
                    self.holder = None;
                    None
                }
            }
            Body::TaskCancel { reason, .. } => {
                self.status = Status::Canceled;
                self.cancel_reason = Some(reason.clone());
                self.holder = None;
                None
            }
            Body::TaskCreate { .. } => Some("duplicate create".into()),
            Body::MemPut { .. } | Body::MemDel { .. } => None,
        };
        if let Some(why) = denial {
            self.ignored.push(format!("{}: {why}", op.at.text()));
        }
    }
}

fn target(body: &Body) -> Option<&str> {
    match body {
        Body::TaskCreate { id, .. }
        | Body::TaskClaim { id, .. }
        | Body::TaskHeartbeat { id, .. }
        | Body::TaskProgress { id, .. }
        | Body::TaskAsk { id, .. }
        | Body::TaskAnswer { id, .. }
        | Body::TaskComplete { id, .. }
        | Body::TaskCancel { id, .. } => Some(id),
        Body::MemPut { .. } | Body::MemDel { .. } => None,
    }
}

/// Fold ops (already in stamp order) into every task's state, keyed by id.
/// Ops for an id that has not been created yet are dropped as orphans.
pub fn fold(ops: &[Op]) -> (BTreeMap<String, TaskState>, Vec<String>) {
    let mut tasks: BTreeMap<String, TaskState> = BTreeMap::new();
    let mut orphans = vec![];
    for op in ops {
        let Some(id) = target(&op.body) else { continue };
        match (&op.body, tasks.get_mut(id)) {
            (Body::TaskCreate { task, .. }, None) => {
                if task.validate().is_ok() {
                    tasks.insert(
                        id.to_string(),
                        TaskState {
                            id: id.to_string(),
                            task: task.clone(),
                            created_by: op.at.replica.clone(),
                            status: Status::Queued,
                            holder: None,
                            lapsed: 0,
                            progress: vec![],
                            dialogue: vec![],
                            resume_session: None,
                            result: None,
                            cancel_reason: None,
                            ignored: vec![],
                        },
                    );
                } else {
                    orphans.push(format!("{}: invalid task envelope for {id}", op.at.text()));
                }
            }
            (_, Some(state)) => state.apply(op),
            (_, None) => orphans.push(format!("{}: op for unknown task {id}", op.at.text())),
        }
    }
    (tasks, orphans)
}

/// Every dependency of `t` is done.
pub fn deps_done(tasks: &BTreeMap<String, TaskState>, t: &Task, now_ms: u64) -> bool {
    t.depends_on
        .iter()
        .all(|d| tasks.get(d).is_some_and(|s| s.status_at(now_ms) == Status::Done))
}

#[cfg(test)]
#[path = "tasks_tests.rs"]
mod tests;
