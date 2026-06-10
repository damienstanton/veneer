# Phase: verify

The repair loop. Bounded: 3 iterations per finding, then surface to the user.

1. Run the project's own test suite first; fix failures.
2. Run `veneer check`. Parse the JSON findings from stdout.
3. For each finding: read `law`, `location`, `message`, `suggested_fix`.
   Fix exactly the named violation — no drive-by refactoring.
4. Re-run `veneer check`. A clean run records the tree hash for the ship
   gate; any edit after it goes stale, so check last.
5. Clean (exit 0, no error findings) → `veneer state set ship`.
   Findings persist after 3 honest attempts → stop, report the finding JSON
   and what you tried to the user.
