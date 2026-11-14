# Console and inspector

The console is a live prompt into the running [TypeScript shell](typescript-shell.md). You type an expression, the
shell evaluates it in its own runtime and prints the value. It sees what `init.ts` sees: the `gnoblin` global, the
clients and workspaces as they are right now, and the bindings the shell has registered.

```sh
gnoblinctl console
```

```text
gnoblin> gnoblin.clients.length
2
gnoblin> gnoblin.clients.focused?.title
"Terminal"
gnoblin> gnoblin.workspaces.map((ws) => [ws.id, ws.clients.length])
[ [ "1", 2 ], [ "2", 0 ], [ "3", 0 ], [ "4", 0 ] ]
```

It needs a session that runs the JavaScript shell, which is the default (see
[Which shell runs](typescript-shell.md#which-shell-runs)). Without one, `gnoblinctl console` exits 1 and says so.

## The prompt

Each line is one expression. A line that ends inside an unfinished expression (an open brace or parenthesis) is
continued on the next line, with a `...` prompt. The terminal's own line editing applies; `rlwrap gnoblinctl console`
adds arrow-key history. These lines are commands of the prompt, not expressions:

| Line       | What it does                              |
| ---------- | ----------------------------------------- |
| `.exit`    | Leaves the console. Ctrl-D does the same. |
| `.history` | Lists the expressions of this session.    |
| `!!`       | Runs the last expression again.           |
| `!N`       | Runs expression N of `.history` again.    |
| `.help`    | Lists these commands.                     |

An error prints as a block with the message, where in the expression it happened and a hint, and the prompt goes on:

```text
gnoblin> gnoblin.clients.map(
... )
error: unexpected token in expression: ')'
why:   at <console>:2:1
hint:  fix the expression and run it again; a syntax error names the column in the expression
```

When standard input is not a terminal there are no prompts, so a script can pipe expressions in:
`echo 'gnoblin.clients.length' | gnoblinctl console`. It exits 1 if any line failed.

## One expression

`--eval` evaluates one expression, prints its value as JSON and exits:

```sh
gnoblinctl console --eval 'gnoblin.clients.map((c) => c.appId)'
```

```text
["foot","firefox"]
```

The exit code is 0 when the expression gave a value, and 1 when it failed or no shell is running. `undefined` prints
as `null`.

With `--json` the whole reply is printed instead. It has `ok`, then either `type`, `value`, `text` and `truncated`, or
`error`.

```sh
gnoblinctl --json console --eval 'gnoblin.clients.at(0)'
```

## What an expression can do

- **Read and write live state.** `gnoblin.clients.at(0).x = 10` moves the window: the write travels in the transaction
  of the next frame, exactly as a write from a listener does.
- **Define variables.** A `var`, a `let` or a `const` at the prompt, or an assignment to an undeclared name, is a global
  of the running generation and stays for the next expression, from any console connection. To redefine a `let` or a
  `const`, use `var` or assign to it.
- **Use the helpers.** `inspect` is a global in the console; see below. In `init.ts` import it:
  `import { inspect } from "gnoblin"`.
- **Return objects.** A value whose first character is `{` is read as an object literal: `{a: 1}` is the object.
  A promise that has already settled shows its result.

Nothing the console defines is kept. A reload (`gnoblinctl config reload`, or a save of `init.ts`) starts a new
generation, so the variables are gone, and nothing is written to disk.

### Between frames, under a budget

Expressions run on the shell's own runtime between frames, never beside one, so what an expression reads is the state
the next frame sees.

Each has the shell's call budget (100 ms). One that runs past it, such as `while (true) {}`, is interrupted and
answered with a `budget` error; the console and the shell go on. An interrupted expression is not a strike against the
generation, so the shell does not restart over it.

### How values print

- A client prints as its kind and id, then its fields: `Client#3 { appId: "foot", title: "Terminal", ... }`. A
  workspace prints the same way.
- A collection prints as an array of summaries: `gnoblin.clients` is `Collection(2) [ Client#3 { ... }, ... ]`, with
  each client's app id and title only. In JSON each is `{"$class": "Client", "id": "3", "appId": "foot", ...}`.
- A function prints as `[Function name]`, a class as `[class Name]`, a cycle as `[Circular]`.
- A value that does not fit the size cap (64 KiB of text) is cut, and the reply says `"truncated": true`; the prompt
  prints a warning. Ask for less with `.map` or `.slice`.

## The inspector helpers

`inspect` returns plain data: numbers, strings, booleans, arrays and objects of those, nothing live. Keep a result
without holding a native object.

| Call                   | Returns                                                                                          |
| ---------------------- | ------------------------------------------------------------------------------------------------ |
| `inspect.clients()`    | Every client: `id`, `appId`, `title`, `focused`, `fullscreen`, `floating`, geometry, `workspace` |
| `inspect.workspaces()` | Every workspace: `id`, `name`, `index`, `active`, `layout` (class name), `clients` (ids)         |
| `inspect.bindings()`   | Every binding registered on a seat: see below                                                    |
| `inspect.config()`     | The effective config: your `defineConfig` laid over the stock one; `null` with no `defineConfig` |

`gnoblin.clients.focused` is the client that holds the keyboard focus, or `null`.

Each entry of `inspect.bindings()` says which binding is registered, who owns it and where it ranks:

```text
gnoblin> inspect.bindings().filter((b) => b.trigger === "Super+MouseLeft")
[ { context: "seat0", id: "floating.move", trigger: "Super+MouseLeft", kind: "pointer",
    owner: "layout", layout: "FloatingLayout", device: null, priority: 0, enabled: true, rank: 1 } ]
```

- `owner` is who bound it: `context` (the seat's own bindings, such as the stock keys), `layout` (a layout attached to
  the seat; `layout` names its class) or `device` (one input device; `device` names it).
- `rank` orders the bindings of one trigger the way a press does: a device's binding before a layout's before the
  context's, then the higher `priority`, then the one registered first. Rank 1 runs first; a binding that declines hands
  the press to rank 2.

`inspect.config()` shows a function (a key's handler, `setup`) as `[Function name]`.

## Who can connect

The shell listens on `js.sock`, in the directory of the compositor's socket (`$XDG_RUNTIME_DIR/gnoblin/js.sock`, or
beside `GNOBLIN_IPC_SOCKET`).

- The socket is mode 0600.
- The shell reads the peer's user id from the kernel on every connection, and only the user the shell runs as is served.
- Anyone who can use the console can run code as that user in the shell, which is why nobody else can.

Set `GNOBLIN_JS_CONSOLE=0` in the session's environment to run the shell without a console.

## The protocol

The console is newline-delimited JSON on the socket, so a tool can speak it without `gnoblinctl`. A request is one line,
`{"id": 1, "expr": "gnoblin.clients.length"}`. The reply carries the same `id`:

```json
{ "id": 1, "ok": true, "type": "number", "value": 2, "text": "2", "truncated": false }
```

A failure has `"ok": false` and an `error` with `kind` (`exception`, `budget`, `memory`, `request` or `engine`),
`message`, `location` (`{"file": "<console>", "line": 1, "column": 5}` or `null`) and `stack`.

## See also

- [TypeScript shell](typescript-shell.md) for the shell the console talks to.
- [gnoblinctl](gnoblinctl.md#console) for the command.
