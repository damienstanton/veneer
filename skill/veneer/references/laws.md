# The laws, operationally

**Finding schema** (what `veneer check` emits, one JSON array on stdout):
`law` (module_budget | module_sealing | idempotency | protocol), `severity`
(error | warning), `location` {path, line?}, `message`, `suggested_fix`.

**module_budget** — a module (file) exceeds the LoC band. Warning >500 is
pressure: prefer splitting when natural. Error >1000 blocks: split before
shipping. Split along behavioral seams: each new module gets one purpose, a
minimal public surface, and its own comprehensibility.

**module_sealing** — a file references another module's internal file.
Depend on the declared public surface (see `.veneer/config.toml` `[[modules]]`).
If the surface is missing something you legitimately need, widen the surface
deliberately (edit config + the module's signature), don't reach inside.

**idempotency** — a proposed diff applied twice diverges from applied once.
Anchor insertions to unique context lines so re-application fails cleanly.

**protocol** — you stepped outside the envelope (malformed intent, invalid
transition, stale ship gate, unreadable state). The message says exactly how
to get back in.
