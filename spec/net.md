# veneer net: coordination contract

veneer net lets any number of agent sessions coordinate work and share memory,
whatever harness runs them (Claude Code, Antigravity) and wherever they run (a
local machine, a cloud session). It extends veneer's single-repo lifecycle to a
DAG of tasks across repos. This document is normative for `verifier/src/net/`.

## 1. Store

The store is an operation-based CRDT persisted as append-only logs.

- **Replica.** Each participant (a machine, or a cloud session) is a replica
  with a name matching `[A-Za-z0-9._-]{1,64}`, not starting with `.`.
- **Hub.** Logs live on one branch (default `veneer-net`) of any git remote.
  Replica `r` writes only `ops/r.jsonl`. Since no two replicas write the same
  file, every merge is conflict-free; sync is commit own file, rebase, push.
- **Ops.** One JSON object per line: `at` (a stamp) plus a tagged body
  (`op`): `task_create`, `task_claim`, `task_heartbeat`, `task_progress`,
  `task_ask`, `task_answer`, `task_complete`, `task_cancel`, `mem_put`,
  `mem_del`.
- **Stamps.** Hybrid logical clock `(wall_ms, counter, replica)`, totally
  ordered in that order. A replica observes every stamp in its own file before
  issuing one, so several processes sharing a replica name stay monotonic.
- **Load.** All files are read, lines that fail to parse or are stamped by a
  replica other than the file's owner are reported and skipped (never fatal),
  ops are sorted by stamp, and identical ops collapse.

**Convergence (N1).** State is a pure function of the set of ops: two replicas
holding the same ops compute the same tasks and memory, whatever order the ops
arrived in. Every rule below depends only on ops and their stamps.

## 2. Tasks

A task is created by `task_create` with an id derived from a stamp and the
envelope (`veneer.task.v1`; see `net/envelope.rs`). Ops for an unknown id are
orphans: reported, ignored.

Statuses: `queued`, `running`, `needs_input`, `done`, `failed`, `canceled`.
Folding applies ops in stamp order:

| Op | Allowed when | Effect |
|---|---|---|
| `task_claim(run, lease)` | queued | holder := run; running |
| `task_heartbeat(run)` | running, run holds | lease renewed at op time |
| `task_progress(run)` | running, run holds | note appended |
| `task_ask(run, q, session?)` | running, run holds | needs_input; holder cleared; session kept for resume |
| `task_answer(text)` | needs_input | queued; answer appended to the dialogue |
| `task_complete(run, result)` | running, run holds, result valid | done or failed |
| `task_cancel` | any non-terminal | canceled |

- **Leases.** Before applying an op at time `t`, a running task whose holder's
  last heartbeat plus lease is earlier than `t` loses its holder and returns to
  queued, or fails after 3 lapsed runs. Queries apply the same rule at the
  query time.
- **Claims (N2).** A CRDT cannot give mutual exclusion. Concurrent claims
  resolve to the earliest stamp on every replica; a later claimant sees its
  claim ignored after the next sync and must stop. Duplicate work is bounded
  by the sync interval; a result from a non-holder is never accepted.
- **Terminal states absorb** every later op.
- **Disallowed ops** are recorded on the task as `ignored`, with the reason.

## 3. Proof of done

A `done` result is accepted only if it validates against its task:

1. every acceptance check it reports passes, and
2. when the task is gated (`gate`, default true), it carries a **witness**:
   the veneer `clean_hash` of the leaf's tree, the commit, and that cycle's
   cache reads. `veneer task complete --status done` builds the witness and
   refuses unless the worktree's phase is `ship`, its `last_clean_check` equals
   the current `clean_hash`, and nothing outside `.veneer/`, `.claude/`, and
   `.agents/` is uncommitted.

The orchestrator can re-verify by checking out `witness.commit` and comparing
`clean_hash`.

## 4. Memory

`mem_put(scope, key, kind, value)` and `mem_del(scope, key)` fold into a
last-writer-wins map keyed by `(scope, key)`, keeping replaced values as
history and deleted values as tombstones. Scopes: `global`,
`repo:<owner/name>`, `task:<id>`. Kinds: `fact`, `decision`, `preference`,
`handoff`. `recall` returns global, then repo, then task entries.

## 5. Worker

`veneer net worker` loops: sync; pick the oldest queued task this machine fits
(harness available, `where`, pinned machine, `needs` tags, dependencies done);
claim; sync; continue only if still the holder. It then:

1. creates a worktree on `veneer/<id>` from `origin/<base>`, runs `veneer init`
   there (installing the skill for both harnesses), sets the state ref
   `task=<id>`, and writes `.veneer/task.json` with the task and run;
2. launches the harness headless (`claude -p` or `agy -p`, JSON output) with a
   prompt that names the leaf protocol;
3. heartbeats every quarter lease, and stops the harness when the budget runs
   out or the claim is lost;
4. if the leaf exited without reporting, records a result with
   `self_reported: false`: `done` only if the gate witness holds (and the
   branch is pushed), `failed` otherwise.

## 6. Configuration

Per machine, at `~/.config/veneer/net.toml` (or `VENEER_NET_CONFIG`): replica
name, hub remote and branch, capability tags, harnesses (detected when empty),
`local` (false for cloud replicas), a map of repos to local clones, work
directory, lease, poll interval, and extra harness arguments. `VENEER_REPLICA`
overrides the name.

## 7. Cloud sandboxes

A cloud sandbox (a Claude Code cloud session, an Antigravity Remote Agent) is
a replica with `local = false`: it takes `where = cloud` tasks and never
`local` ones. It is recognized by `CLAUDE_CODE_REMOTE=true` or declared with
`VENEER_CLOUD=<kind>`.

- **Names.** Unless `VENEER_REPLICA` is set, a cloud replica is named
  `<kind>-<last 12 alphanumerics of the session id>` (`VENEER_SESSION_ID`, then
  Claude Code's session ids), re-derived on every load. A container image can
  be snapshotted with a config file inside it; a fresh name per session keeps
  two sessions from ever writing the same op file.
- **No config file.** With `VENEER_NET_HUB` set and no config file, the config
  is derived from the environment (`VENEER_NET_BRANCH`, `VENEER_NET_TAGS`).
- **Self-claim.** A Claude Code cloud session is itself the agent, so it
  claims with `veneer task claim <id>` from the task's repo and follows the
  same leaf protocol and proof of done.
- **Dispatched remote agents.** A local replica whose config names an agy
  remote project (`agy_remote_project`) and has agy also takes `where = cloud`
  tasks that allow agy. It claims them, opens `agy --remote-agent -i <prompt>`
  in a pseudo-terminal (agy has no print mode for remote agents), and spells
  the task and run ids out in the prompt. The remote agent joins the hub as
  its own replica and reports on that run; since holding is by run, its ops
  are accepted. The worker pulls before each heartbeat, stops the terminal
  once it no longer holds the task, and records the usual fallback result if
  the lease is still its own when the terminal ends.
