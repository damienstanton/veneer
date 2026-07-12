# Building a Python agent with veneer

We build a small tool-using Claude agent — a **codebase Q&A agent** with three
tools (`read_file`, `list_files`, `grep`) — and let veneer's laws and lifecycle
shape it. The [TypeScript walkthrough](building-a-typescript-agent.md) builds the
same agent; read them together to compare idioms.

Everything below is real `anthropic` SDK code. Only `oxidation` (the Rust shadow
check) does not apply here; every other law does.

---

## Setup

```bash
pip install anthropic
veneer init
```

`veneer init` writes `.veneer/config.toml` and the agent skill. Configure the
budget and declare the one module we will seal — `tools` — so the rest of the
codebase depends on its public surface (`tools/__init__.py`), not its internals:

```toml
# .veneer/config.toml
loc_soft = 500
loc_hard = 1000

[[modules]]
path = "tools"
public = ["__init__.py"]
```

---

## Phase: plan

Decompose the agent into first-principles modules — each comprehensible from
its signature alone, each with one purpose. Express the plan as signatures, not
implementations:

```python
# agent_types.py — the ADTs. Errors are data; results are a sum type.
# (named agent_types, not types — the stdlib already owns that name)
@dataclass(frozen=True)
class Ok:
    output: str

@dataclass(frozen=True)
class Err:
    error: "AgentError"

ToolResult = Ok | Err

@dataclass(frozen=True)
class NotFound:
    path: str

@dataclass(frozen=True)
class IOFailure:
    message: str

@dataclass(frozen=True)
class UnknownTool:
    name: str

AgentError = NotFound | IOFailure | UnknownTool

# tools/__init__.py — the sealed public surface.
TOOL_SPECS: list[dict]
def dispatch(name: str, args: dict) -> ToolResult: ...

# client.py — a thin wrapper over the SDK.
def make_client() -> anthropic.Anthropic: ...

# loop.py — the agent turn loop.
def run_agent(client: anthropic.Anthropic, question: str) -> str: ...
```

Acceptance criteria are observable behavior: *given a question about the repo,
the agent calls tools and returns a text answer; an unknown tool or a missing
file surfaces as an `AgentError`, never a raised exception across a module
boundary.*

Record the ticket and move on: `veneer state set implement`.

---

## Phase: implement

Write the feature, keeping each module first-principles and composing via the
three laws.

### Law 1 — type-level constraint (ADTs, exhaustive handling, errors as data)

`ToolResult` and `AgentError` are sum types built from frozen dataclasses.
Dispatch handles every arm with `match`; the `case _` arm turns an unrecognized
tool into *data*, not an exception:

```python
# tools/__init__.py
import os
from agent_types import Ok, Err, NotFound, IOFailure, UnknownTool, ToolResult

TOOL_SPECS = [
    {"name": "read_file", "description": "Read a UTF-8 file by path.",
     "input_schema": {"type": "object",
        "properties": {"path": {"type": "string"}}, "required": ["path"]}},
    {"name": "list_files", "description": "List entries in a directory.",
     "input_schema": {"type": "object",
        "properties": {"dir": {"type": "string"}}, "required": ["dir"]}},
    {"name": "grep", "description": "Find lines matching a regex in a file.",
     "input_schema": {"type": "object",
        "properties": {"path": {"type": "string"}, "pattern": {"type": "string"}},
        "required": ["path", "pattern"]}},
]

def _io(fn) -> ToolResult:
    """The one place that catches — failures become AgentError data here."""
    try:
        return Ok(fn())
    except FileNotFoundError as e:
        return Err(NotFound(path=str(e.filename)))
    except OSError as e:
        return Err(IOFailure(message=str(e)))

def _read(path: str) -> str:
    with open(path, encoding="utf-8") as f:
        return f.read()

def dispatch(name: str, args: dict) -> ToolResult:
    match name:
        case "read_file":
            return _io(lambda: _read(args["path"]))
        case "list_files":
            return _io(lambda: "\n".join(os.listdir(args["dir"])))
        case "grep":
            import re
            return _io(lambda: "\n".join(
                l for l in _read(args["path"]).splitlines()
                if re.search(args["pattern"], l)))
        case _:
            return Err(UnknownTool(name=name))
```

Notice what the laws bought us: `dispatch` returns a `ToolResult` — it never
raises across the module boundary. IO failures are *reclassified into data*
(`NotFound` vs `IOFailure`) before they leave, inside the single `_io` helper.

