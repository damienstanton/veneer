//! The machine-side loop: pull the hub, claim one task this machine fits,
//! give it a fresh git worktree with veneer initialized, run the chosen
//! harness headless, heartbeat while it works, and make sure a result is
//! recorded even if the leaf exits without reporting.

use super::envelope::{Outcome, Task, TaskResult};
use super::error::NetError;
use super::harness::{self, Harness, LaunchSpec};
use super::hub::{Hub, LeafContext};
use super::tasks::{Status, TaskState};
use super::witness::{self, git};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_secs(2);

/// How a run ended, from the worker's side.
#[derive(Debug, Clone, PartialEq)]
pub enum RunEnd {
    Exited { stdout: String, success: bool },
    BudgetExceeded,
    LostClaim,
    LaunchFailed(String),
}

pub fn branch_for(id: &str) -> String {
    format!("veneer/{id}")
}

/// Run until stopped; with `once`, handle at most one task and return.
pub fn run(hub: &mut Hub, once: bool) -> Result<(), NetError> {
    loop {
        hub.sync();
        if let Some(e) = &hub.offline {
            eprintln!("veneer net: hub unreachable ({e}); working from the local log");
        }
        let caps = hub.cfg.capabilities();
        let available = hub.cfg.harnesses();
        let next = hub
            .replica
            .claimable(&caps)?
            .into_iter()
            .find_map(|t| {
                if super::replica::dispatches_remote_agy(&t.task, &caps) {
                    return Some((t, Harness::AgyRemote));
                }
                Harness::choose(t.task.harness, &available).map(|h| (t, h))
            });
        if let Some((t, h)) = next {
            match run_task(hub, &t, h) {
                Ok(status) => eprintln!("veneer net: task {} ended {}", t.id, status.name()),
                Err(e) => eprintln!("veneer net: task {}: {e}", t.id),
            }
        }
        if once {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(hub.cfg.poll_seconds.max(5)));
    }
}

fn run_task(hub: &mut Hub, t: &TaskState, h: Harness) -> Result<Status, NetError> {
    let lease = hub.cfg.lease_ms();
    let run = hub.write(|r| r.claim(&t.id, h.name(), lease))?;
    // Arbitration happens on sync: an earlier claim from another replica wins.
    if !hub.replica.still_holds(&t.id, &run)? {
        return Err(NetError::Rejected("lost the claim to an earlier replica".into()));
    }
    // A remote agent works in its own sandbox; locally it only needs a
    // directory for the terminal log.
    let prepared = if h == Harness::AgyRemote {
        remote_dir(hub, &t.id, &run)
    } else {
        prepare_worktree(hub, &t.id, &t.task, &run)
    };
    let wt = match prepared {
        Ok(wt) => wt,
        Err(e) => {
            let mut r = TaskResult::new(Outcome::Failed, format!("worker could not prepare the worktree: {e}"));
            r.self_reported = false;
            hub.write(|rep| rep.complete(&t.id, &run, r))?;
            return Ok(Status::Failed);
        }
    };
    let branch = branch_for(&t.id);
    let extra = hub.cfg.extra_args(h).to_vec();
    let project = hub.cfg.agy_remote_project.clone();
    let spec = LaunchSpec {
        task: &t.task,
        task_id: &t.id,
        run: &run,
        worktree: &wt,
        branch: &branch,
        dialogue: &t.dialogue,
        resume_session: t.resume_session.as_deref(),
        extra_args: &extra,
        remote_project: project.as_deref(),
    };
    let end = supervise(hub, &t.id, &run, h, &spec, &wt);
    hub.sync();
    let state = hub.replica.task(&t.id)?;
    if state.status_at(hub.replica.now()) == Status::Running && hub.replica.still_holds(&t.id, &run)? {
        let result = fallback_result(h, &end, &wt, &t.task, &branch);
        hub.write(|r| r.complete(&t.id, &run, result))?;
    }
    Ok(hub.replica.task(&t.id)?.status_at(hub.replica.now()))
}

