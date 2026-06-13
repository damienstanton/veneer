//! MCP surface: the same check/state code paths served as tools over stdio.
//! A thin adapter — no second implementation of anything.

use crate::laws::{findings_json_compact, load_config, run_checks, Finding, Law};
use crate::state::{load, set_phase, Phase};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, JsonSchema)]
pub struct CheckArgs {
    /// Restrict the budget check to these paths (default: whole tree).
    #[serde(default)]
    pub paths: Vec<String>,
    /// Unified diff to test for idempotency.
    pub diff: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct StateArgs {
    /// "get", "set", or "reset"
    pub action: String,
    /// Target phase for "set": plan | implement | verify | ship
    pub phase: Option<String>,
    /// External refs to record, e.g. {"issue": "42"}
    #[serde(default)]
    pub refs: std::collections::BTreeMap<String, String>,
}

#[derive(Clone)]
pub struct VeneerServer {
    root: PathBuf,
    #[allow(dead_code)] // used by #[tool_router] macro-generated dispatch code
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl VeneerServer {
    pub fn new(root: PathBuf) -> Self {
        Self { root, tool_router: Self::tool_router() }
    }

    #[tool(description = "Run the veneer laws (module budget, sealing, diff idempotency). Returns a JSON array of findings; empty means clean.")]
    fn veneer_check(&self, args: Parameters<CheckArgs>) -> CallToolResult {
        let cfg = match load_config(&self.root) {
            Ok(c) => c,
            Err(f) => {
                return CallToolResult::success(vec![Content::text(findings_json_compact(&[f]))])
            }
        };
        let paths: Vec<PathBuf> = args.0.paths.iter().map(PathBuf::from).collect();
        if args.0.diff.is_none() && paths.is_empty() {
            if let Ok(s) = crate::state::load(&self.root) {
                if s.last_clean_check == Some(crate::laws::clean_hash(&self.root)) {
                    return CallToolResult::success(vec![Content::text("[]".to_string())]);
                }
            }
        }
        let findings = run_checks(&self.root, &paths, args.0.diff.as_deref(), &cfg);
        CallToolResult::success(vec![Content::text(findings_json_compact(&findings))])
    }

    #[tool(description = "Read or transition the veneer lifecycle state (plan → implement → verify → ship). Invalid transitions and a stale ship gate return protocol findings.")]
    fn veneer_state(&self, args: Parameters<StateArgs>) -> CallToolResult {
        let a = args.0;
        let body = match a.action.as_str() {
            "get" => match load(&self.root) {
                Ok(s) => crate::state::public_json(&s),
                Err(f) => serde_json::to_string(&[f]).unwrap(),
            },
            "reset" => match set_phase(&self.root, Phase::Plan, &[]) {
                Ok(s) => crate::state::public_json(&s),
                Err(f) => serde_json::to_string(&[f]).unwrap(),
            },
            "set" => {
                let refs: Vec<(String, String)> = a.refs.into_iter().collect();
                match a.phase.as_deref().and_then(Phase::parse) {
                    None => serde_json::to_string(&[Finding::error(
                        Law::Protocol,
                        "<mcp>",
                        None,
                        "set requires phase: plan|implement|verify|ship",
                        None,
                    )])
                    .unwrap(), // infallible: plain derived structs
                    Some(p) => match set_phase(&self.root, p, &refs) {
                        Ok(s) => crate::state::public_json(&s),
                        Err(f) => serde_json::to_string(&[f]).unwrap(),
                    },
                }
            }
            _ => serde_json::to_string(&[Finding::error(
                Law::Protocol,
                "<mcp>",
                None,
                "action must be get|set|reset",
                None,
            )])
            .unwrap(),
        };
        CallToolResult::success(vec![Content::text(body)])
    }
}

#[tool_handler(
    name = "veneer",
    version = "0.1.0",
    instructions = "veneer verifier: run veneer_check for law findings; veneer_state to read/transition the lifecycle."
)]
impl ServerHandler for VeneerServer {}

/// Serve over stdio until the client disconnects. Returns a process exit code.
pub fn serve(root: PathBuf) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    rt.block_on(async {
        use rmcp::ServiceExt;
        match VeneerServer::new(root).serve(rmcp::transport::stdio()).await {
            Ok(service) => {
                let _ = service.waiting().await;
                0
            }
            Err(e) => {
                eprintln!("error: mcp serve failed: {e}");
                2
            }
        }
    })
}
