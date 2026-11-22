//! Antigravity Remote Agents, launched from a local machine.
//!
//! agy has no print mode for remote agents yet (`--remote-agent does not
//! support --print`), so the worker opens an interactive remote session in a
//! pseudo-terminal with the task as its first prompt. Nothing is read back from
//! that terminal: the remote agent joins the hub itself (`bin/veneer-up` in the
//! tasknet checkout), reports on the worker's run, and the worker stops the
//! terminal once the task leaves `running`. The lease stays the worker's job.

use super::envelope::Permissions;
use super::harness::{Invocation, LaunchSpec, leaf_prompt};

/// The kind name a remote agent's replica uses (`agy-remote-<run tail>`).
pub const CLOUD_KIND: &str = "agy-remote";

/// Steps a remote sandbox needs before the ordinary leaf protocol: it starts
/// with no environment from us, so every identifier is spelled out.
pub fn remote_prompt(spec: &LaunchSpec) -> String {
    let t = spec.task;
    let env = format!(
        "export VENEER_TASK={id} VENEER_RUN={run} VENEER_HARNESS={kind} VENEER_CLOUD={kind} \
         VENEER_SESSION_ID={run} PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:$PATH\"",
        id = spec.task_id,
        run = spec.run,
        kind = CLOUD_KIND,
    );
    let mut p = format!(
        "You are an Antigravity Remote Agent working veneer net task {id} for a dispatcher on another machine. \
         Do the setup below exactly, then the task.\n\n\
         ## Setup (cloud sandbox)\n\n\
         1. In every shell command you run, first run: `{env}`\n\
         2. Find the checkout of the tasknet hub repo in this workspace (connected repos are cloned for you; \
         try `find / -maxdepth 5 -type d -name tasknet -not -path '*/proc/*' 2>/dev/null`). From its root run \
         `bin/veneer-up`. It installs veneer and joins the hub; if it fails, put its full output in your final \
         answer and stop.\n\
         3. Find the checkout of {repo} the same way (or `git clone https://github.com/{repo}`). In it run \
         `git fetch origin {base} && git checkout -B {branch} origin/{base} && veneer init`, and do all the \
         work below in that checkout.\n\
         4. Before reporting done, commit and run `git push -u origin {branch}`, then pass `--branch {branch}` \
         to `veneer task complete`.\n\
         Pushing uses `GH_TOKEN` from this project's secrets; never print it.\n\n",
        id = spec.task_id,
        repo = t.repo,
        base = t.base,
        branch = spec.branch,
    );
    p.push_str(&leaf_prompt(spec));
    p
}

/// `agy --remote-agent` inside `script`, which supplies the terminal agy's
/// interactive mode needs. Its stdin must stay open (the worker holds it).
pub fn invocation(spec: &LaunchSpec) -> Invocation {
    let mut agy: Vec<String> = vec!["agy".into(), "--remote-agent".into()];
    if let Some(p) = spec.remote_project {
        agy.extend(["--project".into(), p.into()]);
    }
    // Remote sessions support plan and accept-edits; auto maps to accept-edits.
    let mode = if spec.task.permissions == Permissions::Plan { "plan" } else { "accept-edits" };
    agy.extend(["--mode".into(), mode.into()]);
    if let Some(conv) = spec.resume_session {
        agy.extend(["--conversation".into(), conv.into()]);
    }
    agy.extend(["-i".into(), remote_prompt(spec)]);
    agy.extend(spec.extra_args.iter().cloned());
    let args = if cfg!(target_os = "macos") {
        // BSD script: script [-q] file command...
        let mut a = vec!["-q".to_string(), "/dev/null".to_string()];
        a.extend(agy);
        a
    } else {
        // util-linux script: script -qfec "<command>" file
        let cmd = agy.iter().map(|s| shell_quote(s)).collect::<Vec<_>>().join(" ");
        vec!["-qfec".into(), cmd, "/dev/null".into()]
    };
    Invocation {
        program: "script".into(),
        args,
        env: vec![
            ("VENEER_TASK".into(), spec.task_id.into()),
            ("VENEER_RUN".into(), spec.run.into()),
            ("VENEER_HARNESS".into(), CLOUD_KIND.into()),
            ("TERM".into(), "xterm-256color".into()),
        ],
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::envelope::Task;
    use crate::net::envelope::tests::task;
    use std::path::Path;

    fn spec(t: &Task) -> LaunchSpec<'_> {
        LaunchSpec {
            task: t,
            task_id: "t42",
            run: "mac-17",
            worktree: Path::new("/tmp/none"),
            branch: "veneer/t42",
            dialogue: &[],
            resume_session: None,
            extra_args: &[],
            remote_project: Some("tasknet"),
        }
    }

    #[test]
    fn prompt_spells_out_identity_setup_and_push() {
        let t = task();
        let p = remote_prompt(&spec(&t));
        assert!(p.contains("VENEER_TASK=t42 VENEER_RUN=mac-17"));
        assert!(p.contains("bin/veneer-up"));
        assert!(p.contains("git push -u origin veneer/t42"));
        assert!(p.contains("veneer task complete"), "the ordinary leaf protocol follows");
    }

    #[test]
    fn runs_agy_remote_interactively_under_script() {
        let t = task();
        let inv = invocation(&spec(&t));
        assert_eq!(inv.program, "script");
        let all = inv.args.join(" ");
        assert!(all.contains("--remote-agent"));
        assert!(all.contains("--project") && all.contains("tasknet"));
        assert!(all.contains("-i"));
        assert!(!all.contains("--print") && !inv.args.iter().any(|a| a == "-p"));
        assert!(inv.env.iter().any(|(k, _)| k == "TERM"));
    }

    #[test]
    fn quoting_survives_single_quotes() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
