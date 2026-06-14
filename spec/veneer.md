# veneer Harness Contract

The normative contract between agents and the `veneer` binary. The formal
foundation is `spec/basis.md`.

## Laws

1. **Type-level constraint** — compose via sum/product types, sealed
   interfaces, explicit effects; errors are data; equality is structural.
2. **First-principles modules** — every module comprehensible from signature
   + sources alone. Proxy: warn above `loc_soft` (500), error above
   `loc_hard` (1000); configured in `.veneer/config.toml`. Entries in
   `loc_exclude` are exempt from the budget check only (extension entries
   like `".json"` match by suffix; all others are root-relative path
   prefixes like `"docs/"`); excluded files still participate in sealing,
   idempotency, and the tree hash.
3. **Total boundaries** — sealed modules (declared in config `[[modules]]`,
   `path` + `public` surface); idempotent operations.

## Lifecycle

`plan → implement → verify → ship`, `verify → implement` on findings,
`ship → plan` for the next cycle. Same-phase transitions are no-op successes.
`set ship` is gated: the recorded clean-check witness must match the current
one (see State file — the witness covers both the tree and the config).

## Finding schema (finding trace)

JSON array on stdout; human rendering on stderr.

    {
      "law": "module_budget" | "module_sealing" | "idempotency" | "protocol" | "oxidation",
      "severity": "error" | "warning",
      "location": { "path": string, "line"?: number },
      "message": string,            // deterministic for identical input
      "suggested_fix": string | null
    }

In compact output (`--compact`, and always over MCP) `suggested_fix` is
omitted; per-law fix guidance lives in the skill.

## Binary surface

    veneer init [--link <skill-src-dir>]   # config + skill into .claude/ and .agents/
    veneer check [--compact] [--diff <file>] [--intent <file>] [paths...]
    veneer oxidize [--compact] [--file <shadow.rs>]   # transient Rust type-check
    veneer state get|reset|set <phase> [--ref k=v ...]
    veneer mcp                              # veneer_check / veneer_state / veneer_oxidize over stdio

`--link <skill-src-dir>` creates a symlink from `.claude/skills/veneer` (and
`.agents/skills/veneer`) to the given source directory; without `--link` the
single skill file is written verbatim. Exit codes: 0 clean (warnings
permitted) · 1 error findings · 2 usage error.

`--compact` emits the findings JSON on stdout only (no stderr render) and
omits `suggested_fix` — the token-lean trace for agent consumption. MCP
findings are always compact. State responses (CLI and MCP) carry `phase` and
`refs` only; gate internals stay in the state file.

## Intent ADT

    {"intent": "expand_context", "query": <path>}     → file contents | findings
    {"intent": "propose_diff",   "patch": <unified>}  → findings
    {"intent": "conclude",       "summary": <text>}   → ship gate | findings
    {"intent": "oxidize",        "shadow": <rust>}    → findings (Law::Oxidation)

## State file

`.veneer/state.json`: `{phase, refs, last_clean_check, hash}` where `hash`
is the FNV-1a content hash of the rest — replayed writes converge; tampering
is detected as a protocol finding. Never edit by hand.

On the `ship → plan` transition, `last_clean_check` is cleared so that a stale
hash from a prior cycle cannot satisfy the ship gate of the next cycle; every
new cycle must earn a fresh clean check. Writes are crash-atomic: the new state
is written to a temporary file and renamed into place, so a partial write never
corrupts the existing state.

Note: the walker skips `.lock` files (generated artifacts), so lockfile-only edits neither count as modules nor stale the ship gate; their source manifests do.

`last_clean_check` stores the clean-check witness: an FNV-1a hash over the
raw config bytes and the tree hash. Editing `.veneer/config.toml` therefore
stales a recorded clean check, exactly like editing the tree — a verdict
earned under different rules does not transfer. A full `veneer check` with
no paths and no diff short-circuits when the current witness matches the
recorded one: the deterministic verdict is already known, so the laws are
not re-run and `[]` is emitted. A warning-only run records the witness too
(warnings are shippable), so a re-check of an unchanged tree reports `[]`;
warnings reappear on the next tree or config change.

## Oxidation

`veneer oxidize` lifts an agent-authored Rust *shadow skeleton* into the Rust
type system: the shadow is written to a persistent scratch crate
(`.veneer/oxidize/`, ignored via `.veneer/` and skipped by the walker), checked
with `cargo check --message-format=json`. The shadow is never retained as an
artifact — it is overwritten on the next run. rustc diagnostics
become `Law::Oxidation` findings (`location.path` is the stable label
`<shadow>`; `line` indexes the shadow). Two wall-clock caps from the `[oxidize]`
config section bound the run: `steady_timeout_ms` (default 2000) on warm
incremental checks and `cold_timeout_ms` (default 30000) on the one-time
scaffold prime; a timeout is a Protocol finding. Oxidation is a check within the
existing phases, not a new phase.

**Trust boundary.** Oxidation runs `cargo check` on the supplied shadow, and a
`cargo check` *executes code*: built-in macros (`include_str!`, `env!`) and any
procedural macros expand at check time and can read files or the environment and
surface that content in diagnostics. The shadow is therefore trusted input — at
the same level as the source the agent already writes and compiles in the
project. The `edition` is validated against a fixed allowlist (2015/2018/2021/
2024) so a config value cannot inject extra manifest sections, but the shadow
body itself is not sandboxed. Do not feed an untrusted party's Rust through the
`veneer_oxidize` MCP tool without an external sandbox.
