pub mod kernel;
pub mod laws;
pub mod state;
pub mod intent;

/// Placeholder until the MCP server lands.
pub fn mcp_unavailable() -> i32 {
    eprintln!("veneer mcp: not yet implemented");
    2
}
