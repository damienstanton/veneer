//! Replication through git. The hub is any git remote; ops live on one branch
//! (`veneer-net` by default). Each replica only ever changes its own
//! `ops/<replica>.jsonl`, so `pull --rebase` never conflicts and a sync is:
//! commit own file, rebase onto the remote, push. A rejected push (someone
//! else pushed first) is retried after another rebase.

use super::error::NetError;
use std::path::Path;
use std::process::Command;

pub const DEFAULT_BRANCH: &str = "veneer-net";
const PUSH_ATTEMPTS: usize = 4;

fn git(dir: &Path, args: &[&str]) -> Result<String, NetError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| NetError::Io(format!("running git: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(NetError::Io(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

fn remote_has_branch(dir: &Path, branch: &str) -> Result<bool, NetError> {
    Ok(!git(dir, &["ls-remote", "--heads", "origin", branch])?.is_empty())
}

/// Make `dir` a working copy of `remote`'s hub branch, creating the branch
/// (orphan, empty) if the remote does not have it yet. Idempotent.
pub fn open(dir: &Path, remote: &str, branch: &str) -> Result<(), NetError> {
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(dir)?;
        git(dir, &["init", "-q"])?;
        git(dir, &["remote", "add", "origin", remote])?;
    }
    git(dir, &["config", "user.name", "veneer-net"])?;
    git(dir, &["config", "user.email", "veneer-net@localhost"])?;
    if remote_has_branch(dir, branch)? {
        git(dir, &["fetch", "-q", "origin", branch])?;
        let current = git(dir, &["symbolic-ref", "--short", "-q", "HEAD"]).unwrap_or_default();
        let has_commits = git(dir, &["rev-parse", "-q", "--verify", "HEAD"]).is_ok();
        if current != branch || !has_commits {
            git(dir, &["checkout", "-q", "-B", branch, &format!("origin/{branch}")])?;
        }
    } else if git(dir, &["rev-parse", "-q", "--verify", "HEAD"]).is_err() {
        git(dir, &["checkout", "-q", "--orphan", branch])?;
        std::fs::create_dir_all(dir.join("ops"))?;
        std::fs::write(
            dir.join("README.md"),
            "veneer-net hub. Each file in ops/ is one replica's append-only op log. Do not edit by hand.\n",
        )?;
        git(dir, &["add", "README.md"])?;
        git(dir, &["commit", "-q", "-m", "veneer-net: initialize hub"])?;
        git(dir, &["push", "-q", "-u", "origin", branch])?;
    }
    Ok(())
}

/// Serializes git operations between processes sharing one hub working copy
/// (a worker and the leaf it launched). A lock older than two minutes is stale.
struct HubLock(std::path::PathBuf);

impl HubLock {
    fn take(dir: &Path) -> Result<HubLock, NetError> {
        let path = dir.join(".git/veneer-net.lock");
        for _ in 0..300 {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(HubLock(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age.as_secs() > 120);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(NetError::Io("hub is locked by another veneer process".into()))
    }
}

impl Drop for HubLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Publish this replica's ops and pull everyone else's.
pub fn sync(dir: &Path, replica: &str, branch: &str) -> Result<(), NetError> {
    let _lock = HubLock::take(dir)?;
    let own = format!("ops/{replica}.jsonl");
    if dir.join(&own).exists() {
        git(dir, &["add", &own])?;
        let staged = git(dir, &["diff", "--cached", "--name-only"])?;
        if !staged.is_empty() {
            git(dir, &["commit", "-q", "-m", &format!("veneer-net: ops from {replica}")])?;
        }
    }
    let mut last = None;
    for _ in 0..PUSH_ATTEMPTS {
        if remote_has_branch(dir, branch)? {
            git(dir, &["pull", "-q", "--rebase", "origin", branch])?;
        }
        match git(dir, &["push", "-q", "origin", &format!("HEAD:{branch}")]) {
            Ok(_) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| NetError::Io("push failed".into())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::hlc::Clock;
    use crate::net::op::{self, Body, MemKind, Op};
    use crate::net::memory::Memory;

    fn put(clock: &mut Clock, ms: u64, key: &str) -> Op {
        Op {
            at: clock.tick(ms),
            body: Body::MemPut { scope: "global".into(), key: key.into(), kind: MemKind::Fact, value: key.into() },
        }
    }

    #[test]
    fn two_replicas_converge_through_a_bare_remote() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("hub.git");
        Command::new("git").args(["init", "-q", "--bare"]).arg(&remote).status().unwrap();
        let remote = remote.to_str().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        open(&a, remote, DEFAULT_BRANCH).unwrap();
        open(&b, remote, DEFAULT_BRANCH).unwrap();
        // Re-opening is a no-op.
        open(&a, remote, DEFAULT_BRANCH).unwrap();

        let (mut ca, mut cb) = (Clock::new("mac"), Clock::new("cloud"));
        op::append(&a, &put(&mut ca, 10, "from-mac")).unwrap();
        op::append(&b, &put(&mut cb, 11, "from-cloud")).unwrap();
        // Both write before either syncs: the second push must rebase, never conflict.
        sync(&a, "mac", DEFAULT_BRANCH).unwrap();
        sync(&b, "cloud", DEFAULT_BRANCH).unwrap();
        sync(&a, "mac", DEFAULT_BRANCH).unwrap();

        let ma = Memory::fold(&op::load(&a).unwrap().ops);
        let mb = Memory::fold(&op::load(&b).unwrap().ops);
        assert_eq!(ma, mb);
        assert_eq!(ma.len(), 2);

        // Syncing with nothing new is a no-op.
        sync(&b, "cloud", DEFAULT_BRANCH).unwrap();
        sync(&b, "cloud", DEFAULT_BRANCH).unwrap();
    }
}
