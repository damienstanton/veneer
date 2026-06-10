# Phase: plan

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
