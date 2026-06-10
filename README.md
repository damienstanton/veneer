# veneer

A minimal agentic harness for Claude Code and Zed, built for looped,
long-horizon models. One skill manages the whole lifecycle
(plan → implement → verify → ship); one small Rust binary judges it.

Grounded in computational type theory (`spec/basis.md`): the binary enforces
three laws — type-level composition, first-principles modules (~500 LoC
median), and total (sealed, idempotent) boundaries — and emits deterministic
typed findings that the model consumes to repair its own work.

## Install

Requires Rust and cargo-make; `gh` optional (enables the ticket/PR flow).

    cargo make install     # builds and installs the `veneer` binary
    cd <your-project>
    veneer init            # writes .veneer/ config + the skill for both hosts

Then invoke the `veneer` skill from Claude Code or Zed.

## Develop

    cargo make test        # unit, property, golden, and CLI tests
    cargo make self-check  # the harness obeys its own laws

Contract: `spec/veneer.md` · Formal basis: `spec/basis.md`
