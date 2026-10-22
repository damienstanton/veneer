# Changelog

## 1.0.0 (2026-10-22)

First stable release.

### Features

* **ship gate**: `veneer state set ship` is refused unless a clean check's content hash matches the tree right now.
* **three laws**: errors as data, small comprehensible modules, sealed boundaries and idempotent operations, checked deterministically.
* **oxidation**: lifts public signatures, or an agent-written shadow, into type-erased Rust and runs `cargo check` on it.
* **knowledge graph**: `veneer graph query <path>` returns a module's contract for a fraction of the tokens of reading the file.
* **usage**: `veneer usage` measures cache reads from the agent's own transcript, warns past a context budget (`--hook`), and records each shipped cycle's cost in `.veneer/usage.jsonl`.
* **MCP server**: `veneer mcp` serves `veneer_check`, `veneer_state`, `veneer_oxidize`, `veneer_graph` and `veneer_usage` over stdio.
