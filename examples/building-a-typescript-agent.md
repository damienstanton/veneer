# Building a TypeScript agent with veneer

We build a small tool-using Claude agent — a **codebase Q&A agent** with three
tools (`read_file`, `list_files`, `grep`) — and let veneer's laws and lifecycle
shape it. The [Python walkthrough](building-a-python-agent.md) builds the same
agent; read them together to compare idioms.

Everything below is real `@anthropic-ai/sdk` code. Only `oxidation` (the Rust
shadow check) does not apply here; every other law does.

---

## Setup

```bash
npm install @anthropic-ai/sdk
veneer init
```

`veneer init` writes `.veneer/config.toml` and the agent skill. Configure the
budget and declare the one module we will seal — `tools` — so the rest of the
codebase depends on its public surface (`src/tools/index.ts`), not its internals:

```toml
# .veneer/config.toml
loc_soft = 500
loc_hard = 1000

[[modules]]
path = "src/tools"
public = ["index.ts"]
```

---

## Phase: plan

Decompose the agent into first-principles modules — each comprehensible from
its signature alone, each with one purpose. Express the plan as signatures, not
implementations:

```ts
// src/types.ts — the ADTs. Errors are data; results are a sum type.
export type ToolResult =
  | { ok: true; output: string }
  | { ok: false; error: AgentError };

export type AgentError =
  | { kind: "not_found"; path: string }
  | { kind: "io"; message: string }
  | { kind: "unknown_tool"; name: string };

// src/tools/index.ts — the sealed public surface.
export type ToolName = "read_file" | "list_files" | "grep";
export const toolSpecs: Anthropic.Tool[];
export function dispatch(name: string, input: unknown): Promise<ToolResult>;

// src/client.ts — a thin wrapper over the SDK.
export function makeClient(): Anthropic;

// src/loop.ts — the agent turn loop.
export function runAgent(client: Anthropic, question: string): Promise<string>;
```

Acceptance criteria are observable behavior: *given a question about the repo,
the agent calls tools and returns a text answer; an unknown tool or a missing
file surfaces as an `AgentError`, never a thrown exception across a module
boundary.*

Record the ticket and move on: `veneer state set implement`.

---

## Phase: implement

Write the feature, keeping each module first-principles and composing via the
three laws.

### Law 1 — type-level constraint (ADTs, exhaustive handling, errors as data)

`ToolResult` is a discriminated union, so every consumer must handle both arms.
`name` arrives from the API as a bare `string` — an untrusted boundary value, not
yet known to be a `ToolName` — so it is validated once at the edge with a type
guard. Only after that does `dispatch` switch on the now-narrowed `ToolName`,
where a `never` assignment genuinely fails to compile if a variant is ever added
without a matching `case`:

```ts
// src/tools/index.ts
import { readFile, readdir } from "node:fs/promises";
import type { ToolResult, AgentError } from "../types.js";

export type ToolName = "read_file" | "list_files" | "grep";

export const toolSpecs = [
  { name: "read_file", description: "Read a UTF-8 file by path.",
    input_schema: { type: "object", properties: { path: { type: "string" } },
      required: ["path"] } },
  { name: "list_files", description: "List entries in a directory.",
    input_schema: { type: "object", properties: { dir: { type: "string" } },
      required: ["dir"] } },
  { name: "grep", description: "Find lines matching a regex in a file.",
    input_schema: { type: "object",
      properties: { path: { type: "string" }, pattern: { type: "string" } },
      required: ["path", "pattern"] } },
] as const;

const ioError = (e: unknown): AgentError =>
  e && typeof e === "object" && (e as { code?: string }).code === "ENOENT"
    ? { kind: "not_found", path: String((e as { path?: string }).path ?? "") }
    : { kind: "io", message: e instanceof Error ? e.message : String(e) };

const TOOL_NAMES = ["read_file", "list_files", "grep"] as const;

function isToolName(name: string): name is ToolName {
  return (TOOL_NAMES as readonly string[]).includes(name);
}

export function dispatch(name: string, input: unknown): Promise<ToolResult> {
  if (!isToolName(name)) {
    return Promise.resolve({ ok: false, error: { kind: "unknown_tool", name } });
  }
  const args = input as Record<string, string>;
  switch (name) {                      // name: ToolName here — genuinely exhaustive
    case "read_file":
      return runIO(() => readFile(args.path, "utf8"));
    case "list_files":
      return runIO(() => readdir(args.dir).then((xs) => xs.join("\n")));
    case "grep":
      return runIO(() =>
        readFile(args.path, "utf8").then((text) =>
          text.split("\n").filter((l) => new RegExp(args.pattern).test(l)).join("\n"),
        ),
      );
    default: {
      // Adding a ToolName variant without a case above stops this line compiling.
      const _exhaustive: never = name;
      return _exhaustive;
    }
  }
}
```