### Law 2 — first-principles modules (~500 LoC, grow by adding, not enlarging)

The agent is five small modules, each understandable from its signature:
`agent_types`, `client`, `tools`, `loop`, `main`. When you add a fourth tool, you do
**not** grow `dispatch` past its budget — you factor each tool into its own
module inside the `tools` package and keep `__init__.py` as the thin dispatcher.
Adding a tool is adding a module.

### Law 3 — total boundaries (sealing + idempotency)

The `[[modules]]` entry from setup seals `tools`: `loop.py` imports from the
`tools` package surface (`__init__.py`), never from a tool's internal module. If
`loop.py` reached into `tools/_grep_impl.py`, veneer would raise a
`module_sealing` finding.

The agent loop itself is total and structural — it exhaustively handles each
content block and feeds tool results back until the model stops calling tools:

```python
# loop.py
from tools import TOOL_SPECS, dispatch
from agent_types import Ok, Err

def run_agent(client, question: str) -> str:
    messages = [{"role": "user", "content": question}]
    while True:
        res = client.messages.create(
            model="claude-opus-4-8",
            max_tokens=16000,
            tools=TOOL_SPECS,
            messages=messages,
        )
        if res.stop_reason != "tool_use":
            return next((b.text for b in res.content if b.type == "text"), "")
        messages.append({"role": "assistant", "content": res.content})
        results = []
        for block in res.content:
            if block.type != "tool_use":
                continue                               # exhaustive over block kinds
            match dispatch(block.name, block.input):   # both arms handled
                case Ok(output):
                    results.append({"type": "tool_result",
                        "tool_use_id": block.id, "content": output})
                case Err(error):
                    results.append({"type": "tool_result",
                        "tool_use_id": block.id, "content": render(error),
                        "is_error": True})
        messages.append({"role": "user", "content": results})
```

Idempotency shows up when veneer checks a *proposed diff*: anchor insertions to
unique context lines so re-applying the same diff twice fails cleanly rather
than duplicating code. The loop above is also idempotent in the operational
sense — re-running a `read_file` for the same path is a no-op.

Run `veneer check --compact` as you go; it is cheap and deterministic. Because
these are `.py` files, `oxidation` never fires — the findings you will see are
only `module_budget`, `module_sealing`, and `idempotency`.

When the feature is written and `pytest` passes: `veneer state set verify`.

---

## Phase: verify

1. Run your own suite (`pytest`) and fix failures first.
2. `veneer check --compact` → parse the JSON findings from stdout.
3. For each finding, read `law` / `location` / `message` and fix exactly that —
   split a module that tripped `module_budget`, redirect an import that tripped
   `module_sealing`. No drive-by refactoring.
4. Re-check. A clean run records the ship-gate witness.

Clean (exit 0, no error findings) → `veneer state set ship`.

---

## Phase: ship

The gate has already proven the tree clean. Branch, commit a message that
describes behavior, and open the PR (`gh pr create`), linking the issue from
your state refs. Report the URL. Next cycle: `veneer state set plan`.

---

## Using the knowledge graph to save tokens

When `loop.py` needs the `tools` surface, don't re-read the file — query the
cached graph:

```bash
veneer graph query tools/__init__.py
# → { "entry": { "signatures": [...], "doc_summary": "...", "loc": ... },
#     "stale": false }
```

A fresh entry gives you the module's signatures and doc summary at a fraction of
the tokens of the full source. `stale: true` means the file changed since the
last refresh — run `veneer check` (a clean check refreshes the graph
automatically). The graph never affects the ship gate; reading it, or ignoring
it, is always safe.

---

## The laws, side by side

| Law | Python idiom | TypeScript idiom |
|---|---|---|
| **1 — type-level** | `@dataclass` sum types + `match`; `case _` turns the miss into data; errors as `AgentError` values | discriminated union `\| { ok: true } \| { ok: false }`; exhaustive `switch` with `never` |
| **2 — modules** | five modules under the LoC band; add a tool = add a module in `tools/` | five files under the LoC band; add a tool = add a file under `src/tools/` |
| **3 — boundaries** | `[[modules]]` seals `tools` to `__init__.py`; imports hit the package surface | `[[modules]]` seals `src/tools` to `index.ts`; imports hit the public surface |
| **oxidation** | does not fire (`.py` files) | does not fire (`.ts` files) |

The point of building the same agent twice: the laws are the constant, the
syntax is the variable.