/// Launch the harness and watch it: heartbeat on a quarter of the lease, stop
/// it if the budget runs out or the claim is lost (canceled, or out-raced).
fn supervise(hub: &mut Hub, id: &str, run: &str, h: Harness, spec: &LaunchSpec, wt: &Path) -> RunEnd {
    let inv = harness::invocation(h, spec);
    let log = std::fs::File::create(wt.join(".veneer/leaf.log"));
    let mut cmd = Command::new(&inv.program);
    cmd.args(&inv.args).envs(inv.env.iter().cloned()).current_dir(wt).stdout(Stdio::piped());
    // An interactive remote session ends at EOF, so hold its stdin open.
    cmd.stdin(if h == Harness::AgyRemote { Stdio::piped() } else { Stdio::null() });
    if let Ok(f) = log {
        cmd.stderr(f);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return RunEnd::LaunchFailed(format!("{}: {e}", inv.program)),
    };
    let _hold_stdin = child.stdin.take();
    let reader = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = out.read_to_string(&mut s);
            s
        })
    });
    let started = Instant::now();
    let budget = Duration::from_secs(u64::from(spec.task.budget.minutes) * 60);
    let beat_every = Duration::from_millis(hub.cfg.lease_ms() / 4);
    let mut last_beat = Instant::now();
    let collect = |reader: Option<std::thread::JoinHandle<String>>| reader.and_then(|r| r.join().ok()).unwrap_or_default();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return RunEnd::Exited { stdout: collect(reader), success: status.success() },
            Ok(None) => {}
            Err(e) => return RunEnd::LaunchFailed(e.to_string()),
        }
        if started.elapsed() > budget {
            stop(&mut child);
            return RunEnd::BudgetExceeded;
        }
        if last_beat.elapsed() >= beat_every {
            last_beat = Instant::now();
            // Pull first: the leaf may have reported (a remote agent reports
            // from its own replica), and a settled task needs no heartbeat.
            hub.sync();
            if !hub.replica.still_holds(id, run).unwrap_or(true) {
                stop(&mut child);
                return RunEnd::LostClaim;
            }
            let _ = hub.write(|r| r.heartbeat(id, run));
        }
        std::thread::sleep(POLL);
    }
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The result the worker records when the leaf did not report its own.
pub fn fallback_result(h: Harness, end: &RunEnd, wt: &Path, task: &Task, branch: &str) -> TaskResult {
    let parsed = match end {
        RunEnd::Exited { stdout, .. } => harness::parse_output(h, stdout).ok(),
        _ => None,
    };
    let summary = match end {
        RunEnd::BudgetExceeded => format!("stopped after the {}-minute budget", task.budget.minutes),
        RunEnd::LostClaim => "stopped: the claim was lost".into(),
        RunEnd::LaunchFailed(e) => format!("could not launch {}: {e}", h.name()),
        RunEnd::Exited { success, .. } => match &parsed {
            Some(p) if p.ok => format!("leaf exited without reporting; its last message: {}", clip(&p.response)),
            Some(p) => format!("{} failed: {}", h.name(), p.error.clone().unwrap_or_default()),
            None => format!("{} exited ({}) with unreadable output", h.name(), if *success { "ok" } else { "error" }),
        },
    };
    let witness = witness::gate_witness(wt).ok();
    let mut r = TaskResult::new(if witness.is_some() { Outcome::Done } else { Outcome::Failed }, summary);
    if witness.is_some() && push_branch(wt, branch).is_ok() {
        r.branch = Some(branch.into());
    }
    r.witness = witness;
    r.harness = Some(h.name().into());
    r.session_id = parsed.as_ref().and_then(|p| p.session_id.clone());
    r.usage = parsed.and_then(|p| p.usage);
    r.self_reported = false;
    if r.validate_for(task).is_err() {
        // e.g. gated `done` with criteria the worker cannot vouch for.
        r.status = Outcome::Failed;
    }
    r
}

