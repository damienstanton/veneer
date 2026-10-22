use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn turn(id: &str, read: u64) -> String {
    format!(
        r#"{{"type":"assistant","requestId":"r{id}","message":{{"id":"m{id}","usage":{{"input_tokens":1,"cache_creation_input_tokens":9,"cache_read_input_tokens":{read},"output_tokens":3}}}}}}"#
    )
}

/// A project dir plus a Claude Code config dir holding its transcript dir.
struct Fixture {
    project: tempfile::TempDir,
    config: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        Fixture { project: tempfile::tempdir().unwrap(), config: tempfile::tempdir().unwrap() }
    }
    fn root(&self) -> PathBuf {
        std::fs::canonicalize(self.project.path()).unwrap()
    }
    fn transcript_dir(&self) -> PathBuf {
        let slug: String = self
            .root()
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let d = self.config.path().join("projects").join(slug);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn write_session(&self, name: &str, lines: &[String]) -> PathBuf {
        let p = self.transcript_dir().join(name);
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        p
    }
    fn veneer(&self, args: &[&str], stdin: &str) -> std::process::Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_veneer"))
            .current_dir(self.root())
            .env("CLAUDE_CONFIG_DIR", self.config.path())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }
    fn json(&self, args: &[&str]) -> serde_json::Value {
        let out = self.veneer(args, "");
        assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice(&out.stdout).expect("stdout is JSON")
    }
}

fn budget(root: &Path, tokens: u64) {
    std::fs::create_dir_all(root.join(".veneer")).unwrap();
    std::fs::write(root.join(".veneer/config.toml"), format!("[usage]\nbudget_tokens = {tokens}\n")).unwrap();
}

#[test]
fn report_finds_the_projects_transcripts_and_dedupes_repeated_lines() {
    let fx = Fixture::new();
    fx.write_session("a.jsonl", &[turn("1", 100), turn("1", 100), turn("2", 300)]);
    let r = fx.json(&["usage", "--compact"]);
    assert_eq!(r["total"]["sessions"], 1);
    assert_eq!(r["sessions"][0]["turns"], 2);
    assert_eq!(r["sessions"][0]["cache_read"], 400);
    assert_eq!(r["findings"], serde_json::json!([]));
}

#[test]
fn over_budget_sessions_warn_without_failing() {
    let fx = Fixture::new();
    budget(&fx.root(), 200);
    fx.write_session("a.jsonl", &[turn("1", 100), turn("2", 300)]);
    let r = fx.json(&["usage", "--compact"]);
    let f = &r["findings"][0];
    assert_eq!((f["law"].as_str(), f["severity"].as_str()), (Some("context_budget"), Some("warning")));
    assert!(f.get("suggested_fix").is_none(), "compact strips suggested_fix");
    assert_eq!(r["sessions"][0]["cache_read_over_budget"], 100);
}

#[test]
fn transcripts_are_found_from_a_parent_launch_directory() {
    let fx = Fixture::new();
    fx.write_session("a.jsonl", &[turn("1", 10)]);
    let child = fx.root().join("clone");
    std::fs::create_dir_all(&child).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_veneer"))
        .current_dir(&child)
        .env("CLAUDE_CONFIG_DIR", fx.config.path())
        .args(["usage", "--compact"])
        .output()
        .unwrap();
    let r: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(r["total"]["sessions"], 1);
}

#[test]
fn hook_speaks_only_over_budget_and_always_exits_0() {
    let fx = Fixture::new();
    budget(&fx.root(), 200);
    let p = fx.write_session("a.jsonl", &[turn("1", 100)]);
    let input = |p: &Path| format!(r#"{{"hook_event_name":"UserPromptSubmit","transcript_path":"{}"}}"#, p.display());
    let quiet = fx.veneer(&["usage", "--hook"], &input(&p));
    assert_eq!((quiet.status.code(), quiet.stdout.is_empty()), (Some(0), true));
    let p = fx.write_session("a.jsonl", &[turn("1", 100), turn("2", 400)]);
    let loud = fx.veneer(&["usage", "--hook"], &input(&p));
    assert_eq!(loud.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&loud.stdout).unwrap();
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
    assert!(v["hookSpecificOutput"]["additionalContext"].as_str().unwrap().contains("fresh session"));
    assert_eq!(fx.veneer(&["usage", "--hook"], "not json").status.code(), Some(0));
}

#[test]
fn shipping_records_each_cycles_own_cache_reads() {
    let fx = Fixture::new();
    std::fs::write(fx.root().join("a.rs"), "fn main() {}\n").unwrap();
    fx.write_session("s.jsonl", &[turn("1", 100), turn("2", 200)]);
    let ship = |fx: &Fixture| {
        for step in [&["state", "set", "implement"][..], &["state", "set", "verify"], &["check"]] {
            assert_eq!(fx.veneer(step, "").status.code(), Some(0));
        }
        let out = fx.veneer(&["state", "set", "ship", "--ref", "pr=7"], "");
        assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
        // Ship → Ship is an idempotent no-op and must not record twice.
        fx.veneer(&["state", "set", "ship"], "");
        fx.veneer(&["state", "set", "plan"], "");
    };
    ship(&fx);
    fx.write_session("s.jsonl", &[turn("1", 100), turn("2", 200), turn("3", 500)]);
    std::fs::write(fx.root().join("b.rs"), "fn b() {}\n").unwrap();
    ship(&fx);
    let cycles = fx.json(&["usage", "--cycles"]);
    let cycles = cycles.as_array().unwrap();
    assert_eq!(cycles.len(), 2);
    assert_eq!((cycles[0]["cycle"].as_u64(), cycles[0]["cache_read"].as_u64()), (Some(1), Some(300)));
    assert_eq!((cycles[1]["turns"].as_u64(), cycles[1]["cache_read"].as_u64()), (Some(1), Some(500)));
    assert_eq!(cycles[1]["refs"]["pr"], "7");
}

#[test]
fn shipping_without_a_transcript_still_ships() {
    let fx = Fixture::new();
    std::fs::write(fx.root().join("a.rs"), "fn main() {}\n").unwrap();
    for step in [&["state", "set", "implement"][..], &["state", "set", "verify"], &["check"], &["state", "set", "ship"]] {
        assert_eq!(fx.veneer(step, "").status.code(), Some(0));
    }
    assert_eq!(fx.json(&["usage", "--cycles"]), serde_json::json!([]));
}
