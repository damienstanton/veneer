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
    veneer graph build [--compact] | query [--compact] <path>
    veneer state get [--json]|reset|set <phase> [--ref k=v ...]
    veneer mcp                              # veneer_check / veneer_state / veneer_oxidize / veneer_graph over stdio

`--link <skill-src-dir>` creates a symlink from `.claude/skills/veneer` (and
`.agents/skills/veneer`) to the given source directory; without `--link` the
single skill file is written verbatim. Exit codes: 0 clean (warnings
permitted) · 1 error findings · 2 usage error.

`init` is idempotent and is the only manual step when upgrading an existing
project to a new veneer version: it rewrites the embedded skill only when its
content changed and leaves an existing `.veneer/config.toml` untouched.
Everything else is automatic — state files migrate to the current format on
their next write, and the knowledge graph rebuilds on the next clean `check`.

`--compact` emits the findings JSON on stdout only (no stderr render) and
omits `suggested_fix` — the token-lean trace for agent consumption. MCP
findings are always compact. State responses (CLI and MCP) carry `phase` and
`refs` only; gate internals stay in the state file. `veneer state get --json`
is the on-demand exception: it decodes the full stored state (including
`last_clean_check`) to readable, parseable JSON for inspection.

## Intent ADT

    {"intent": "expand_context", "query": <path>}     → file contents | findings
    {"intent": "propose_diff",   "patch": <unified>}  → findings
    {"intent": "conclude",       "summary": <text>}   → ship gate | findings
    {"intent": "oxidize",        "shadow": <rust>}    → findings (Law::Oxidation)
    {"intent": "query_graph",    "query": <path>}     → {entry, stale}

## State file

`.veneer/state.toon`: `{phase, refs, last_clean_check, hash}` encoded as TOON
(token-efficient JSON) where `hash` is the FNV-1a content hash of the logical
state — taken over its canonical JSON form, so the witness is format-independent
and survives migration. Replayed writes converge; tampering is detected as a
protocol finding. `last_clean_check` is stored as a quoted decimal string so the
full-width u64 round-trips through TOON exactly. Never edit by hand.

A project written by an older veneer carries a legacy `.veneer/state.json`.
`load` reads either file (TOON preferred, JSON fallback, decoded identically);
the next state-mutating write produces `.veneer/state.toon` and removes the
legacy JSON. Migration is seamless and invisible.

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

## Knowledge graph

`veneer graph build` walks the tree (the same walker `check` uses — `.veneer/`,
`target/`, etc. are skipped) and extracts, per file: heuristic public
signatures, a leading doc-comment summary, LoC, and a branch-keyword
complexity score. Heuristic, not an AST — honest about being structural, not
a real parser; the marker table is per-extension (Rust/Python/TypeScript today),
empty markers ⇒ no signatures for that file (LoC/complexity still apply).

For `.rs` files specifically, the extracted signatures are *lifted*: every
project-specific type is generic-erased to a fresh type parameter, consistently
within each signature, preserving its ownership/borrowing shape (`&`, `&mut`,
`Vec<_>`, `Option<_>`, `Box<_>`, ...) while erasing concrete names not
resolvable in a bare scratch crate. The result — the *canonical form* — is a
self-contained, generic Rust rendering of "what this module's public contract
owns and borrows": denser than source, language-uniform regardless of the
original source language, and itself a per-module domain-specific type theory
for the project. Running it through the **existing** `oxidize::oxidize()` (no
second semantic-analysis engine) attaches any real rustc-grade findings —
lifetime ambiguity, trait-bound failures, anything rustc itself would catch —
as that entry's `semantic_findings`.

Output is `.veneer/graph.toon` (TOON-encoded), with its own FNV-1a content-hash
witness and its own staleness witness (`built_from`, a tree hash) — both fully
independent of `.veneer/state.toon` and the ship-gate `clean_hash`. The graph
is never read by `check`, never gates a transition; a missing or stale graph
is a query-time concern only. Building/rebuilding it is idempotent: an
unchanged tree always produces a byte-identical `.veneer/graph.toon` (`semantic_findings`
included — cargo's diagnostics for a fixed shadow are deterministic).

The graph is maintained automatically: a clean full `veneer check` (no paths,
no diff) refreshes it as a best-effort side effect, immediately after
recording the ship-gate witness. This keeps it fresh once per cycle (a cycle
cannot ship without a clean check) with no explicit `graph build`. The
refresh is write-only — `check`'s findings and exit code are computed before
it and are byte-for-byte identical whether the rebuild succeeds, fails, or is
skipped; failures are swallowed. The clean-tree short-circuit returns before
the refresh, so an unchanged tree never pays the cost. `veneer graph build`
remains for a forced/manual rebuild.

`veneer graph query <path>` (CLI, MCP `veneer_graph`, and the
`query_graph` intent) returns `{"entry": <GraphEntry|null>, "stale": bool}` —
a read against the cache only, no re-walk or re-extraction. `--compact`/MCP
strip `suggested_fix` from any nested `semantic_findings`, same as a top-level
findings array.
