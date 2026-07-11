---
name: veneer
description: Minimal CTT-grounded harness managing the full task lifecycle (plan → implement → verify → ship) with deterministic law checking and gh-based ticket/PR flow. Use when starting, resuming, or shipping any coding task in a veneer-enabled project (one with a .veneer/ directory or veneer binary on PATH).
---

# veneer

You are operating inside a typed synthesis envelope. The `veneer` binary is the
judge; you are the prover. Trust your own planning; let the binary verify.

## The three laws

The laws are the operational form of veneer's CTT basis (`spec/basis.md`): types
are behavioral specifications, equality is structural, state change is a declared
effect. They are **language-polymorphic** — where the host compiler enforces them
(ADTs, signatures, borrow checking) let it; where it cannot (dynamic or untyped
languages) **you** enforce them as protocol: tagged unions with total dispatch and
no default fallthrough, a documented public surface, discriminated return values
in place of exceptions, freeze-at-boundaries in place of value semantics. The
discipline is identical in every language; only who checks it changes.

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
2. Consult only the "Phase:" section below matching the current phase.
3. Do the phase's work.
4. Run `veneer check --compact` and consume the findings (JSON on stdout).
5. Transition: `veneer state set <next-phase>`. Never bypass a refused
   transition — it is the protocol telling you what to do next.

| Phase | Section | Exit transition |
|---|---|---|
| plan | Phase: plan | `veneer state set implement` |
| implement | Phase: implement | `veneer state set verify` |
| verify | Phase: verify | `set implement` (findings) or `set ship` (clean) |
| ship | Phase: ship | `veneer state set plan` (new cycle) |

## Rules

- Never edit `.veneer/state.toon` by hand; go through `veneer state`.
  For a readable, parseable dump of the full state, run `veneer state get --json`.
- Never claim completion while `veneer check` reports Error findings.
- Findings are the repair signal: read `law`, `location`, `message`, fix
  precisely that (per-law guidance below), re-check.
- After 3 failed repair iterations on the same finding, stop and surface it
  to the user with the full finding JSON.

## The laws, operationally

**Finding schema** (what `veneer check --compact` emits, one JSON array on
stdout): `law` (module_budget | module_sealing | idempotency | protocol |
oxidation), `severity` (error | warning), `location` {path, line?}, `message`.
(Without `--compact`, findings also carry `suggested_fix` and render on stderr.)

**module_budget** — a module (file) exceeds the LoC band. Warning >500 is
pressure: prefer splitting when natural. Error >1000 blocks: split before
shipping. Split along behavioral seams: each new module gets one purpose, a
minimal public surface, and its own comprehensibility. Metadata files
(schemas, configs, docs) wrongly flagged as modules belong in the config's
`loc_exclude` list.

