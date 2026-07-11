# veneer

**A tiny verifier that turns "the agent says it's done" into "the tree
carries a witness that it's done."**

veneer is a minimal harness for agentic coding — Claude Code, Zed, or any
MCP client. One small Rust binary judges the work; one skill file teaches
the agent the loop. No daemon, no service: a `.veneer/` directory, three
laws, and a lifecycle.

## Why

LLM agents fail in stereotyped ways: modules balloon, boundaries erode,
errors hide in exceptions, and "done" is asserted rather than shown. Most
harnesses respond with more prompting. veneer responds with **verification**:
a deterministic binary that checks a small set of structural laws and refuses
to let a cycle ship until the tree itself proves clean.

The design is grounded in computational type theory (Harper's CTT: types are
specifications of *behavior*, verified by evaluation — see `spec/basis.md`)
and in linking types (Patterson–Wagner–Ahmed: typed mediation between
languages — see `spec/oxidation.md`).

## What is guaranteed — and what is not

Within its scope, veneer gives you guarantees, not vibes:

- **Determinism.** `veneer check` on the same tree and config always emits
  the same findings, bit for bit. Findings are data (typed JSON), never
  prose, never a panic.
- **The ship gate is a witness, not a convention.** Shipping requires that a
  content hash (FNV-1a over config bytes + tree) recorded by a clean check
  matches the tree *right now*. Edit one byte — code or config — and the
  gate goes stale. An agent cannot talk its way past it.
- **Ownership stories are checked by rustc.** "Oxidation" lifts public
  signatures (or an agent-authored shadow) into a type-erased Rust program
  preserving exactly the ownership/borrowing shape, and runs the real Rust
  compiler on it. A clean result means rustc — the strongest deployed affine
  type checker — found the modeled ownership story coherent, whatever
  language the source is written in.
- **The pipeline is total.** Every failure mode is a typed finding;
  the state file and graph cache are content-hashed and crash-atomic.

What veneer does **not** claim: that your code is correct. Oxidation judges
the shadow, and the shadow's fidelity to the real code is the agent's stated
obligation; signature extraction is heuristic; behavior is out of scope
(`todo!()` bodies prove ownership, not output). The honest boundary of every
claim is written down in `spec/oxidation.md` §5, with the test that witnesses
each clause.

## The three laws

1. **Type-level constraint** — compose via sum/product types and sealed
   interfaces; errors are data; equality is structural.
2. **First-principles modules** — every module comprehensible from signature
   + sources alone; warn > 500 LoC, error > 1000.
3. **Total boundaries** — sealed modules (declared public surfaces only) and
   idempotent operations (a patch applied twice equals once — judged by a
   small CTT kernel via evaluation to canonical form).

## The lifecycle

    plan → implement → verify → ship
              ↑           |
              └───────────┘  (findings send you back)

The agent runs `veneer check --compact`, consumes findings as JSON, repairs,
re-checks. `veneer state set ship` is refused unless the recorded clean-check
witness matches the current tree. Everything the agent needs to know lives in
one skill file the binary installs itself.

## Install

Requires Rust and [cargo-make](https://github.com/sagiegurari/cargo-make);
`gh` optional (enables the ticket/PR flow).

    cargo make install     # builds and installs the `veneer` binary
    cd <your-project>
    veneer init            # writes .veneer/ config + the skill (Claude Code & Zed)

Then invoke the `veneer` skill from your agent, or serve the same checks over
MCP with `veneer mcp` (tools: `veneer_check`, `veneer_state`,
`veneer_oxidize`, `veneer_graph`).

## The knowledge graph

A clean check refreshes `.veneer/graph.toon`: per file — public signatures, a
doc summary, LoC, complexity, and (for Rust) the lifted canonical form with
real rustc findings attached. `veneer graph query <path>` gives an agent a
module's contract at a fraction of the tokens of reading it. The graph is a
cache: orthogonal to the gate, self-healing, safe to delete.

## Upgrading

Any `.veneer/` directory written by an older veneer works with a newer one,
with zero manual steps: legacy `state.json` files are read and migrated to
TOON on the next write, and the graph cache self-heals and regenerates on the
next clean check. Re-run `veneer init` once after upgrading to refresh the
embedded skill (it never touches your config). This contract is enforced by
an end-to-end test (`verifier/tests/upgrade.rs`).

## Research grounding

- Harper, R. *Computational Type Theory* (OPLSS 2018) — the basis:
  types as behavioral specifications, equality by evaluation. `spec/basis.md`
  distills exactly the judgements veneer uses.
- Patterson, D., Wagner, A., Ahmed, A. *Semantic Encapsulation using Linking
  Types* (TyDe 2023) — the primary influence on oxidation: the shadow is a
  linking-types-style mediation into Rust's affine discipline.
  `spec/oxidation.md` states what the lift preserves and what the verdict
  means.

      @article{Patterson2023SemanticEU,
        title={Semantic Encapsulation using Linking Types},
        author={Daniel Patterson and Andrew Wagner and Amal J. Ahmed},
        journal={Proceedings of the 8th ACM SIGPLAN International Workshop
                 on Type-Driven Development},
        year={2023},
        url={https://api.semanticscholar.org/CorpusID:261395954}
      }

- Bauer, A. *Algebraic Effects and Handlers* — effects as declared
  operations, the discipline behind "mutation is a declared effect".

## Develop

    cargo make test        # unit, property, golden, and CLI tests
    cargo make self-check  # the harness obeys its own laws

Contract: `spec/veneer.md` · Formal basis: `spec/basis.md` · Oxidation:
`spec/oxidation.md`
