# The two tools

A client connected to `t3cm` sees two tools. `search` finds what to call, and
`execute` calls it. The model writes a short async arrow function for each.

## search

**search** `{code}` runs an async arrow function whose only globals are
`catalog` and `console`, and returns `{"result": ...}` with the function's
value, plus `logs` when the function logged anything.

`catalog` is an array of `{name, kind, server, description, signature,
inputSchema}`, in this order:

1. One entry per T3 tool: `kind: "t3"`, `server: "t3"`, the bare T3 name.
   `signature` is a generated TypeScript declaration with input and result
   types.
2. One per upstream tool: `kind: "mcp"`, `server` is the upstream's namespace,
   and `name` is `ns.tool`. The signature reads `ns.tool(...)`, or
   `ns["raw-name"](...)` when the tool name is not an identifier.
3. The `wf`, `mem` and `vars` API, with `kind` and `server` set to `"wf"` or
   `"mem"`.

Return only what you need. The point of the tool is to keep the catalog out of
the model's context:

```js
async () => catalog.find(t => t.name === "t3_thread_send").signature
```

```js
async () => catalog.filter(t => t.server === "cbm").map(t => t.name)
```

```js
async () => catalog
  .filter(t => /worktree/.test(t.name))
  .map(t => `${t.name}: ${t.description.slice(0, 80)}`)
```

The [API reference](api.md) is the same set of signatures, generated from a
snapshot of T3's tools.

## execute

**execute** `{code, session?, timeout_ms?, thread?}` runs an async arrow
function in a fresh sandbox and returns `{"result": ...}`, plus
`"logs": [...]` when the script logged anything.
The service strips TypeScript annotations and accepts loose input too:
markdown fences, `export default`, a named function, or a bare body whose last
expression becomes the result.

[Execute globals](globals.md) lists the sandbox's globals. In short,
`t3` calls T3, each upstream has its own namespace, and `mem`, `vars` and `wf`
keep state between runs.

```js
async () => {
  const { threads } = await t3.thread_list({ allProjects: true, statuses: ["idle"], limit: 50 });
  vars.idle = threads.map(t => t.threadId);
  return vars.idle.length;
}
```

Filter and count inside the script, keep the bulk in `vars`, and return the
answer. Only the return value and the logs reach the model.

```js
async () => {
  const { threads } = await t3.thread_list({ allProjects: true, statuses: ["failed"], limit: 200 });
  console.log(`${threads.length} failed threads`);
  return threads.slice(0, 5).map(t => t.title);
}
```

### Arguments

| Argument | Does |
|---|---|
| `code` | The function to run |
| `session` | Names the `mem` and `vars` namespace. It defaults to `thread:<caller>`, or `default` with no caller. |
| `timeout_ms` | Wall-clock limit, default 60000, max 600000 (`defaultTimeoutMs` and `maxTimeoutMs` in the [config](config.md)) |
| `thread` | The caller's T3 thread id, for a connection that does not send the `X-T3-Thread` header |

The caller is the `X-T3-Thread` header, else the `thread` input, else none.

### Results and errors

The service cuts results over 24,000 chars structurally. The output stays valid
JSON, the largest values go first, each cut carries a `--- TRUNCATED ---`
marker, and a `truncated` note says so.

Errors come back as `isError` with `Error: <message>` and any logs. The service
never echoes the script back, except that a syntax error quotes up to 160
characters of the line it failed on. A failed T3 or upstream call throws inside the script, so a
`try`/`catch` can handle it there.

A run that finds every slot taken waits up to `runQueueMs` (10 s) and then
fails with a busy error that says to retry. See
[Troubleshooting](troubleshooting.md#t3cm-is-busy).

## From a shell

`t3cm exec` and `t3cm search` call the running service's tools from a file or
stdin, for humans and scripts:

```bash
t3cm search - <<'JS'
async () => catalog.filter(t => t.kind === "mcp").length
JS
```
