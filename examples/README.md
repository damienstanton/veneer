# Building with veneer

veneer is a harness, not a language. Its judge is a Rust binary, but the
contracts it enforces — the [three laws](../skill/veneer/SKILL.md) and the
`plan → implement → verify → ship` lifecycle — are about *shape*, and shape is
language-agnostic. This directory shows how to use veneer to build well-formed
**AI agents** in TypeScript and Python.

## What carries across languages, and what doesn't

Four of veneer's five checks are language-agnostic — the walker covers any
file, and the laws reason about structure, not syntax:

| Check | What it enforces | Applies to TS / Python? |
|---|---|---|
| `module_budget` | ~500 LoC soft / 1000 LoC hard per file | **Yes** — any language |
| `module_sealing` | depend only on a module's declared public surface | **Yes** — declared in `.veneer/config.toml` |
| `idempotency` | re-applying a proposed diff twice is a no-op | **Yes** — pure diff check |
| `protocol` | you stayed inside the lifecycle envelope | **Yes** — the state machine is language-neutral |
| `oxidation` | rustc accepts the type/ownership story | **Rust only** — it lifts a Rust shadow, so it never fires for `.ts`/`.py` |

So a TypeScript or Python project gets the full lifecycle, the LoC budget,
module sealing, and diff idempotency. It does not get oxidation — that check is
the one place veneer reaches for `rustc`, and it simply does not run on
non-Rust files. Everything else in the loop is identical to using veneer on a
Rust codebase.

## The loop, applied to any project

The lifecycle is the same five moves regardless of language:

1. **plan** — decompose the work into first-principles modules, expressed by
   signature. Write acceptance criteria. Record the ticket.
2. **implement** — write the feature, composing via the laws. Run
   `veneer check --compact` early and often.
3. **verify** — run the project's own tests, then the repair loop: read each
   finding's `law`/`location`/`message`, fix exactly that, re-check.
4. **ship** — the gate has proven the tree clean; branch, commit, open the PR.
5. back to **plan** for the next cycle.

The binary is the judge; you are the prover. You do the decomposition and the
synthesis; veneer verifies that the result holds the laws.

## The two walkthroughs

Both build the **same agent** — a small tool-using Claude agent that answers
questions about a codebase, with three tools (`read_file`, `list_files`,
`grep`) — so you can read them side by side and see how the same first-
principles decomposition lands in each language's idioms.

- [Building a TypeScript agent](building-a-typescript-agent.md)
- [Building a Python agent](building-a-python-agent.md)

Each walkthrough is organized by the veneer loop and ends with a table mapping
**each law → its TypeScript idiom → its Python idiom**. Start with either; they
mirror each other section for section.

## Setting up veneer in your own project

```bash
veneer init          # writes .veneer/config.toml and the agent skill
veneer state get     # should report phase: plan (or run veneer init first)
```

Then edit `.veneer/config.toml` to set your LoC budget and declare any sealed
modules. The rest is the loop above. See [spec/veneer.md](../spec/veneer.md)
for the full harness contract.
