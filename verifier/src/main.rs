//! CLI dispatch: thin wrappers over the intent/laws/state modules.
//! Exit codes: 0 clean (warnings allowed) · 1 error findings · 2 usage error.

use std::path::{Path, PathBuf};
use veneer::intent::{parse_intent, execute, Outcome};
use veneer::laws::{clean_hash, findings_json_compact, load_config, run_checks, Finding, Severity};
use veneer::state::{load, record_clean_check, set_phase, Phase};

const USAGE: &str = "\
veneer — minimal CTT-grounded agentic harness
USAGE:
  veneer init [--link <skill-src-dir>]
  veneer check [--compact] [--diff <patch-file>] [--intent <intent.json>] [paths...]
  veneer state get | set <phase> [--ref k=v ...] | reset
  veneer mcp
";

// Embedded skill (kept in sync by include_str! — a build error if files move).
const SKILL_FILES: &[(&str, &str)] = &[
    ("SKILL.md", include_str!("../../skill/veneer/SKILL.md")),
];

const DEFAULT_CONFIG: &str = "\
# veneer configuration — see spec/veneer.md
loc_soft = 500
loc_hard = 1000

# Exclude file types and directories from the LoC budget check:
# loc_exclude = [\".json\", \".yaml\", \"docs/\"]

# Declare sealed modules:
# [[modules]]
# path = \"src/core\"
# public = [\"api.rs\"]
";

fn emit(findings: &[Finding]) -> i32 {
    println!("{}", serde_json::to_string(findings).unwrap());
    for f in findings {
        let sev = match f.severity { Severity::Error => "error", Severity::Warning => "warning" };
        let law = serde_json::to_value(&f.law).unwrap();
        eprintln!(
            "{sev} [{}] {}: {}{}",
            law.as_str().unwrap_or("?"),
            f.location.path,
            f.message,
            f.suggested_fix.as_deref().map(|s| format!(" — fix: {s}")).unwrap_or_default()
        );
    }
    if findings.iter().any(|f| f.severity == Severity::Error) { 1 } else { 0 }
}

fn emit_compact(findings: &[Finding]) -> i32 {
    println!("{}", findings_json_compact(findings));
    if findings.iter().any(|f| f.severity == Severity::Error) { 1 } else { 0 }
}

fn cmd_init(root: &Path, link: Option<&str>) -> i32 {
    let cfg_path = root.join(".veneer/config.toml");
    if !cfg_path.exists() {
        if std::fs::create_dir_all(root.join(".veneer")).is_err()
            || std::fs::write(&cfg_path, DEFAULT_CONFIG).is_err()
        {
            eprintln!("error: cannot create .veneer/");
            return 2;
        }
    }
    for host in [".claude/skills/veneer", ".agents/skills/veneer"] {
        let dest = root.join(host);
        if let Some(src) = link {
            let Ok(src_abs) = std::fs::canonicalize(src) else {
                eprintln!("error: --link source not found: {src}");
                return 2;
            };
            let _ = std::fs::create_dir_all(dest.parent().unwrap());
            if !dest.exists() {
                #[cfg(unix)]
                if std::os::unix::fs::symlink(&src_abs, &dest).is_err() {
                    eprintln!("error: cannot symlink {host}");
                    return 2;
                }
                #[cfg(not(unix))]
                {
                    let _ = &src_abs;
                    eprintln!("error: --link requires a unix platform (symlinks); run `veneer init` without --link");
                    return 2;
                }
            }
        } else {
            for (rel_path, contents) in SKILL_FILES {
                let p = dest.join(rel_path);
                let _ = std::fs::create_dir_all(p.parent().unwrap());
                let already_written = std::fs::read_to_string(&p)
                    .map(|existing| existing == *contents)
                    .unwrap_or(false);
                if !already_written && std::fs::write(&p, contents).is_err()
                {
                    eprintln!("error: cannot write {}", p.display());
                    return 2;
                }
            }
        }
    }
    eprintln!("veneer initialized (config: .veneer/config.toml, skill: .claude + .agents)");
    0
}

