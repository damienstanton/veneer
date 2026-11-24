# Setting up veneer net with a private state repository

veneer net keeps its task queue and shared memory in a git repository you own, the **hub**. This page is for people setting one up, and for agents helping them. The contract is `spec/net.md`; the commands are in the veneer skill.

The hub holds task prompts, results and agent memory, so make it **private**. It needs no code and no CI.

## 1. Create the hub

1. Create an empty private repository, for example `you/agent-hub`. Do not add a README or any other file.
2. Optional but recommended: add a bootstrap on `main` that installs veneer and joins the hub for you (a script, an `AGENTS.md`, and a Claude Code SessionStart hook). [damienstanton/tasknet](https://github.com/damienstanton/tasknet) is a working example: `bin/veneer-up`, `AGENTS.md`, `.claude/hooks/session-start.sh`.

veneer writes only to the branch `veneer-net` (override with `VENEER_NET_BRANCH`), one file per machine (`ops/<replica>.jsonl`). It creates the branch on first use, and nothing else in the repository is touched. Never edit that branch by hand.

## 2. Give each place push access to the hub

Every machine or sandbox that joins must be able to `git push` to the hub. How depends on where it runs.

| Where | How it authenticates | Notes |
|---|---|---|
| A Mac or Linux workstation | its existing git login (`gh auth login`, SSH key, or keychain) | nothing to add; `veneer net init` uses the same git |
| Claude Code cloud session | attach the hub repository to the session or environment | verified: a session with the hub attached joins and pushes with no token |
| Antigravity (`agy`) running locally | the workstation's git login | verified: the worker pushes a headless `agy -p` leaf's branch |
| Antigravity Remote Agent | not supported yet | agy 1.5.14 refuses `--remote-agent` with `-p`, and its CLI has no way to attach repos or secrets; veneer dispatch for it exists but is untested end to end |
| Any other cloud VM or CI runner | a token in the environment | see below |

### Tokens, for VMs and runners only

Prefer a **fine-grained personal access token**, never a classic one, because a classic `repo` token reaches every private repository you own.

- **Resource owner:** the owner of the repository. A fine-grained token belongs to one owner, so repositories in two owners (a user and an organization) need two tokens. An organization may need to allow fine-grained tokens and approve each one (Organization settings, Personal access tokens).
- **Repository access:** only the hub and the repositories agents will work on.
- **Permissions:** Contents read and write (Metadata read is added for you). Add Pull requests read and write only if agents should open pull requests. Leave Workflows off unless agents must edit `.github/workflows`.
- **Expiry:** 30 to 90 days; rotate before it lapses.

Expose tokens as environment variables, never in a prompt, a file in a repository, or a chat message. veneer never needs the value. The tasknet `bin/veneer-up` shows one way to use them inside a sandbox: a git credential helper that reads `GH_TOKEN_<OWNER>` (for example `GH_TOKEN_ACME`), then `GH_TOKEN`, at the moment git asks, so nothing is written to disk. It installs the helper only inside a cloud sandbox: on a workstation a global helper would route every push through one narrow token.

## 3. Join

On each machine or sandbox, once:

```sh
veneer net init --hub git@github.com:you/agent-hub.git --name my-mac --tags macos --harness claude,agy
veneer net status        # online: true, problems: []
```

The config is `~/.config/veneer/net.toml` (override with `VENEER_NET_CONFIG`). It holds no secrets.

In a cloud sandbox, no config file is needed: set `VENEER_NET_HUB=<remote>` and veneer derives one. Claude Code cloud sessions are detected automatically; anywhere else, set `VENEER_CLOUD=<kind>` (for example `ci` or `agy-remote`). A cloud replica is named `<kind>-<session id tail>`, so two sessions never share an op log. Set `VENEER_REPLICA` to choose a name.

To run tasks on a machine, also map repositories to local clones in `net.toml` (`[repos]` table, `"owner/name" = "/path"`) and start `veneer net worker`. Start it from a login shell if your harness needs tokens from your shell profile.

## 4. Check the setup

```sh
veneer net sync                      # exit 0 means the hub is reachable and writable
echo '{"title":"hello","repo":"you/some-repo","prompt":"Add a README line.","harness":"any","where":"any","budget":{"minutes":10}}' | veneer task new -
veneer net worker --once             # on a machine with a harness; claims and runs it
veneer task show <id>
```

A task is `done` only with a passing ship-gate witness (`spec/net.md` section 3).

## For agents

If `veneer net status` reports no config, do this:

1. Ask the user for the hub remote. Do not guess one, and do not create a repository for them.
2. Run `veneer net init --hub <remote>` (add `--cloud` in a cloud sandbox). If `veneer net sync` fails on authentication, stop and tell the user which row of the table in section 2 applies to this environment, and what they need to add. Do not try other credentials you find on the machine.
3. Never ask for, print, log, commit or paste a token. If a token is needed, tell the user the environment variable name and let them set it where the sandbox can read it.
4. Do not change global git configuration on a workstation.
5. Report `veneer net status` back to the user.

## Troubleshooting

| Symptom | Cause |
|---|---|
| `hub unreachable ... kept locally` | no network or no push access; ops are kept in the local hub clone and sync later |
| `Authentication failed` or 403 on push | the token is for the wrong owner, lacks Contents write, or an organization has not approved it |
| `no config` | run `veneer net init`, or set `VENEER_NET_HUB` |
| two sessions show as one replica | set a distinct `VENEER_REPLICA`; replicas must not share a name |
