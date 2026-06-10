# veneer Harness Contract

The normative contract between agents and the `veneer` binary. The formal
foundation is `spec/basis.md`; the design rationale is
`docs/superpowers/specs/2026-06-10-veneer-design.md`.

## Laws

1. **Type-level constraint** — compose via sum/product types, sealed
   interfaces, explicit effects; errors are data; equality is structural.
2. **First-principles modules** — every module comprehensible from signature
   + sources alone. Proxy: warn above `loc_soft` (500), error above
   `loc_hard` (1000); configured in `.veneer/config.toml`.
3. **Total boundaries** — sealed modules (declared in config `[[modules]]`,
   `path` + `public` surface); idempotent operations.

## Lifecycle

`plan → implement → verify → ship`, `verify → implement` on findings,
`ship → plan` for the next cycle. Same-phase transitions are no-op successes.
`set ship` is gated: the recorded clean-check tree hash must match the
current tree.

## Finding schema (finding trace)

JSON array on stdout; human rendering on stderr.

    {
      "law": "module_budget" | "module_sealing" | "idempotency" | "protocol",
      "severity": "error" | "warning",
      "location": { "path": string, "line"?: number },
      "message": string,            // deterministic for identical input
      "suggested_fix": string | null
    }

## Binary surface

    veneer init [--link <skill-src-dir>]   # config + skill into .claude/ and .agents/
    veneer check [--diff <file>] [--intent <file>] [paths...]
    veneer state get|reset|set <phase> [--ref k=v ...]
    veneer mcp                              # veneer_check / veneer_state over stdio

`--link <skill-src-dir>` creates a symlink from `.claude/skills/veneer` (and
`.agents/skills/veneer`) to the given source directory; without `--link` the
skill files are written verbatim. Exit codes: 0 clean (warnings permitted) ·
1 error findings · 2 usage error.

## Intent ADT

    {"intent": "expand_context", "query": <path>}     → file contents | findings
    {"intent": "propose_diff",   "patch": <unified>}  → findings
    {"intent": "conclude",       "summary": <text>}   → ship gate | findings

## State file

`.veneer/state.json`: `{phase, refs, last_clean_check, hash}` where `hash`
is the FNV-1a content hash of the rest — replayed writes converge; tampering
is detected as a protocol finding. Never edit by hand.

On the `ship → plan` transition, `last_clean_check` is cleared so that a stale
hash from a prior cycle cannot satisfy the ship gate of the next cycle; every
new cycle must earn a fresh clean check. Writes are crash-atomic: the new state
is written to a temporary file and renamed into place, so a partial write never
corrupts the existing state.
