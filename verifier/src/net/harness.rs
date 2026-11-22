//! Adapters for the two headless harnesses. Each turns a task into a command
//! line and turns that harness's JSON output into a common [`RunOutput`].

use crate::net::envelope::{HarnessPref, Permissions, Task};
use crate::net::error::NetError;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harness {
    Claude,
    Agy,
    /// An Antigravity Remote Agent, launched from this machine (see `remote`).
    AgyRemote,
}

impl Harness {
    pub fn name(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Agy => "agy",
            Harness::AgyRemote => "agy-remote",
        }
    }

    pub fn parse(s: &str) -> Option<Harness> {
        match s {
            "claude" => Some(Harness::Claude),
            "agy" | "antigravity" => Some(Harness::Agy),
            "agy-remote" => Some(Harness::AgyRemote),
            _ => None,
        }
    }

    /// Choose a harness for a task given what the machine has, in preference order.
    pub fn choose(pref: HarnessPref, available: &[Harness]) -> Option<Harness> {
        match pref {
            HarnessPref::Claude => available.contains(&Harness::Claude).then_some(Harness::Claude),
            HarnessPref::Agy => available.contains(&Harness::Agy).then_some(Harness::Agy),
            HarnessPref::Any => available.first().copied(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

pub struct LaunchSpec<'a> {
    pub task: &'a Task,
    pub task_id: &'a str,
    pub run: &'a str,
    pub worktree: &'a Path,
    pub branch: &'a str,
    /// Prior questions and answers, oldest first.
    pub dialogue: &'a [String],
    pub resume_session: Option<&'a str>,
    pub extra_args: &'a [String],
    /// The agy remote project, for [`Harness::AgyRemote`].
    pub remote_project: Option<&'a str>,
}

/// The prompt every leaf receives. It is explicit about the skill and the exit
/// protocol so the leaf never depends on noticing the skill on its own.
pub fn leaf_prompt(spec: &LaunchSpec) -> String {
    let t = spec.task;
    let mut p = format!(
        "You are a veneer leaf for task {id}. Use the `veneer` skill: run the normal veneer loop \
         (plan, implement, verify, ship) in this git worktree on branch `{branch}` (repo {repo}, base {base}).\n\
         Start with `veneer task show` and `veneer memory recall`. Report milestones with `veneer task progress \"...\"`.\n\
         Record anything the next agent should know with `veneer memory put`.\n\
         Before you stop you MUST run either `veneer task complete --status done|failed --summary \"...\"` \
         (done needs a passed `veneer state set ship`; add --pr or --branch and one --check per acceptance criterion) \
         or `veneer task ask \"question\"`.\n\n\
         ## Task: {title}\n\n{prompt}\n",
        id = spec.task_id,
        branch = spec.branch,
        repo = t.repo,
        base = t.base,
        title = t.title,
        prompt = t.prompt,
    );
    if !t.context.is_empty() {
        p.push_str("\n## Read first\n\nFor source files, start with `veneer graph query <path>` (signatures and docs) and open the file only if that is not enough.\n\n");
        for c in &t.context {
            p.push_str(&format!("- {c}\n"));
        }
    }
    if !t.acceptance.is_empty() {
        p.push_str("\n## Acceptance criteria (verify each before reporting done)\n\n");
        for a in &t.acceptance {
            p.push_str(&format!("- {a}\n"));
        }
    }
    p.push_str(&format!("\n## Deliver\n\n{:?}", t.deliver).to_lowercase());
    p.push('\n');
    if !spec.dialogue.is_empty() {
        p.push_str("\n## Earlier questions and answers\n\n");
        for d in spec.dialogue {
            p.push_str(d);
            p.push_str("\n\n");
        }
    }
    p
}

pub fn invocation(h: Harness, spec: &LaunchSpec) -> Invocation {
    if h == Harness::AgyRemote {
        return super::remote::invocation(spec);
    }
    let prompt = leaf_prompt(spec);
    let mut args: Vec<String> = vec!["-p".into(), prompt, "--output-format".into(), "json".into()];
    match h {
        Harness::Claude => {
            let mode = match spec.task.permissions {
                Permissions::Plan => "plan",
                Permissions::Edit => "acceptEdits",
                Permissions::Auto => "bypassPermissions",
            };
            args.extend(["--permission-mode".into(), mode.into()]);
            // veneer itself must be callable for the exit protocol.
            args.extend(["--allowedTools".into(), "Bash(veneer:*)".into()]);
            if let Some(n) = spec.task.budget.max_turns {
                args.extend(["--max-turns".into(), n.to_string()]);
            }
            if let Some(s) = spec.resume_session {
                args.extend(["--resume".into(), s.into()]);
            }
        }
        Harness::Agy => {
            let mode = match spec.task.permissions {
                Permissions::Plan => "plan",
                Permissions::Edit => "accept-edits",
                Permissions::Auto => "auto",
            };
            args.extend(["--mode".into(), mode.into()]);
            args.extend([
                "--print-timeout".into(),
                format!("{}s", u64::from(spec.task.budget.minutes) * 60),
            ]);
            if let Some(s) = spec.resume_session {
                args.extend(["--conversation".into(), s.into()]);
            }
        }
        Harness::AgyRemote => unreachable!("handled above"),
    }
    args.extend(spec.extra_args.iter().cloned());
    Invocation {
        program: h.name().into(),
        args,
        env: vec![
            ("VENEER_TASK".into(), spec.task_id.into()),
            ("VENEER_RUN".into(), spec.run.into()),
            ("VENEER_HARNESS".into(), h.name().into()),
        ],
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunOutput {
    pub ok: bool,
    pub session_id: Option<String>,
    pub response: String,
    pub error: Option<String>,
    pub turns: Option<u64>,
    pub usage: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ClaudeOut {
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    num_turns: Option<u64>,
    #[serde(default)]
    usage: Option<serde_json::Value>,
    #[serde(default)]
    total_cost_usd: Option<f64>,
}

#[derive(Deserialize)]
struct AgyOut {
    #[serde(default)]
    conversation_id: Option<String>,
    status: String,
    #[serde(default)]
    response: Option<String>,
    #[serde(default)]
    error: Option<serde_json::Value>,
    #[serde(default)]
    num_turns: Option<u64>,
    #[serde(default)]
    usage: Option<serde_json::Value>,
}

/// Parse the final JSON document a harness printed. Some versions print log
/// lines first, so take the last line that parses as a JSON object.
fn last_json_object(stdout: &str) -> Result<&str, NetError> {
    let trimmed = stdout.trim();
    if trimmed.starts_with('{') && serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return Ok(trimmed);
    }
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with('{') && serde_json::from_str::<serde_json::Value>(l).is_ok())
        .ok_or_else(|| NetError::Invalid("no JSON object in harness output".into()))
}

pub fn parse_output(h: Harness, stdout: &str) -> Result<RunOutput, NetError> {
    let bad = |e: serde_json::Error| NetError::Invalid(format!("{} JSON output: {e}", h.name()));
    let json = last_json_object(stdout)?;
    match h {
        Harness::Claude => {
            let o: ClaudeOut = serde_json::from_str(json).map_err(bad)?;
            let mut usage = o.usage;
            if let (Some(serde_json::Value::Object(m)), Some(c)) = (usage.as_mut(), o.total_cost_usd) {
                m.insert("total_cost_usd".into(), c.into());
            }
            let ok = !o.is_error && o.subtype.as_deref().is_none_or(|s| s == "success");
            Ok(RunOutput {
                ok,
                session_id: o.session_id,
                response: o.result.unwrap_or_default(),
                error: (!ok).then(|| o.subtype.unwrap_or_else(|| "error".into())),
                turns: o.num_turns,
                usage,
            })
        }
        // The remote agent reports through the hub; its terminal output is a TUI.
        Harness::AgyRemote => Err(NetError::Invalid("agy-remote output is a terminal session, not JSON".into())),
        Harness::Agy => {
            let o: AgyOut = serde_json::from_str(json).map_err(bad)?;
            let ok = o.status == "SUCCESS";
            let error = match (ok, o.error) {
                (true, _) => None,
                (false, Some(serde_json::Value::String(s))) => Some(s),
                (false, Some(v)) => Some(v.to_string()),
                (false, None) => Some(o.status.clone()),
            };
            Ok(RunOutput {
                ok,
                session_id: o.conversation_id,
                response: o.response.unwrap_or_default(),
                error,
                turns: o.num_turns,
                usage: o.usage,
            })
        }
    }
}

/// Which harness binaries are on PATH, Claude first.
pub fn detect() -> Vec<Harness> {
    [Harness::Claude, Harness::Agy]
        .into_iter()
        .filter(|h| which(h.name()).is_some())
        .collect()
}

pub fn which(bin: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(bin))
        .find(|p| p.is_file())
}

pub fn require(h: Harness) -> Result<(), NetError> {
    match which(h.name()) {
        Some(_) => Ok(()),
        None => Err(NetError::Io(format!("{} is not on PATH", h.name()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::envelope::*;

    fn task() -> Task {
        serde_json::from_value(serde_json::json!({
            "schema": TASK_SCHEMA, "title": "Fix it", "repo": "o/r", "prompt": "Fix the bug",
            "acceptance": ["tests pass"], "context": ["src/lib.rs"],
            "budget": {"minutes": 20, "max_turns": 30}
        }))
        .unwrap()
    }

    fn spec<'a>(t: &'a Task, d: &'a [String], resume: Option<&'a str>) -> LaunchSpec<'a> {
        LaunchSpec {
            task: t,
            task_id: "t42",
            run: "r1",
            worktree: Path::new("/tmp/wt"),
            branch: "tn/42",
            dialogue: d,
            resume_session: resume,
            extra_args: &[],
            remote_project: None,
        }
    }

    #[test]
    fn choose_respects_preference_and_availability() {
        let both = [Harness::Claude, Harness::Agy];
        assert_eq!(Harness::choose(HarnessPref::Agy, &both), Some(Harness::Agy));
        assert_eq!(Harness::choose(HarnessPref::Any, &both), Some(Harness::Claude));
        assert_eq!(Harness::choose(HarnessPref::Agy, &[Harness::Claude]), None);
        assert_eq!(Harness::choose(HarnessPref::Any, &[]), None);
    }

    #[test]
    fn claude_invocation() {
        let t = task();
        let inv = invocation(Harness::Claude, &spec(&t, &[], Some("s1")));
        assert_eq!(inv.program, "claude");
        let a = inv.args.join(" ");
        assert!(a.contains("--output-format json"));
        assert!(a.contains("--permission-mode acceptEdits"));
        assert!(a.contains("--max-turns 30"));
        assert!(a.contains("--resume s1"));
        assert!(inv.env.contains(&("VENEER_TASK".into(), "t42".into())));
    }

    #[test]
    fn agy_invocation() {
        let mut t = task();
        t.permissions = Permissions::Plan;
        let inv = invocation(Harness::Agy, &spec(&t, &[], Some("conv-9")));
        assert_eq!(inv.program, "agy");
        let a = inv.args.join(" ");
        assert!(a.contains("--mode plan"));
        assert!(a.contains("--print-timeout 1200s"));
        assert!(a.contains("--conversation conv-9"));
        assert!(!a.contains("--max-turns"));
    }

    #[test]
    fn prompt_carries_protocol_and_dialogue() {
        let t = task();
        let d = vec!["Q: which?".to_string(), "A: v2".to_string()];
        let p = leaf_prompt(&spec(&t, &d, None));
        assert!(p.contains("veneer leaf for task t42"));
        assert!(p.contains("veneer task complete"));
        assert!(p.contains("- tests pass"));
        assert!(p.contains("- src/lib.rs"));
        assert!(p.contains("A: v2"));
    }

    #[test]
    fn parses_claude_output() {
        let out = include_str!("../../tests/fixtures/net/claude_success.json");
        let r = parse_output(Harness::Claude, out).unwrap();
        assert!(r.ok);
        assert_eq!(r.session_id.as_deref(), Some("8b0c3a2e-1111-4f5e-9d7a-0123456789ab"));
        assert_eq!(r.turns, Some(7));
        assert!(r.usage.unwrap().get("total_cost_usd").is_some());

        let err = include_str!("../../tests/fixtures/net/claude_max_turns.json");
        let r = parse_output(Harness::Claude, err).unwrap();
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("error_max_turns"));
    }

    #[test]
    fn parses_agy_output_with_leading_logs() {
        let out = include_str!("../../tests/fixtures/net/agy_success.txt");
        let r = parse_output(Harness::Agy, out).unwrap();
        assert!(r.ok);
        assert_eq!(r.session_id.as_deref(), Some("c-5f2e9a"));
        let err = include_str!("../../tests/fixtures/net/agy_error.json");
        let r = parse_output(Harness::Agy, err).unwrap();
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("authentication required"));
    }

    #[test]
    fn garbage_output_is_an_error() {
        assert!(parse_output(Harness::Agy, "panic: boom").is_err());
    }
}