**module_sealing** — a file references another module's internal file.
Depend on the declared public surface (see `.veneer/config.toml` `[[modules]]`).
If the surface is missing something you legitimately need, widen the surface
deliberately (edit config + the module's signature), don't reach inside.

**idempotency** — a proposed diff applied twice diverges from applied once.
Anchor insertions to unique context lines so re-application fails cleanly.

**protocol** — you stepped outside the envelope (malformed intent, invalid
transition, stale ship gate, unreadable state). The message says exactly how
to get back in.

**oxidation** — rustc rejected the type or ownership (affine) story of your
proposed code, expressed as a Rust shadow skeleton. The `location.line` points
into the shadow you authored. Fix the *real* design the shadow models — keep the
shadow faithful to it, don't just patch the shadow to compile — then re-oxidize
(`veneer oxidize --file <shadow.rs>`). It is an on-demand check; running it
during implement (before writing the real code) or verify is encouraged.
The verdict is about the *shadow*, not your code: a clean oxidation proves the
modeled ownership story coheres, so a shadow that misrepresents the design gives
a false pass — faithfulness is your obligation, not the binary's
(`spec/oxidation.md`, N1). The lift is Rust-only; on non-Rust files this law
never fires, and the other four laws carry the discipline.

## Knowledge graph

The graph caches, per file: heuristic public signatures, a doc summary, LoC,
and a complexity score — any language the walker covers, best-effort. For
`.rs` files it also lifts those signatures into a generic, concrete-type-
erased Rust shadow (the *canonical form* — a per-module, language-uniform
rendering of what the module's contract owns and borrows) and runs it through
the same oxidation pipeline, attaching any real findings as that entry's
`semantic_findings`.

You do not maintain it by hand: a clean full `veneer check` refreshes it
automatically as a side effect, so it stays fresh once per cycle. Just read
from it — `veneer graph query <path>` reads the cache only (no re-walk) and
returns `{"entry": ..., "stale": bool}`. `stale: true` means the source
changed since the last refresh; run `veneer check` (or `veneer graph build`
to force it) to refresh. The graph has no bearing on `check` or the ship
gate — querying it, or ignoring it entirely, is always safe.

The graph cache self-heals: a corrupt or old-format cache reads as never-built and regenerates on the next clean check — never a finding.

**Token discipline — prefer the graph to raw source.** Whenever you need only a
dependency's *contract*, `veneer graph query <path>` and read its `signatures`
and `doc_summary` — a fraction of the tokens of the file. For `.rs` files the
entry's `semantic_findings` already carry the module's oxidation verdict, so you
can see its ownership story without re-oxidizing. Open full source only when the
entry is `stale`, missing, or the task genuinely needs an implementation rather
than a signature. Loading whole files to learn what a one-line signature would
have told you is the most common avoidable token cost in the loop.

## Phase: plan

Refine the request into a ticket. No artifact ceremony — the issue body is
the plan.

1. Decompose the request into first-principles modules/features: each one
   comprehensible from its signature and sources alone, sized to the
   500–1000 LoC band with a median near 500.
2. Express every dependency as a module signature (public surface), never an
   implementation. If you need to read code to plan, prefer signatures.
3. Write acceptance criteria: observable behavior, not implementation detail.
4. If `gh auth status` succeeds: create or update the GitHub Issue —
   `gh issue create --title <t> --body <plan>` (or `gh issue edit`) — then
   record it: `veneer state set plan --ref issue=<N>`.
   If gh is unavailable, proceed untracked.
5. When the plan is decomposed and criteria are written:
   `veneer state set implement`.

## Phase: implement

The synthesis envelope.

1. Load only the signatures of dependencies — not implementations. If a file
   you need exceeds the budget, that is the protocol telling you to read its
   public surface instead. Before reading a dependency's source, try
   `veneer graph query <path>` — if a fresh entry exists, its `signatures`/
   `doc_summary` are usually enough, at a fraction of the tokens.
2. Write the feature. Keep every touched module first-principles: one
   purpose, minimal public surface, within the LoC band.
3. Compose via the laws: sum types (or tagged equivalents) with exhaustive
   handling; errors as data; structural equality; mutation only as an
   explicit, declared effect.
4. Write tests alongside (test files are modules too).
5. Run `veneer check --compact` early and often; it is cheap, deterministic,
   and free on an unchanged clean tree.
6. Optional but encouraged: oxidize a Rust shadow skeleton of any non-trivial
   resource/ownership protocol (`veneer oxidize --file <shadow.rs>`) to catch
   use-after-move and aliasing bugs before they reach the real code.
7. When the feature is written and local tests pass:
   `veneer state set verify`.

## Phase: verify

The repair loop. Bounded: 3 iterations per finding, then surface to the user.

1. Run the project's own test suite first; fix failures.
2. Run `veneer check --compact`. Parse the JSON findings from stdout.
3. For each finding: read `law`, `location`, `message`. Fix exactly the
   named violation (per-law guidance above) — no drive-by refactoring.
4. Re-run `veneer check --compact`. A clean run records the ship-gate
   witness (tree + config); any edit to either goes stale, so check last.
5. Clean (exit 0, no error findings) → `veneer state set ship`.
   Findings persist after 3 honest attempts → stop, report the finding JSON
   and what you tried to the user.

## Phase: ship

The gate has already proven the tree clean; this phase is delivery.

1. `veneer state set ship` must already have succeeded (it is the gate). If
   it was refused, go back to verify — never work around it.
2. Branch and commit if not already done (`git switch -c <branch>`, commit
   with a message describing behavior, not process).
3. If `gh auth status` succeeds: `gh pr create --title <t> --body <b>`,
   linking the issue from state refs (`Closes #<issue>`). Then record it:
   `veneer state set ship --ref pr=<N>`.
4. Report the PR URL (or the branch name if untracked) to the user.
5. Start the next cycle when new work arrives: `veneer state set plan`.
