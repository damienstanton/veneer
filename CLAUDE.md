# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo make build       # builds verifier/target/release/veneer
cargo make test        # unit, property, and integration tests
cargo make install     # installs the veneer binary to cargo's bin
cargo make self-check  # runs veneer check against this repo (uses the built binary)

# Run a single test
cargo test --manifest-path verifier/Cargo.toml <test_name>

# Run veneer's own MCP server (stdio)
veneer mcp
```

## Architecture

The repository has three layers:

**`spec/`** — normative contracts. `spec/veneer.md` is the harness contract (laws, lifecycle, finding schema, binary surface, state file). `spec/basis.md` is the CTT formal foundation. Read these before changing law semantics.

**`verifier/`** — the Rust binary (`veneer`). One crate, five modules:
- `kernel` — the CTT `Expr` ADT, gas-bounded `eval`, and `check_eq` (judgemental equality). Used only for lifting FNV hashes into canonical forms to verify idempotency. No law logic lives here.
- `laws` — the three laws as deterministic checks, plus the `Finding` value type, `Config` (from `.veneer/config.toml`), file walker, patch parser/applier, and `run_checks` orchestrator. The authoritative output surface: every veneer result is a `Finding`.
- `state` — the lifecycle state machine (Plan → Implement → Verify → Ship) with a content-hashed, crash-atomic state file. The ship gate: `set_phase(Verify→Ship)` only if `last_clean_check` matches the current `clean_hash`.
- `intent` — the `AgentIntent` ADT (expand_context / propose_diff / conclude) dispatched by `--intent` and MCP.
- `mcp` — thin MCP adapter over `laws` and `state` (`veneer_check` and `veneer_state` tools, served over stdio via `rmcp`).
- `main` — CLI dispatch only; no logic.

**`skill/veneer/SKILL.md`** — the agent-facing skill. `main.rs` embeds it with `include_str!` so it is always in sync with the binary. `veneer init` writes it to `.claude/skills/veneer/` and `.agents/skills/veneer/`.

## Key design invariants

- **Errors are data**: everything fails as a `Finding`, never a panic or naked exception.
- **Determinism**: `run_checks` on the same tree always produces the same findings. The `clean_hash` (FNV-1a over raw config bytes + tree hash) is the equality witness for the ship gate and the clean-tree short-circuit.
- **`--compact` vs full**: `--compact` and MCP always omit `suggested_fix` to save agent tokens. Per-law fix guidance lives in the skill, not in the binary output.
- **Walker skips**: `.git`, `.veneer`, `target`, `node_modules`, `.claude`, `.agents` (dirs); `*.lock`, `package-lock.json`, `pnpm-lock.yaml` (generated files). Lockfiles never count toward LoC budget or tree hash.
- **`loc_exclude`**: entries starting with `.` are extension suffixes; all others are root-relative path prefixes (include trailing `/` for directories). Excluded files still participate in sealing, idempotency, and the tree hash — only the budget check skips them.
- **State file integrity**: `.veneer/state.json` embeds an FNV-1a content hash (`"hash": "fnv:<hex>"`); `load` rejects mismatches as a Protocol finding. Never edit by hand; use `veneer state`.

## Self-check

`cargo make self-check` runs the built binary against this repo. Laws declared in `.veneer/config.toml`: `loc_soft=500`, `loc_hard=1000`, `loc_exclude=["docs/", "spec/"]`. No sealed modules are declared for this repo.
