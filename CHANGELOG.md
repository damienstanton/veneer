# Changelog

## [1.1.0](https://github.com/damienstanton/veneer/compare/v1.0.0...v1.1.0) (2026-10-22)


### Features

* **usage:** measure agent cache reads and record each cycle's cost ([#18](https://github.com/damienstanton/veneer/issues/18)) ([3835d1b](https://github.com/damienstanton/veneer/commit/3835d1b7aa4d0fc505f702f369d8a2a6bbf70760))


### Bug Fixes

* **mcp:** resolve memory leak by upgrading rmcp to 3.5 ([#17](https://github.com/damienstanton/veneer/issues/17)) ([ad5320d](https://github.com/damienstanton/veneer/commit/ad5320d2c45e50004cc9feb36de77c119b42e21f))

## 1.0.0 (2026-10-22)

First stable release.

### Features

* **ship gate**: `veneer state set ship` is refused unless a clean check's content hash matches the tree right now.
* **three laws**: errors as data, small comprehensible modules, sealed boundaries and idempotent operations, checked deterministically.
* **oxidation**: lifts public signatures, or an agent-written shadow, into type-erased Rust and runs `cargo check` on it.
* **knowledge graph**: `veneer graph query <path>` returns a module's contract for a fraction of the tokens of reading the file.
* **usage**: `veneer usage` measures cache reads from the agent's own transcript, warns past a context budget (`--hook`), and records each shipped cycle's cost in `.veneer/usage.jsonl`.
* **MCP server**: `veneer mcp` serves `veneer_check`, `veneer_state`, `veneer_oxidize`, `veneer_graph` and `veneer_usage` over stdio.
