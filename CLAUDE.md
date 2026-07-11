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

The repository has four layers:

**`spec/`** — normative contracts. `spec/veneer.md` is the harness contract (laws, lifecycle, finding schema, binary surface, state file). `spec/basis.md` is the CTT formal foundation. `spec/oxidation.md` is the lift/soundness spec for the Rust shadow check. Read these before changing law semantics.

**`verifier/`** — the Rust binary (`veneer`). One crate, eleven modules:
- `kernel` — the CTT `Expr` ADT, gas-bounded `eval`, and `check_eq` (judgemental equality). Used only for lifting FNV hashes into canonical forms to verify idempotency. No law logic lives here.
- `laws` — the three laws as deterministic checks, plus the `Finding` value type, `Config` (from `.veneer/config.toml`), file walker, and `run_checks` orchestrator. The authoritative output surface: every veneer result is a `Finding`.
- `patch` — unified-diff parse/apply over an in-memory tree; pure value
  transformations re-exported through `laws` for API stability.
- `state` — the lifecycle state machine (Plan → Implement → Verify → Ship) with a content-hashed, crash-atomic state file. The ship gate: `set_phase(Verify→Ship)` only if `last_clean_check` matches the current `clean_hash`.
- `oxidize` — transient Rust shadow type-check: writes an agent-authored (or graph-lifted) shadow to a persistent scratch crate and turns `cargo check` diagnostics into `Law::Oxidation` findings.
- `graph` — the codebase knowledge graph: heuristic per-file signatures/docs/LoC/complexity (any language), plus, for Rust files, semantic findings from running a lifted shadow (see `graph_lift`) through `oxidize`. Cached in `.veneer/graph.toon`, its own witness, orthogonal to the ship gate.
- `graph_lift` — crate-private: the generic-erasure engine that turns extracted `pub fn` signatures into a self-contained, type-erased Rust program preserving their ownership/borrowing shape. Re-exported as `graph::lift_shadow`; no dependency on `graph`'s persistence or extraction concerns.
- `wire` — crate-private ASCII armor for the TOON wire format, shared by `graph` and `state`: free text is whitelist-encoded (ASCII letters/digits/space/underscore pass through; empty maps to a bare `%` sentinel; leading digit-like bytes and everything else percent-escape) because toon-rust 0.1.3 mishandles many scalar shapes.
- `intent` — the `AgentIntent` ADT (expand_context / propose_diff / conclude / oxidize / query_graph) dispatched by `--intent` and MCP.
- `mcp` — thin MCP adapter over `laws`, `state`, `oxidize`, and `graph` (`veneer_check`, `veneer_state`, `veneer_oxidize`, `veneer_graph` tools, served over stdio via `rmcp`).
- `main` — CLI dispatch only; no logic.

**`skill/veneer/SKILL.md`** — the agent-facing skill. `main.rs` embeds it with `include_str!` so it is always in sync with the binary. `veneer init` writes it to `.claude/skills/veneer/` and `.agents/skills/veneer/`.

**`examples/`** — docs-first walkthroughs (prose, `loc_exclude`d) showing how to use veneer's language-agnostic laws and lifecycle to build TypeScript and Python AI agents. The same tool-using Claude agent is built in both languages so the law → idiom mapping is directly comparable.

## Key design invariants

- **Errors are data**: everything fails as a `Finding`, never a panic or naked exception.
- **Determinism**: `run_checks` on the same tree always produces the same findings. The `clean_hash` (FNV-1a over raw config bytes + tree hash) is the equality witness for the ship gate and the clean-tree short-circuit.
- **`--compact` vs full**: `--compact` and MCP always omit `suggested_fix` to save agent tokens. Per-law fix guidance lives in the skill, not in the binary output.
- **Walker skips**: `.git`, `.veneer`, `target`, `node_modules`, `.claude`, `.agents` (dirs); `*.lock`, `package-lock.json`, `pnpm-lock.yaml` (generated files). Lockfiles never count toward LoC budget or tree hash.
- **`loc_exclude`**: entries ending in `/` are root-relative directory path-prefixes (checked first, so a dot-prefixed directory like `.lake/` is a prefix match, not a suffix); entries starting with `.` are extension suffixes; all other entries are path prefixes. Excluded files still participate in sealing, idempotency, and the tree hash — only the budget check skips them.
- **State file integrity**: `.veneer/state.toon` (TOON-encoded) embeds an FNV-1a content hash (`hash: fnv:<hex>`) taken over the state's canonical JSON form, so the witness is format-independent; `load` rejects mismatches as a Protocol finding. A legacy `.veneer/state.json` is read as a fallback and migrated to TOON (legacy file removed) on the next write — seamless and invisible. `veneer state get --json` decodes the full stored state to readable JSON on demand. Never edit by hand; use `veneer state`.
- **Knowledge graph is orthogonal to the gate**: `.veneer/graph.toon` has its own integrity witness and its own staleness witness (`built_from`, a tree hash) — both independent of `clean_hash`. It is self-healing: a malformed, old-version, or hash-mismatched graph file reads as never-built — a Protocol finding only for real IO errors — and the on-disk document carries a `version: 1` wire field so an incompatible format is recognized rather than misparsed. A clean full `veneer check` rebuilds it automatically (`graph::rebuild`, best-effort, right after `record_clean_check`) so it stays fresh once per cycle without an explicit `graph build`. `check` never *reads* it; this write-only refresh, and building/rebuilding/deleting the file, never affect findings or the ship gate — those are computed from `run_checks` and `clean_hash` alone, before the refresh, with its errors swallowed.
- **`toon-rust` 0.1.3 mishandles many scalar shapes** — property testing found six distinct bug classes: multi-byte UTF-8 decoder offset corruption (a single em dash in a doc comment corrupts parsing of every later line, and some sequences panic outright); empty strings lost from array/tabular encodings; literal double quotes corrupting document syntax; digit-leading strings mistyped as numbers; structural punctuation corrupting or dropping entries; and space-edged or backslash-containing tabular cells dropped. All worked around in one place: the `wire` module's whitelist encoding, applied to all free text in `.veneer/graph.toon` and to `.veneer/state.toon` refs — never in public shapes or agent-facing JSON. Separately, `toon-rust` cannot tabular-encode an array of objects with any non-primitive field (e.g. `Finding.location`), so `GraphEntry.semantic_findings` is flattened to a private `FlatFinding`.

## Self-check

`cargo make self-check` runs the built binary against this repo. Laws declared in `.veneer/config.toml`: `loc_soft=500`, `loc_hard=1000`, `loc_exclude=["docs/", "spec/", "examples/"]`. No sealed modules are declared for this repo.
