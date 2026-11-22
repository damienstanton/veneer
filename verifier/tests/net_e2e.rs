//! End to end: an orchestrator replica queues a task, a worker replica on
//! "another machine" claims it and runs a stand-in harness that follows the
//! leaf protocol (veneer loop, ship gate, `veneer task complete`), and the
//! orchestrator sees it done with a witness, all through a bare git hub.
#![cfg(unix)]

use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn sh(dir: &Path, cmd: &str) {
    let ok = Command::new("sh").arg("-c").arg(cmd).current_dir(dir).status().unwrap().success();
    assert!(ok, "failed: {cmd}");
}

struct Env {
    tmp: tempfile::TempDir,
    bin: PathBuf,
}

impl Env {
    fn veneer(&self, cfg: &str, cwd: &Path, args: &[&str], stdin: Option<&str>) -> (i32, Value) {
        let path = format!("{}:{}", self.bin.display(), std::env::var("PATH").unwrap());
        let mut c = Command::new(env!("CARGO_BIN_EXE_veneer"));
        c.args(args)
            .current_dir(cwd)
            .env("PATH", path)
            .env("HOME", self.tmp.path())
            .env("VENEER_NET_CONFIG", self.tmp.path().join(cfg))
            .env_remove("VENEER_TASK")
            .env_remove("VENEER_RUN")
            .env_remove("VENEER_REPLICA")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn().unwrap();
        if let Some(s) = stdin {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
        } else {
            drop(child.stdin.take());
        }
        let out = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let last = text.lines().last().unwrap_or("null");
        let v = serde_json::from_str(last).unwrap_or(Value::Null);
        assert!(
            out.status.code() != Some(2),
            "usage error: {args:?}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.code().unwrap_or(-1), v)
    }
}

const FAKE_CLAUDE: &str = r#"#!/bin/sh
set -e
veneer task show > /dev/null
veneer memory put "task:$VENEER_TASK" next-step handoff "added() is in; wire it into main next" > /dev/null
echo 'pub fn added() {}' > added.rs
git add added.rs
git commit -qm "add added()"
veneer state set implement > /dev/null
veneer state set verify > /dev/null
veneer check --compact > /dev/null
veneer state set ship > /dev/null
veneer task complete --status done --summary "added added()" --check "it compiles=pass" > /dev/null
echo '{"type":"result","subtype":"success","is_error":false,"result":"ok","session_id":"s-1","num_turns":3}'
"#;

#[test]
fn orchestrator_to_worker_to_done_through_a_git_hub() {
    let tmp = tempfile::tempdir().unwrap();
    let t = &tmp.path().to_path_buf();
    // Hub and a target repo with one commit on main.
    sh(t, "git init -q --bare hub.git && git init -q --bare origin.git");
    sh(
        t,
        "git init -q -b main src && cd src && git config user.email t@t && git config user.name t \
         && echo 'pub fn a() {}' > lib.rs && git add . && git commit -qm init \
         && git remote add origin ../origin.git && git push -q origin main",
    );
    sh(t, "git clone -q origin.git clone");
    let bin = t.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_veneer"), bin.join("veneer")).unwrap();
    std::fs::write(bin.join("claude"), FAKE_CLAUDE).unwrap();
    std::fs::set_permissions(bin.join("claude"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let env = Env { tmp, bin };
    let hub = t.join("hub.git");

    // Orchestrator replica.
    let (code, v) = env.veneer("orch.toml", t, &["net", "init", "--hub", hub.to_str().unwrap(), "--name", "orch"], None);
    assert_eq!(code, 0, "{v}");
    let envelope = r#"{"title":"Add added()","repo":"o/r","prompt":"Add a function named added.","harness":"claude","acceptance":["it compiles"],"budget":{"minutes":5}}"#;
    let (code, v) = env.veneer("orch.toml", t, &["task", "new", "-"], Some(envelope));
    assert_eq!(code, 0, "{v}");
    let id = v["id"].as_str().unwrap().to_string();

    // Worker replica on "another machine".
    std::fs::write(
        t.join("mac.toml"),
        format!(
            "replica = \"mac\"\nhub = \"{}\"\nhub_dir = \"{}\"\nwork_dir = \"{}\"\nharnesses = [\"claude\"]\n[repos]\n\"o/r\" = \"{}\"\n",
            hub.display(),
            t.join("mac-hub").display(),
            t.join("mac-work").display(),
            t.join("clone").display()
        ),
    )
    .unwrap();
    let (code, _) = env.veneer("mac.toml", t, &["net", "worker", "--once"], None);
    assert_eq!(code, 0);

    // The orchestrator sees it done, with a gate witness and the leaf's memory.
    let (code, v) = env.veneer("orch.toml", t, &["task", "wait", &id, "--timeout", "1"], None);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["settled"], true, "{v}");
    let task = &v["tasks"][0];
    assert_eq!(task["status"], "done", "{task}");
    assert!(task["result"]["witness"]["clean_hash"].as_str().unwrap().starts_with("fnv:"));
    assert_eq!(task["result"]["self_reported"], true);
    assert_eq!(task["result"]["branch"], format!("veneer/{id}"));

    let (_, v) = env.veneer("orch.toml", t, &["memory", "recall", "--task", &id], None);
    assert_eq!(v["entries"][0]["value"], "added() is in; wire it into main next", "{v}");
    assert_eq!(v["repo"], "o/r");

    // A second worker pass finds nothing to do.
    let (code, _) = env.veneer("mac.toml", t, &["net", "worker", "--once"], None);
    assert_eq!(code, 0);
    let (_, v) = env.veneer("orch.toml", t, &["task", "list"], None);
    assert_eq!(v["tasks"].as_array().unwrap().len(), 0, "{v}");
}

#[test]
fn leaf_commands_outside_a_task_are_findings_not_crashes() {
    let tmp = tempfile::tempdir().unwrap();
    let t = &tmp.path().to_path_buf();
    sh(t, "git init -q --bare hub.git");
    let env = Env { bin: t.join("bin"), tmp };
    let hub = t.join("hub.git");
    env.veneer("o.toml", t, &["net", "init", "--hub", hub.to_str().unwrap(), "--name", "o"], None);
    let (code, v) = env.veneer("o.toml", t, &["task", "progress", "hello"], None);
    assert_eq!(code, 1);
    assert_eq!(v[0]["law"], "protocol");
    let (code, v) = env.veneer("o.toml", t, &["task", "new", "-"], Some(r#"{"title":"x","repo":"bad","prompt":"p"}"#));
    assert_eq!(code, 1, "{v}");
    let (code, _) = env.veneer("missing.toml", t, &["task", "list"], None);
    assert_eq!(code, 1);
}
