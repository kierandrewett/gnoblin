# Grants

A `Grant` proves that the user just did something: pressed a shortcut, clicked with a binding, or asked for a window
menu. The compositor makes the proof. A script receives it as `event.grant`.

Pass the grant to an operation that an application must never start by itself: take focus, move or resize a
window, read the focused window for a snap, or type into the focused client.

```lua
local switch = gnoblin.shortcuts.bind("switch-notes", "<Super>n")

switch:on("activated", function(shortcut, event)
    local notes = gnoblin.windows.list({app_id = "notes"})[1]
    if not notes or not event.grant then
        return
    end
    notes:focus(event.grant)       -- takes the "focus" scope
    notes:begin_move(event.grant)  -- takes the "begin_move" scope of the same key press
end)
```

One key press gives one gesture. Each operation of a gesture can run once, so this handler does both. Before this,
every operation used up the whole gesture, and a script that wanted to focus and then move needed two key presses.

## Where a grant comes from

| Event                                                 | Gesture                                         |
| ----------------------------------------------------- | ----------------------------------------------- |
| `gnoblin.shortcut.activated`                          | A key press that matched a shortcut.            |
| `gnoblin.shortcut.binding-activated`                  | The first activation of a Lua shortcut binding. |
| `gnoblin.shortcut.session.key`                        | A real key press or release in a modal session. |
| `gnoblin.pointer.binding-activated`                   | A pointer binding such as `<Super>Button1`.     |
| `gnoblin.input` events for a `gnoblin.input.on`       | A real input event that the handler matched.    |
| `gnoblin.window.menu-requested` (window-manager menu) | A request for the window menu of one window.    |

Nothing else has a grant. A timer callback, a `Process` end, `gnoblin.window.created`, a repeated key and a
synthetic or input-method event carry no `event.grant`. A script cannot make one: the value is not a table and has no
constructor, and `getmetatable` returns only the name.

## Scope

A grant lists the operations it can still take in `grant.scope`, a list of these names:

| Scope          | Operation                             | Acts on                    |
| -------------- | ------------------------------------- | -------------------------- |
| `focus`        | `Window:focus(grant)`                 | The window you call it on. |
| `begin_move`   | `Window:begin_move(grant)`            | The window you call it on. |
| `begin_resize` | `Window:begin_resize(edge, grant)`    | The window you call it on. |
| `snap_context` | `gnoblin.windows.snap_context(grant)` | The window that had focus. |
| `text_target`  | `gnoblin.input.text_target(grant)`    | The client that had focus. |

A grant from a key press or an input event has all five. A grant from a window menu request has `begin_move` and
`begin_resize` only, and works for the window that asked for the menu: it offers one move or one resize, whichever
comes first, and only in the event callback that received it.

`grant:narrow(scope, window?)` returns a grant for fewer operations, and for one window if you pass it. `scope` is one
name or a list of names. `window` is a `Window` or its ID.

```lua
local mover = event.grant:narrow("begin_move", window)
other_code(mover)                      -- can start one move of `window`, and nothing else
```

Narrowing never widens. It fails when:

- you name an operation the grant does not have, or that the gesture already used;
- you name a window and the grant is already narrowed to another;
- you name something that is not an operation.

Narrowed grants share their gesture. When `mover` starts the move, the grant it came from has no `begin_move` left,
and a second narrowing cannot take it again.

To see the state of a grant, read:

- `grant.scope`: what is left;
- `grant.window`: the window, or `nil`;
- `grant.expires_at_us` and `grant.alive`.

An operation fails at once, with a Lua error, when the grant has no such scope, is narrowed to another window, or when
its gesture already used the operation. It does not use up the scope in these cases.

## Order

The compositor checks again when it runs an operation, and the focus changes between operations.
`gnoblin.windows.snap_context` and `gnoblin.input.text_target` hold only while the window that had focus at the gesture
still has it, so take them before `Window:focus`. Queued operations run in the order you made them.

## Limits

- A gesture ends five seconds after the key press or click. A grant that you keep ends with it.
- A configuration reload, which starts a new runtime, ends every grant of the old one.
- A session lock ends every gesture. The compositor refuses a grant after it, and it issues none while the session is
  locked.
- A pointer binding starts a move or resize only for the window under the pointer when it fired, and only while the
  button stays down.
- `text_target` needs a gesture without a modifier key, and the focused client must have an active text input.
- Operations that have no grant keep the focus-stealing rules. `Window:present` needs no grant, but only works for a
  window of a program that you started with `gnoblin.process.spawn`, and waits for the user to stop typing.

A grant says that the user acted. It does not say what the script may do afterwards: that is the scope.

## In gnoblinctl lua

`gnoblinctl lua` gives an event the same `Grant`. The compositor has no narrowing on the socket, so `narrow` works in
the `gnoblinctl` process, and the compositor still allows each operation once. A grant here has no `expires_at_us`.

## Socket clients

A socket client receives `focus_context`, a token for its connection, on the events listed in the
[bridge](/bridge/shortcuts-and-events#api-version-110-shortcut-focus-grants). From API 1.92 the token lasts for the
whole gesture. Each of the five operations can use it once. A different connection loses its token for the gesture
when the first connection uses one. The wire has no narrowing.