fn clip(s: &str) -> String {
    let t: String = s.chars().take(400).collect();
    if t.len() < s.len() { format!("{t}...") } else { t }
}

fn push_branch(wt: &Path, branch: &str) -> Result<(), NetError> {
    git(wt, &["push", "-q", "-u", "origin", &format!("HEAD:refs/heads/{branch}")]).map(|_| ())
}

fn remote_dir(hub: &Hub, id: &str, run: &str) -> Result<PathBuf, NetError> {
    let d = hub.cfg.work_dir().join("remote").join(format!("{id}-{run}"));
    std::fs::create_dir_all(d.join(".veneer"))?;
    Ok(d)
}

/// Clone (or reuse) the repo, add a worktree on `veneer/<id>` from the base,
/// initialize veneer there (which installs the skill for both harnesses), and
/// record which task and run the worktree belongs to.
fn prepare_worktree(hub: &Hub, id: &str, task: &Task, run: &str) -> Result<PathBuf, NetError> {
    let work = hub.cfg.work_dir();
    let repo_dir = match hub.cfg.repos.get(&task.repo) {
        Some(p) => p.clone(),
        None => {
            let p = work.join("clones").join(&task.repo);
            if !p.join(".git").exists() {
                std::fs::create_dir_all(p.parent().unwrap_or(&work))?;
                let url = format!("https://github.com/{}.git", task.repo);
                git(&work, &["clone", "-q", &url, &p.to_string_lossy()])?;
            }
            p
        }
    };
    git(&repo_dir, &["fetch", "-q", "origin", &task.base])?;
    let wt = work.join("worktrees").join(format!("{id}-{run}"));
    if wt.exists() {
        let _ = git(&repo_dir, &["worktree", "remove", "--force", &wt.to_string_lossy()]);
    }
    git(
        &repo_dir,
        &["worktree", "add", "-q", "-B", &branch_for(id), &wt.to_string_lossy(), &format!("origin/{}", task.base)],
    )?;
    let exe = std::env::current_exe().map_err(|e| NetError::Io(e.to_string()))?;
    let init = Command::new(exe).arg("init").current_dir(&wt).output()?;
    if !init.status.success() {
        return Err(NetError::Io(format!("veneer init: {}", String::from_utf8_lossy(&init.stderr).trim())));
    }
    crate::state::set_phase(&wt, crate::state::Phase::Plan, &[("task".into(), id.into())])
        .map_err(|f| NetError::Io(f.message))?;
    LeafContext { task: id.into(), run: run.into() }.store(&wt)?;
    Ok(wt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::envelope::tests::task;

    #[test]
    fn fallback_without_gate_is_failed_and_not_self_reported() {
        let d = tempfile::tempdir().unwrap();
        let out = include_str!("../../tests/fixtures/net/claude_success.json");
        let end = RunEnd::Exited { stdout: out.into(), success: true };
        let r = fallback_result(Harness::Claude, &end, d.path(), &task(), "veneer/t1");
        assert_eq!(r.status, Outcome::Failed);
        assert!(!r.self_reported);
        assert!(r.summary.contains("without reporting"));
        assert_eq!(r.session_id.as_deref(), Some("8b0c3a2e-1111-4f5e-9d7a-0123456789ab"));
        r.validate_for(&task()).unwrap();
    }

    #[test]
    fn fallback_names_budget_and_launch_failures() {
        let d = tempfile::tempdir().unwrap();
        let r = fallback_result(Harness::Agy, &RunEnd::BudgetExceeded, d.path(), &task(), "b");
        assert!(r.summary.contains("20-minute budget"));
        let r = fallback_result(Harness::Agy, &RunEnd::LaunchFailed("no such file".into()), d.path(), &task(), "b");
        assert!(r.summary.contains("could not launch agy"));
    }
}
