---
name: veneer
description: Minimal CTT-grounded harness managing the full task lifecycle (plan → implement → verify → ship) with deterministic law checking and gh-based ticket/PR flow. Use when starting, resuming, or shipping any coding task in a veneer-enabled project (one with a .veneer/ directory or veneer binary on PATH).
---

# veneer

You are operating inside a typed synthesis envelope. The `veneer` binary is the
judge; you are the prover. Trust your own planning; let the binary verify.

## The three laws

1. **Type-level constraint.** Compose via the host language's closest
   approximation of ADTs and sealed interfaces: sum types with exhaustive
   handling, products, explicit effects. Errors are data, never naked
   exceptions across module boundaries. Equality is structural.
2. **First-principles modules.** Anyone must be able to understand a module
   from its signature and sources alone. Target ~500 LoC per module; the
   verifier warns above 500 and errors above 1000. Grow systems by adding
   modules, never by growing them.
3. **Total boundaries.** Modules are sealed: depend only on declared public
   surfaces. Operations are idempotent: re-running any step is a no-op.

## The loop

1. Run `veneer state get` (if it fails, run `veneer init` first).
2. Read `references/<phase>.md` for the current phase — only that file.
3. Do the phase's work.
4. Run `veneer check` and consume the findings (JSON on stdout).
5. Transition: `veneer state set <next-phase>`. Never bypass a refused
   transition — it is the protocol telling you what to do next.

| Phase | Reference | Exit transition |
|---|---|---|
| plan | references/plan.md | `veneer state set implement` |
| implement | references/implement.md | `veneer state set verify` |
| verify | references/verify.md | `set implement` (findings) or `set ship` (clean) |
| ship | references/ship.md | `veneer state set plan` (new cycle) |

## Rules

- Never edit `.veneer/state.json` by hand; go through `veneer state`.
- Never claim completion while `veneer check` reports Error findings.
- Findings are the repair signal: read `law`, `location`, `message`,
  `suggested_fix`, fix precisely that, re-check.
- After 3 failed repair iterations on the same finding, stop and surface it
  to the user with the full finding JSON.
