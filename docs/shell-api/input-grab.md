# Keyboard grabs

`gnoblin.input.grab(options?)` takes the keyboard for your shell. While the grab is alive, every key press and release
goes to your code. No application, no shortcut and no other handler sees a key. Use it for a launcher, a password
prompt, a key-binding recorder or a modal overlay that types text. It returns a `Grab`.

```lua
local grab = gnoblin.input.grab({timeout = 10})
local typed = ""

grab:on("key", function(grab, key)
    if not key.pressed then
        return
    end
    if key.key == "Escape" then
        grab:release()
    elseif key.key == "BackSpace" then
        typed = typed:sub(1, -2)
    elseif key.text then
        typed = typed .. key.text
    end
end)

grab:on("closed", function(grab, reason)
    print("grab ended:", reason, "typed:", typed)
end)
```

Call `gnoblin.input.grab` while you handle an event, as with every operation that changes the session.

## The lease

A grab is a lease. It ends by itself when it has had no key event and no `grab:renew()` for `timeout` seconds. A script
that stops, hangs or forgets to release cannot keep the keyboard.

| Option    | Value                                                  |
| --------- | ------------------------------------------------------ |
| `timeout` | Idle seconds, a whole number from 1 to 60. Default 10. |

- Every key event that reaches the grab starts the timeout again. Someone who types needs the grab, so a long entry
  does not expire. A script that waits for a slow task, with no key, calls `grab:renew()`.
- `grab:renew()` returns an `Operation` that succeeds with `{id, renewed = true}`. It fails with `not_found` for a grab
  that has ended.
- `grab:release()` gives the keyboard back. It returns an `Operation` that succeeds with `{id, released = true}`, and
  fails with `not_found` for a grab that has ended. The grab ends, and its `"closed"` listeners run with the reason
  `"released"`, when the compositor has done it.

There is one grab for the whole compositor. A new grab ends the old one with the reason `"replaced"`.

## The Grab object

| Field     | Value                                            |
| --------- | ------------------------------------------------ |
| `id`      | Identifier of this grab, `"grab-"` and a number. |
| `alive`   | `true` until the grab has ended.                 |
| `timeout` | The idle seconds you asked for.                  |
| `device`  | What the grab holds. It is always `"keyboard"`.  |

The `Grab` exists at once, before the compositor confirms it. If the compositor refuses the grab, the object ends and its
`"closed"` listeners get the reason, which is the message of the compositor.

`grab:on(signal, fn)` takes one of these signals and returns a subscription with an `unsubscribe()` method:

| Signal     | `fn` receives    | Runs when                                             |
| ---------- | ---------------- | ----------------------------------------------------- |
| `"key"`    | `grab`, `key`    | A key is pressed or released, or a press repeats.     |
| `"closed"` | `grab`, `reason` | The grab has ended. Listening ends after this signal. |

## Key events

`key` is a table with these fields:

| Field       | Value                                                                                 |
| ----------- | ------------------------------------------------------------------------------------- |
| `key`       | Name of the keysym: `"a"`, `"A"`, `"Return"`, `"Escape"`, `"dead_acute"`.             |
| `keysym`    | The keysym number.                                                                    |
| `keycode`   | The hardware key code.                                                                |
| `modifiers` | The modifier mask when the key came. Shift is 1, Control 4, Alt 8 and Super 67108864. |
| `pressed`   | `true` for a press, `false` for a release.                                            |
| `repeated`  | `true` for a press that key repeat made.                                              |
| `text`      | What the press types, or `nil`. See below.                                            |
| `composing` | `true` while a dead key or Compose sequence is in progress.                           |
| `time`      | The event time in milliseconds.                                                       |

`key` names the key as the layout in use reads it, so the same physical key gives `"a"` on one layout and `"q"` on
another, and a shifted key gives `"A"`. Use `keycode` to name a physical key.

### Text

A press has `text` when it types something. The text comes from the keysym. Gnoblin also feeds every press to the
compose table of the locale (libxkbcommon), so dead keys and Compose sequences work:

- A key that types a character has that character as `text`: `"a"`, `"A"`, `"7"`, `"é"`.
- A dead key, or the Compose key, starts a sequence. It and the keys inside the sequence have `composing = true` and no
  `text`.
- The key that completes the sequence has the composed character as `text` and `composing = false`. Dead acute and `e`
  give `"é"`.
- A key that breaks the sequence has its own `text`, if it types anything. The sequence is dropped, so the dead key's
  own character is lost.
- A release, a modifier, `Return`, `Tab`, `BackSpace`, an arrow and any other key that types nothing have no `text`. Use
  `key.key` for these.
- A press with Control, Alt or Super held has no `text`, because it is a command and not typing.

The locale comes from `LC_ALL`, `LC_CTYPE` or `LANG`; the `C` locale uses the `en_US.UTF-8` table. If the locale has no
compose table, a dead key has no text, and Gnoblin logs a warning once.

## When the grab ends

The `"closed"` signal passes one of these reasons:

| Reason              | Meaning                                                                                     |
| ------------------- | ------------------------------------------------------------------------------------------- |
| `"released"`        | Your code called `grab:release()`.                                                          |
| `"expired"`         | The grab had no key and no renewal for `timeout` seconds.                                   |
| `"replaced"`        | Another call to `gnoblin.input.grab` took the keyboard.                                     |
| `"locked"`          | The session locked. The lock screen owns the keyboard.                                      |
| `"preempted"`       | Mutter gave the keyboard to another owner, such as a popup grab or an input-capture portal. |
| `"config_reloaded"` | A configuration reload committed.                                                           |
| `"runtime_stopped"` | The Lua worker is shutting down.                                                            |

A refused grab passes the message of the compositor instead.

On `"config_reloaded"` and `"runtime_stopped"` the listener runs as the old runtime goes. It can read, but it cannot start
an operation. A Lua worker that restarts loses its grab without a signal, because the new worker is a different
runtime. The compositor gives the keyboard back in every case.

## What a grab cannot do

- It refuses to start while the session is locked, and it ends when the session locks. The lock screen always owns the
  keyboard. The call raises no error; the `Grab` ends when the compositor answers, with a reason that says so.
- It never takes the keyboard from a shortcut session (a `modal` or `capture_input` shortcut) or from
  `gnoblin.shortcuts.capture`, and they cannot start while a grab holds the keyboard. The grab ends when the compositor
  answers, with a reason that says so. Bare `Super` and every other shortcut stop working until the grab ends, because the grab takes
  the keys before the shortcuts see them.
- Its key events carry no `focus_context`. A grab cannot move, resize or focus a window with the keys it holds, which a
  trusted shortcut can.
- It does not change the focus. The focused window stays focused, but it gets no keys.
- A key that is held when the grab starts has its release go to the grab, so the focused application can see a key that
  never lifts. Start a grab from a key release, or when no key is down.

## Why shortcut sessions keep their own timer

A `modal` or `capture_input` shortcut session still ends after 10 seconds, with the reason `timed_out`. It does not use
the grab lease.

A session belongs to a gesture of the user: it starts when the user holds a key and ends when the user lets go. The 10
seconds only guard against a lost key release. No script owns a session, so nothing can renew it. Its events carry a
`focus_context` for a trusted shortcut.

The two cannot share a lease. A lease that a script renews would let a script hold a session open for ever, and a grab
that gives a `focus_context` would let a script move windows without a user gesture. They have different owners and
different trust, so they share Mutter's one key-capture slot and nothing else.

## Keyboard grabs in `gnoblinctl lua`

A `gnoblinctl lua` script has no runtime that receives key events, so `gnoblin.input.grab` raises an error there. Make
the grab in a configuration file.

## Bridge

Lua reaches the grab with the runtime operations `input.grab`, `input.grab.renew` and `input.grab.release` (bridge API
1.91), and receives the events `gnoblin.input.grab.key` and `gnoblin.input.grab.closed`. Socket clients do not have
them.