fn cmd_check(root: &Path, args: &[String]) -> i32 {
    let mut diff: Option<String> = None;
    let mut intent: Option<String> = None;
    let mut compact = false;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--compact" => {
                compact = true;
                i += 1;
            }
            "--diff" | "--intent" => {
                let flag = args[i].clone();
                let Some(file) = args.get(i + 1) else {
                    eprintln!("{USAGE}");
                    return 2;
                };
                let Ok(contents) = std::fs::read_to_string(file) else {
                    eprintln!("error: cannot read {file}");
                    return 2;
                };
                if flag == "--diff" {
                    diff = Some(contents);
                } else {
                    intent = Some(contents);
                }
                i += 2;
            }
            p => {
                paths.push(PathBuf::from(p));
                i += 1;
            }
        }
    }
    let emit_findings = |fs: &[Finding]| if compact { emit_compact(fs) } else { emit(fs) };
    let cfg = match load_config(root) {
        Ok(c) => c,
        Err(f) => return emit_findings(&[f]),
    };
    if let Some(contents) = intent {
        return cmd_intent(root, &contents, &cfg);
    }
    // Clean-tree short-circuit: same config bytes + same tree ⇒ the
    // deterministic verdict is already recorded; skip the law checks.
    // Note: a warning-only run records too (warnings are shippable), so a
    // re-check of an unchanged tree returns [] — warning dedup by design.
    if diff.is_none() && paths.is_empty() {
        if let Ok(s) = load(root) {
            if s.last_clean_check == Some(clean_hash(root)) {
                return emit_findings(&[]);
            }
        }
    }
    let findings = run_checks(root, &paths, diff.as_deref(), &cfg);
    let code = emit_findings(&findings);
    if code == 0 && diff.is_none() && paths.is_empty() {
        // A clean full check records the clean_hash for the ship gate.
        if let Err(f) = record_clean_check(root, clean_hash(root)) {
            eprintln!("error [protocol] {}: {}", f.location.path, f.message);
            return 1;
        }
    }
    code
}

fn cmd_intent(root: &Path, contents: &str, cfg: &veneer::laws::Config) -> i32 {
    match parse_intent(contents) {
        Err(f) => emit(&[f]),
        Ok(intent) => match execute(root, intent, cfg) {
            Outcome::Context(c) => {
                println!("{c}");
                0
            }
            Outcome::Findings(fs) => emit(&fs),
            Outcome::Concluded(summary) => {
                println!("{}", serde_json::json!({ "concluded": summary }));
                0
            }
        },
    }
}

fn cmd_state(root: &Path, args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("get") => match load(root) {
            Ok(s) => {
                println!("{}", veneer::state::public_json(&s));
                eprintln!("phase: {}", s.phase.name());
                0
            }
            Err(f) => emit(&[f]),
        },
        Some("reset") => match set_phase(root, Phase::Plan, &[]) {
            Ok(_) => 0,
            Err(_) => {
                // Corrupt state: reset must still work — rewrite the default.
                let s = veneer::state::State::default();
                match veneer::state::store(root, &s) {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("error: {e}");
                        2
                    }
                }
            }
        },
        Some("set") => {
            let Some(phase) = args.get(1).and_then(|p| Phase::parse(p)) else {
                eprintln!("{USAGE}");
                return 2;
            };
            let mut refs = Vec::new();
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--ref" {
                    let Some((k, v)) = args.get(i + 1).and_then(|r| r.split_once('=')) else {
                        eprintln!("{USAGE}");
                        return 2;
                    };
                    refs.push((k.to_string(), v.to_string()));
                    i += 2;
                } else {
                    eprintln!("{USAGE}");
                    return 2;
                }
            }
            match set_phase(root, phase, &refs) {
                Ok(s) => {
                    println!("{}", veneer::state::public_json(&s));
                    0
                }
                Err(f) => emit(&[f]),
            }
        }
        _ => {
            eprintln!("{USAGE}");
            2
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = std::env::current_dir().expect("cwd");
    let code = match args.first().map(String::as_str) {
        Some("init") => {
            if args.get(1).map(String::as_str) == Some("--link") {
                match args.get(2) {
                    Some(src) => cmd_init(&root, Some(src.as_str())),
                    None => {
                        eprintln!("{USAGE}");
                        2
                    }
                }
            } else {
                cmd_init(&root, None)
            }
        }
        Some("check") => cmd_check(&root, &args[1..]),
        Some("state") => cmd_state(&root, &args[1..]),
        Some("mcp") => veneer::mcp::serve(root.clone()),
        _ => {
            eprintln!("{USAGE}");
            2
        }
    };
    std::process::exit(code);
}
