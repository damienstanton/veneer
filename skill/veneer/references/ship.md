# Phase: ship

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