Notice what the laws bought us: `dispatch` returns a `ToolResult` — it never
throws across the module boundary. IO failures are *reclassified into data*
(`not_found` vs `io`) before they leave. `runIO` is the one place that catches:

```ts
async function runIO(fn: () => Promise<string>): Promise<ToolResult> { /* ... */ }
```

(`dispatch` returning a promise is a detail; the important shape is that the
result type is a sum, not an exception channel.)

### Law 2 — first-principles modules (~500 LoC, grow by adding, not enlarging)

The agent is five small files, each understandable from its signature:
`types`, `client`, `tools`, `loop`, `main`. When you add a fourth tool, you do
**not** grow `dispatch` past its budget — you factor each tool into its own file
under `src/tools/` and keep `index.ts` as the thin dispatcher. Adding a tool is
adding a module.

### Law 3 — total boundaries (sealing + idempotency)

The `[[modules]]` entry from setup seals `tools`: `loop.ts` imports from
`src/tools/index.ts` (the public surface), never from a tool's internal file.
If `loop.ts` reached into `src/tools/grep_impl.ts`, veneer would raise a
`module_sealing` finding.

The agent loop itself is total and structural — it exhaustively handles each
content block and feeds tool results back until the model stops calling tools:

```ts
// src/loop.ts
export async function runAgent(client: Anthropic, question: string): Promise<string> {
  const messages: Anthropic.MessageParam[] = [{ role: "user", content: question }];
  for (;;) {
    const res = await client.messages.create({
      model: "claude-opus-4-8",
      max_tokens: 16000,
      tools: toolSpecs as unknown as Anthropic.Tool[],
      messages,
    });
    if (res.stop_reason !== "tool_use") {
      const text = res.content.find((b) => b.type === "text");
      return text?.type === "text" ? text.text : "";
    }
    messages.push({ role: "assistant", content: res.content });
    const results: Anthropic.ToolResultBlockParam[] = [];
    for (const block of res.content) {
      if (block.type !== "tool_use") continue;      // exhaustive over block kinds
      const r = await dispatch(block.name, block.input);
      results.push({
        type: "tool_result",
        tool_use_id: block.id,
        content: r.ok ? r.output : render(r.error),  // both arms handled
        is_error: !r.ok,
      });
    }
    messages.push({ role: "user", content: results });
  }
}
```

Idempotency shows up when veneer checks a *proposed diff*: anchor insertions to
unique context lines so re-applying the same diff twice fails cleanly rather
than duplicating code. The agent loop above is also idempotent in the
operational sense — re-running a `read_file` for the same path is a no-op.

Run `veneer check --compact` as you go; it is cheap and deterministic. Because
these are `.ts` files, `oxidation` never fires — the findings you will see are
only `module_budget`, `module_sealing`, and `idempotency`.

When the feature is written and `npm test` passes: `veneer state set verify`.

---

## Phase: verify

1. Run your own suite (`npm test`) and fix failures first.
2. `veneer check --compact` → parse the JSON findings from stdout.
3. For each finding, read `law` / `location` / `message` and fix exactly that —
   split a file that tripped `module_budget`, redirect an import that tripped
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

When `loop.ts` needs the `tools` surface, don't re-read the file — query the
cached graph:

```bash
veneer graph query src/tools/index.ts
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

| Law | TypeScript idiom | Python idiom |
|---|---|---|
| **1 — type-level** | discriminated union `\| { ok: true } \| { ok: false }`; exhaustive `switch` with a `never` guard; errors as `AgentError` data | `@dataclass` + `typing.Union` / `match`; `Protocol`; errors as returned values |
| **2 — modules** | five files under the LoC band; add a tool = add a file under `src/tools/` | five modules under the LoC band; add a tool = add a module in `tools/` |
| **3 — boundaries** | `[[modules]]` seals `src/tools` to `index.ts`; imports hit the public surface | `[[modules]]` seals `tools` to `__init__.py`; imports hit the package surface |
| **oxidation** | does not fire (`.ts` files) | does not fire (`.py` files) |

The point of building the same agent twice: the laws are the constant, the
syntax is the variable.
