# Phase: implement

The synthesis envelope.

1. Load only the signatures of dependencies — not implementations. If a file
   you need exceeds the budget, that is the protocol telling you to read its
   public surface instead.
2. Write the feature. Keep every touched module first-principles: one
   purpose, minimal public surface, within the LoC band.
3. Compose via the laws: sum types (or tagged equivalents) with exhaustive
   handling; errors as data; structural equality; mutation only as an
   explicit, declared effect.
4. Write tests alongside (test files are modules too).
5. Run `veneer check` early and often; it is cheap and deterministic.
6. When the feature is written and local tests pass:
   `veneer state set verify`.
