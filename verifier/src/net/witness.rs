//! The proof a leaf attaches to `done`: this worktree passed veneer's ship
//! gate, the tree has not changed since, and the work is committed.

use super::envelope::Witness;
use super::error::NetError;
use crate::laws::clean_hash;
use crate::state::{Phase, load};
use std::path::Path;
use std::process::Command;

/// Paths veneer and the harnesses write that never count as uncommitted work.
const TOOL_DIRS: [&str; 3] = [".veneer/", ".claude/", ".agents/"];

pub fn git(root: &Path, args: &[&str]) -> Result<String, NetError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| NetError::Io(format!("running git: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        Err(NetError::Io(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())))
    }
}

/// Uncommitted paths, ignoring tool directories.
pub fn dirty(root: &Path) -> Result<Vec<String>, NetError> {
    Ok(git(root, &["status", "--porcelain", "--untracked-files=all"])?
        .lines()
        .filter_map(|l| l.get(3..))
        .map(|p| p.trim_matches('"').to_string())
        .filter(|p| !TOOL_DIRS.iter().any(|d| p.starts_with(d)))
        .collect())
}

pub fn hash_text(h: u64) -> String {
    format!("fnv:{h:016x}")
}

/// Build the witness, or explain exactly which gate condition fails.
pub fn gate_witness(root: &Path) -> Result<Witness, NetError> {
    let state = load(root).map_err(|f| NetError::Rejected(f.message))?;
    if state.phase != Phase::Ship {
        return Err(NetError::Rejected(format!(
            "veneer phase is {}, not ship; finish the loop with `veneer state set ship`",
            state.phase.name()
        )));
    }
    let now = clean_hash(root);
    if state.last_clean_check != Some(now) {
        return Err(NetError::Rejected(
            "the tree changed after the ship gate; re-run verify (`veneer check`) and ship again".into(),
        ));
    }
    let dirty = dirty(root)?;
    if !dirty.is_empty() {
        return Err(NetError::Rejected(format!("uncommitted changes: {}; commit them first", dirty.join(", "))));
    }
    Ok(Witness {
        clean_hash: hash_text(now),
        commit: git(root, &["rev-parse", "HEAD"])?,
        cycle_cache_reads: crate::cycles::load(root).last().map(|c| c.cache_read),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{record_clean_check, set_phase};

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "t"],
        ] {
            git(p, &args).unwrap();
        }
        std::fs::create_dir_all(p.join(".veneer")).unwrap();
        std::fs::write(p.join(".veneer/config.toml"), "loc_soft = 500\nloc_hard = 1000\n").unwrap();
        std::fs::write(p.join("lib.rs"), "pub fn a() {}\n").unwrap();
        git(p, &["add", "lib.rs"]).unwrap();
        git(p, &["commit", "-qm", "init"]).unwrap();
        d
    }

    fn ship(p: &Path) {
        set_phase(p, Phase::Implement, &[]).unwrap();
        set_phase(p, Phase::Verify, &[]).unwrap();
        record_clean_check(p, clean_hash(p)).unwrap();
        set_phase(p, Phase::Ship, &[]).unwrap();
    }

    #[test]
    fn witness_requires_ship_unchanged_tree_and_commit() {
        let d = repo();
        let p = d.path();
        assert!(gate_witness(p).unwrap_err().to_string().contains("not ship"));
        ship(p);
        let w = gate_witness(p).unwrap();
        assert_eq!(w.clean_hash, hash_text(clean_hash(p)));
        assert_eq!(w.commit.len(), 40);

        std::fs::write(p.join("lib.rs"), "pub fn b() {}\n").unwrap();
        assert!(gate_witness(p).unwrap_err().to_string().contains("tree changed"));
    }

    #[test]
    fn uncommitted_work_blocks_but_tool_dirs_do_not() {
        let d = repo();
        let p = d.path();
        std::fs::create_dir_all(p.join(".claude/skills/veneer")).unwrap();
        std::fs::write(p.join(".claude/skills/veneer/SKILL.md"), "x").unwrap();
        assert!(dirty(p).unwrap().is_empty());
        std::fs::write(p.join("new.rs"), "pub fn n() {}\n").unwrap();
        ship(p);
        assert!(gate_witness(p).unwrap_err().to_string().contains("uncommitted"));
    }
}
