# veneer

**Agents say they're done. veneer makes the tree prove it.**

One small Rust binary judges the work, and one skill file teaches the agent the loop. It works with Claude Code, Zed, or any MCP client. There's no daemon and no service, just a `.veneer/` directory.

    plan → implement → verify → ship
              ↑           │
              └───────────┘  findings send you back

## What it does

- **A ship gate the agent can't argue with.** `veneer state set ship` is refused unless a clean check's content hash, taken over the tree and the config, matches the tree right now. Change one byte and the gate goes stale.
- **rustc checks the ownership story, whatever the language.** Oxidation lifts public signatures, or a shadow the agent writes, into type-erased Rust and runs the real compiler on it.
- **Three laws, checked deterministically.** Errors are data. Modules stay small and comprehensible (a warning past 500 LoC, an error past 1000). Boundaries are sealed and operations idempotent; idempotency is judged by a small CTT kernel. The same tree always gives the same findings, as typed JSON.
- **A knowledge graph instead of whole files.** `veneer graph query <path>` returns a module's contract (signatures, docs, its ownership verdict) for a fraction of the tokens it takes to read the file.
- **Cache reads made visible.** Every turn re-reads the whole context, so context × turns is the bill. `veneer usage` measures it from the agent's own transcript. A `--hook` mode warns the agent once it's over budget, and each shipped cycle records what it cost.

## Install

    cargo make install   # needs Rust + cargo-make
    cd <project> && veneer init

Then invoke the `veneer` skill, or serve the same tools over MCP with `veneer mcp`. To upgrade, re-run `veneer init`. An older `.veneer/` directory migrates on its own.

## What it saves

Here is one real workload: a developer's Claude Code logs covering 9,622 turns. They re-read **3.53B cache tokens**, an average context of 367K per turn, because two long-running sessions kept their context growing.

If the agent compacts or starts a fresh session whenever it passes the 150K budget, no turn re-reads more than 150K. That caps the same turns at 1.44B, so it saves **at least 2.09B tokens (59%)**. This is a floor: real compaction drops well below the budget, so the actual saving is larger.

| Cache-read price | Before | Saved, at least | Per turn, before → after |
|---|---|---|---|
| $0.50/MTok (Opus 5, Opus 4.x) | $1,765 | $1,043 | $0.18 → ≤ $0.075 |
| $0.20/MTok (Opus 5.5, Sonnet 5.5) | $706 | $417 | $0.07 → ≤ $0.03 |

These figures are API-equivalent; on a subscription plan the saving shows up as headroom against usage limits. Each fresh session re-caches the fixed context once, about 75K tokens of cache writes, which is small next to what it saves. `veneer usage` reports your own version of these numbers. Its `cache_read_over_budget` field is exactly this floor, per session.

## Measure your agent

    veneer usage --latest     # this session: turns, cache reads, mean/peak context
    veneer usage --cycles     # cost of each shipped cycle

To have the agent warned in-session once its context passes `[usage] budget_tokens` (default 150K), add this to `.claude/settings.json`:

    {"hooks": {"UserPromptSubmit": [{"hooks": [{"type": "command", "command": "veneer usage --hook"}]}]}}

## What it doesn't claim

veneer doesn't claim your code is correct. Oxidation judges the shadow, and keeping the shadow faithful to the real code is the agent's job. The exact boundary of every guarantee is in `spec/oxidation.md` §5.

## Learn more

- `spec/veneer.md`: the contract.
- `spec/basis.md`: the formal basis (Harper's *Computational Type Theory*).
- `spec/oxidation.md`: the lift and its soundness. It builds on Patterson, Wagner and Ahmed, *Semantic Encapsulation using Linking Types* (TyDe 2023).

To develop veneer itself: `cargo make test`, then `cargo make self-check` (veneer checks its own laws).
