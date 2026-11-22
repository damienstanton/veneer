//! Per-machine configuration for veneer net, kept outside any repo at
//! `~/.config/veneer/net.toml` (override with `VENEER_NET_CONFIG`). A repo's
//! `.veneer/config.toml` describes the repo; this file describes the machine.

use super::error::NetError;
use super::harness::{self, Harness};
use super::replica::Capabilities;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetConfig {
    /// This replica's name; also its op-log file name.
    pub replica: String,
    /// Git remote of the hub (any URL git can push to).
    pub hub: String,
    #[serde(default = "default_branch")]
    pub branch: String,
    /// Working copy of the hub branch.
    #[serde(default)]
    pub hub_dir: Option<PathBuf>,
    /// Capability tags this machine offers (`macos`, `xcode`, `gpu`, ...).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Harnesses this machine runs; empty means detect from PATH.
    #[serde(default)]
    pub harnesses: Vec<String>,
    /// False for a cloud session replica (it takes `where = cloud` tasks).
    #[serde(default = "yes")]
    pub local: bool,
    /// `owner/name` to an existing local clone.
    #[serde(default)]
    pub repos: BTreeMap<String, PathBuf>,
    /// Where repos without a local clone are cloned, and where worktrees go.
    #[serde(default)]
    pub work_dir: Option<PathBuf>,
    #[serde(default = "default_lease")]
    pub lease_minutes: u64,
    #[serde(default = "default_poll")]
    pub poll_seconds: u64,
    #[serde(default)]
    pub claude_args: Vec<String>,
    #[serde(default)]
    pub agy_args: Vec<String>,
}

fn default_branch() -> String {
    super::sync::DEFAULT_BRANCH.into()
}
fn yes() -> bool {
    true
}
fn default_lease() -> u64 {
    10
}
fn default_poll() -> u64 {
    30
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_path() -> PathBuf {
    std::env::var_os("VENEER_NET_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config/veneer/net.toml"))
}

fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(|d| PathBuf::from(d).join("veneer/net"))
        .unwrap_or_else(|| home().join(".local/share/veneer/net"))
}

/// A replica name derived from the host, restricted to the op-file alphabet.
pub fn default_replica() -> String {
    let raw = std::env::var("VENEER_REPLICA")
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
        .unwrap_or_default();
    let clean: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .take(64)
        .collect();
    let clean = clean.trim_matches('-').to_string();
    if clean.is_empty() { "replica".into() } else { clean }
}

impl NetConfig {
    pub fn new(replica: String, hub: String) -> NetConfig {
        NetConfig {
            replica,
            hub,
            branch: default_branch(),
            hub_dir: None,
            tags: vec![],
            harnesses: vec![],
            local: true,
            repos: BTreeMap::new(),
            work_dir: None,
            lease_minutes: default_lease(),
            poll_seconds: default_poll(),
            claude_args: vec![],
            agy_args: vec![],
        }
    }

    pub fn load_from(path: &Path) -> Result<NetConfig, NetError> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            NetError::Io(format!("{}: {e}; run `veneer net init --hub <git-remote>` first", path.display()))
        })?;
        let mut c: NetConfig =
            toml::from_str(&text).map_err(|e| NetError::Invalid(format!("{}: {e}", path.display())))?;
        if let Ok(r) = std::env::var("VENEER_REPLICA") {
            c.replica = r;
        }
        Ok(c)
    }

    pub fn load() -> Result<NetConfig, NetError> {
        Self::load_from(&config_path())
    }

    pub fn store_to(&self, path: &Path) -> Result<(), NetError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| NetError::Invalid(e.to_string()))?;
        std::fs::write(path, text)?;
        Ok(())
    }

    pub fn hub_dir(&self) -> PathBuf {
        self.hub_dir.clone().unwrap_or_else(|| data_dir().join("hub"))
    }

    pub fn work_dir(&self) -> PathBuf {
        self.work_dir.clone().unwrap_or_else(|| data_dir().join("work"))
    }

    pub fn lease_ms(&self) -> u64 {
        self.lease_minutes.max(1) * 60_000
    }

    pub fn harnesses(&self) -> Vec<Harness> {
        if self.harnesses.is_empty() {
            harness::detect()
        } else {
            self.harnesses.iter().filter_map(|h| Harness::parse(h)).collect()
        }
    }

    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            machine: self.replica.clone(),
            tags: self.tags.clone(),
            harnesses: self.harnesses().iter().map(|h| h.name().to_string()).collect(),
            local: self.local,
        }
    }

    pub fn extra_args(&self, h: Harness) -> &[String] {
        match h {
            Harness::Claude => &self.claude_args,
            Harness::Agy => &self.agy_args,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_with_defaults() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("net.toml");
        std::fs::write(&p, "replica = \"mac\"\nhub = \"git@github.com:o/hub.git\"\ntags = [\"macos\"]\n").unwrap();
        let c = NetConfig::load_from(&p).unwrap();
        assert_eq!(c.branch, "veneer-net");
        assert!(c.local);
        assert_eq!(c.lease_ms(), 600_000);
        c.store_to(&p).unwrap();
        assert_eq!(NetConfig::load_from(&p).unwrap(), c);
    }

    #[test]
    fn missing_config_names_the_fix() {
        let e = NetConfig::load_from(Path::new("/nonexistent/net.toml")).unwrap_err();
        assert!(e.to_string().contains("veneer net init"));
    }

    #[test]
    fn default_replica_is_a_valid_file_name() {
        assert!(super::super::op::valid_replica(&default_replica()));
    }

    #[test]
    fn explicit_harnesses_override_detection() {
        let mut c = NetConfig::new("mac".into(), "x".into());
        c.harnesses = vec!["agy".into(), "nope".into()];
        assert_eq!(c.harnesses(), vec![Harness::Agy]);
        assert_eq!(c.capabilities().harnesses, vec!["agy"]);
    }
}
