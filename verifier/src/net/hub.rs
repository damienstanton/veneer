//! A replica bound to its hub: every read pulls first, every write publishes
//! after. Sync failures (offline, no auth) never lose work: ops stay in the
//! local log and go out with the next successful sync.

use super::config::NetConfig;
use super::error::NetError;
use super::replica::Replica;
use super::sync;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub struct Hub {
    pub cfg: NetConfig,
    pub replica: Replica,
    /// The last sync error, if the hub could not be reached.
    pub offline: Option<String>,
}

impl Hub {
    pub fn open(cfg: NetConfig) -> Result<Hub, NetError> {
        let dir = cfg.hub_dir();
        sync::open(&dir, &cfg.hub, &cfg.branch)?;
        let replica = Replica::open(&dir, &cfg.replica)?;
        let mut hub = Hub { cfg, replica, offline: None };
        hub.sync();
        Ok(hub)
    }

    pub fn load() -> Result<Hub, NetError> {
        Hub::open(NetConfig::load()?)
    }

    /// Best-effort publish and pull; re-opens the replica so its clock observes
    /// everything that arrived.
    pub fn sync(&mut self) {
        let dir = self.cfg.hub_dir();
        match sync::sync(&dir, &self.cfg.replica, &self.cfg.branch) {
            Ok(()) => self.offline = None,
            Err(e) => self.offline = Some(e.to_string()),
        }
        if let Ok(r) = Replica::open(&dir, &self.cfg.replica) {
            self.replica = r;
        }
    }

    /// Run a write against the replica, then publish it.
    pub fn write<T>(&mut self, f: impl FnOnce(&mut Replica) -> Result<T, NetError>) -> Result<T, NetError> {
        let out = f(&mut self.replica)?;
        self.sync();
        Ok(out)
    }
}

/// Which task and run this working directory belongs to. The worker writes it
/// into each worktree; `veneer task claim` writes it for a self-claimed leaf.
/// `VENEER_TASK` and `VENEER_RUN` override it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeafContext {
    pub task: String,
    pub run: String,
}

fn context_path(root: &Path) -> PathBuf {
    root.join(".veneer/task.json")
}

impl LeafContext {
    pub fn store(&self, root: &Path) -> Result<(), NetError> {
        std::fs::create_dir_all(root.join(".veneer"))?;
        let text = serde_json::to_string(self).map_err(|e| NetError::Invalid(e.to_string()))?;
        std::fs::write(context_path(root), text)?;
        Ok(())
    }

    pub fn find(root: &Path) -> Option<LeafContext> {
        let from_file = std::fs::read_to_string(context_path(root))
            .ok()
            .and_then(|t| serde_json::from_str::<LeafContext>(&t).ok());
        match (std::env::var("VENEER_TASK").ok(), std::env::var("VENEER_RUN").ok()) {
            (Some(task), Some(run)) => Some(LeafContext { task, run }),
            (Some(task), None) => from_file.filter(|c| c.task == task),
            _ => from_file,
        }
    }

    pub fn require(root: &Path) -> Result<LeafContext, NetError> {
        LeafContext::find(root).ok_or_else(|| {
            NetError::Invalid("not inside a task: no VENEER_TASK/VENEER_RUN and no .veneer/task.json; claim one with `veneer task claim <id>`".into())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_context_round_trips_through_the_worktree() {
        let d = tempfile::tempdir().unwrap();
        let c = LeafContext { task: "t1".into(), run: "mac-1".into() };
        c.store(d.path()).unwrap();
        assert_eq!(LeafContext::find(d.path()), Some(c));
        let empty = tempfile::tempdir().unwrap();
        if std::env::var("VENEER_TASK").is_err() {
            assert!(LeafContext::require(empty.path()).is_err());
        }
    }

    #[test]
    fn hub_open_write_and_converge() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("hub.git");
        std::process::Command::new("git").args(["init", "-q", "--bare"]).arg(&remote).status().unwrap();
        let mk = |name: &str| {
            let mut c = NetConfig::new(name.into(), remote.to_str().unwrap().into());
            c.hub_dir = Some(tmp.path().join(name));
            c
        };
        let mut a = Hub::open(mk("mac")).unwrap();
        let id = a.write(|r| r.create(crate::net::envelope::tests::task())).unwrap();
        assert!(a.offline.is_none());
        let b = Hub::open(mk("cloud")).unwrap();
        assert!(b.replica.task(&id).is_ok());
    }
}
