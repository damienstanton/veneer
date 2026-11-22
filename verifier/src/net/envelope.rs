//! Handoff envelopes: what an orchestrator hands a leaf (`Task`) and what a
//! leaf hands back (`TaskResult`). Pure values; validation is data, not panics.

use crate::net::error::NetError;
use serde::{Deserialize, Serialize};

pub const TASK_SCHEMA: &str = "veneer.task.v1";
pub const RESULT_SCHEMA: &str = "veneer.result.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HarnessPref {
    Claude,
    Agy,
    #[default]
    Any,
}

impl HarnessPref {
    pub fn name(self) -> &'static str {
        match self {
            HarnessPref::Claude => "claude",
            HarnessPref::Agy => "agy",
            HarnessPref::Any => "any",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Where {
    Local,
    Cloud,
    #[default]
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Deliver {
    #[default]
    Pr,
    Branch,
    Report,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Permissions {
    /// Read-only; the leaf must not modify files.
    Plan,
    /// Edit files in the worktree; shell follows the harness's own approval rules.
    #[default]
    Edit,
    /// Full autonomy inside the worktree.
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub minutes: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
}

impl Default for Budget {
    fn default() -> Self {
        Budget { minutes: 60, max_turns: None }
    }
}

fn default_base() -> String {
    "main".into()
}

fn yes() -> bool {
    true
}

/// A leaf has none of the orchestrator's context, so `prompt` must stand alone
/// and every `acceptance` line must be checkable by the leaf itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub schema: String,
    pub title: String,
    /// `owner/name` of the repository the work happens in.
    pub repo: String,
    #[serde(default = "default_base")]
    pub base: String,
    #[serde(default)]
    pub harness: HarnessPref,
    #[serde(default, rename = "where")]
    pub location: Where,
    /// Pin to one replica (machine) by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// Capability tags the executing machine must have (`macos`, `xcode`, ...).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance: Vec<String>,
    #[serde(default)]
    pub deliver: Deliver,
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub permissions: Permissions,
    /// Task ids that must be done first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Require a passed veneer ship gate (a `clean_hash` witness) for `done`.
    #[serde(default = "yes")]
    pub gate: bool,
}

impl Task {
    pub fn validate(&self) -> Result<(), NetError> {
        let bad = |m: String| Err(NetError::Invalid(m));
        if self.schema != TASK_SCHEMA {
            return bad(format!("schema must be {TASK_SCHEMA}, got {}", self.schema));
        }
        if self.title.trim().is_empty() {
            return bad("title is empty".into());
        }
        let parts: Vec<&str> = self.repo.split('/').collect();
        if parts.len() != 2 || parts.iter().any(|p| p.is_empty()) {
            return bad(format!("repo must be owner/name, got {:?}", self.repo));
        }
        if self.prompt.trim().is_empty() {
            return bad("prompt is empty".into());
        }
        if self.budget.minutes == 0 || self.budget.minutes > 24 * 60 {
            return bad("budget.minutes must be within 1..=1440".into());
        }
        if self.harness == HarnessPref::Agy && self.location == Where::Cloud {
            return bad("agy cannot be a cloud leaf: Antigravity remote agents have no headless mode".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub criterion: String,
    pub pass: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Evidence that the leaf's tree passed veneer's ship gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Witness {
    /// veneer `clean_hash` of the shipped tree, hex.
    pub clean_hash: String,
    /// The commit the hash was taken on.
    pub commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_cache_reads: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskResult {
    pub schema: String,
    pub status: Outcome,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance: Vec<Check>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub witness: Option<Witness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<serde_json::Value>,
    /// False when the worker wrote this because the leaf exited without
    /// reporting. Orchestrators should review such work by hand.
    #[serde(default = "yes")]
    pub self_reported: bool,
}

impl TaskResult {
    pub fn new(status: Outcome, summary: impl Into<String>) -> Self {
        TaskResult {
            schema: RESULT_SCHEMA.into(),
            status,
            summary: summary.into(),
            branch: None,
            pr: None,
            acceptance: vec![],
            witness: None,
            harness: None,
            session_id: None,
            usage: None,
            self_reported: true,
        }
    }

    /// Validate against the task it answers.
    pub fn validate_for(&self, task: &Task) -> Result<(), NetError> {
        let bad = |m: &str| Err(NetError::Rejected(m.into()));
        if self.schema != RESULT_SCHEMA {
            return bad("result schema mismatch");
        }
        if self.summary.trim().is_empty() {
            return bad("result summary is empty");
        }
        if self.status == Outcome::Done {
            if self.acceptance.iter().any(|c| !c.pass) {
                return bad("done with a failing acceptance check; report failed or ask instead");
            }
            if task.gate && self.witness.is_none() {
                return bad("done without a veneer ship-gate witness; run veneer to ship first");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn task() -> Task {
        serde_json::from_value(serde_json::json!({
            "schema": TASK_SCHEMA, "title": "Add retry", "repo": "damienstanton/cm4-rs",
            "prompt": "Add exponential backoff to fetch().", "acceptance": ["cargo test passes"],
            "context": ["src/fetch.rs"], "budget": {"minutes": 20, "max_turns": 30}
        }))
        .unwrap()
    }

    pub fn witness() -> Witness {
        Witness { clean_hash: "fnv:00ff".into(), commit: "abc123".into(), cycle_cache_reads: Some(1000) }
    }

    #[test]
    fn defaults_fill_in() {
        let t: Task = serde_json::from_str(
            r#"{"schema":"veneer.task.v1","title":"t","repo":"a/b","prompt":"p"}"#,
        )
        .unwrap();
        assert_eq!(t.base, "main");
        assert_eq!(t.harness, HarnessPref::Any);
        assert_eq!(t.location, Where::Any);
        assert_eq!(t.permissions, Permissions::Edit);
        assert!(t.gate);
        t.validate().unwrap();
    }

    #[test]
    fn rejects_bad_tasks() {
        let mut t = task();
        t.repo = "nope".into();
        assert!(t.validate().is_err());
        let mut t = task();
        t.harness = HarnessPref::Agy;
        t.location = Where::Cloud;
        assert!(t.validate().is_err());
        let mut t = task();
        t.budget.minutes = 0;
        assert!(t.validate().is_err());
    }

    #[test]
    fn done_needs_witness_and_passing_checks() {
        let t = task();
        let mut r = TaskResult::new(Outcome::Done, "ok");
        assert!(r.validate_for(&t).is_err(), "no witness");
        r.witness = Some(witness());
        r.validate_for(&t).unwrap();
        r.acceptance = vec![Check { criterion: "x".into(), pass: false, note: None }];
        assert!(r.validate_for(&t).is_err());
        let mut ungated = task();
        ungated.gate = false;
        let r = TaskResult::new(Outcome::Done, "ok");
        r.validate_for(&ungated).unwrap();
        TaskResult::new(Outcome::Failed, "nope").validate_for(&t).unwrap();
    }
}
