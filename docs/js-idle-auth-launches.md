# Idle, authentication and launch feedback

Three parts of the `gnoblin` global reach the compositor beyond the window tree: `gnoblin.idle`, `gnoblin.authentication`
and `gnoblin.launches`. They are part of the [TypeScript shell](typescript-shell.md). Each one is backed by something the
compositor serves today, and the last section says which Lua pages have no TypeScript counterpart and why.

## Idle watches

`gnoblin.idle.watch(ms, handlers)` runs your code when the session has had no input for `ms` milliseconds. It returns an
`IdleWatch`.

```ts
import { defineConfig } from "gnoblin";

defineConfig({
    setup: (gnoblin) => {
        const watch = gnoblin.idle.watch(300_000, {
            onIdle: () => gnoblin.spawn("loginctl lock-session"),
            onResume: () => console.log("the user is back"),
        });
        // watch.cancel() ends it.
    },
});
```

`ms` is a number above 0 and at most 86400000 (a day). A runtime has at most 256 live watches, and each has its own time.
Any other value throws a `RangeError`.

A watch is not idle when it starts. It becomes idle once the session has been idle for `ms` and nothing holds it awake.

- `onIdle` runs for every idle period until you cancel the watch.
- `onResume` runs on the first input after an idle period, or when a hold appears.
- A watch made while the session is already idle runs `onIdle` at once.

| Member                    | Value                                                                 |
| ------------------------- | --------------------------------------------------------------------- |
| `watch.id`                | `"idle-"` and a number.                                               |
| `watch.ms`                | The idle time you asked for.                                          |
| `watch.idle`              | `true` while the session is idle for `ms` and nothing holds it awake. |
| `watch.alive`             | `true` until `cancel()`.                                              |
| `watch.cancel()`          | Ends the watch. `onClosed(watch, "cancelled")` runs once.             |
| `gnoblin.idle.idleMs`     | Milliseconds since the last input at the last reading, or `null`.     |
| `gnoblin.idle.inhibitors` | How many holds keep the session awake, or `null`.                     |
| `gnoblin.idle.watches`    | The live watches, oldest first.                                       |

The compositor publishes the `idle` object with `idle_ms` and `inhibitors`; a watch is a threshold over that reading. A
hold from a Wayland idle-inhibit surface, such as a video player's, therefore delays `onIdle` the way it delays a screen
blank.

### Holding the session awake

`gnoblin.idle.inhibit(holder)` records a hold under a name you choose and returns an `IdleInhibitor`. While one stands the
session is not idle, and every watch resumes. `inhibitor.release()` gives it back. Holding again under a name you already
hold returns the same object. If the compositor refuses the hold (it keeps a limited number), `inhibitor.alive` is `false`
and the shell logs why.

The shell gives every hold back when the generation ends, which a reload and a shutdown both do. The compositor records a
hold by name and not by connection, so a shell that is killed without ending its generation leaves its holds standing
until the same name is released. Keep your holders few and named for what they are.

### What differs from the Lua page

- Times are milliseconds, not seconds.
- `watch.watch(field, fn)` and the `"closed"` reasons `config_reloaded` and `runtime_stopped` have no counterpart: a
  generation's watches end with it, silently. `onClosed` runs for `cancel()` only.
- The connection to the compositor is opened when the first watch or hold is made and closed when the last one is gone, so
  a config with none costs the compositor nothing.

## Authentication

When a program needs administrator rights, polkit asks an authentication agent. The compositor draws the password
dialog itself and checks the password with polkit's own helper, `polkit-agent-helper-1`. A script never draws it, reads
the password or says that a password was right. `gnoblin.authentication` lets your config watch the requests and refuse
one.

```ts
import { AuthenticationListener, defineConfig } from "gnoblin";

class NoRoot extends AuthenticationListener {
    onAuthenticationRequest(request) {
        if (request.identities.includes("unix-user:root") && request.subject !== "pkexec") {
            request.decline("only pkexec may ask for root");
        }
    }
}

defineConfig({ setup: (gnoblin) => gnoblin.authentication.addListener(new NoRoot()) });
```

A listener is shown each request for a short window before the dialog opens, and may refuse it. A request carries:

- `actionId`: the polkit action, such as `org.freedesktop.policykit.exec`.
- `message`: the sentence polkit wants shown.
- `subject`: the program asking, such as `pkexec`. It is text from polkit, not an identity to trust.
- `appId`: always empty, because polkit names a process and not an application.
- `identities`: who may authenticate, written `unix-user:alice`.
- `deadline`: when the compositor takes the request back.

There is no `allow` and no way to read or give a password. `request.decline(reason)` is the only call: polkit is told the
request is not authorized and the dialog never opens. The reason is for the log. A listener that does nothing, throws or
runs past its budget leaves the request to the dialog.

The cookie polkit gives the request never reaches a script. A shell cannot claim the `auth.polkit` slot, so a reload or a
crash of the shell leaves a request in the compositor's dialog, where the user or the requester can still end it.

Polkit allows one agent per session. If `polkit-gnome`, `polkit-mate` or another agent is running, it remains the agent.
See [authentication prompts](/shell-api/authentication) for how the dialog and the helper work.

## Launch feedback

`gnoblin.launches` tracks the busy state between asking for an application and its window.

```ts
const launch = gnoblin.launches.begin("fuzzel-1", "org.example.App", {
    timeoutMs: 5000,
    onStarted: () => hideSpinner(),
    onTimedOut: () => hideSpinner(),
});
```

`begin(token, application, options)` returns a `Launch` that is `pending` at once, with `revision` 0. Anything outside
these limits throws:

- `token`: a non-empty string of at most 128 characters.
- `application`: a non-empty string of at most 512 characters.
- `timeoutMs`: an integer from 100 to 10000. The default is 3000.

A launch leaves `pending` once, for one of three states:

| State       | When                                                                                        |
| ----------- | ------------------------------------------------------------------------------------------- |
| `started`   | A window of the application appears, takes focus or names its application ID.               |
| `timed_out` | The deadline passes first.                                                                  |
| `ended`     | `launch.finish()` is called, the token is begun again (`replaced`) or the registry is full. |

The handlers are `onChanged(launch, fields)`, `onStarted`, `onTimedOut` and `onClosed(launch, reason)`, which runs last.

`gnoblin.launches.list()` returns the registry, oldest first. It holds at most 64 launches. A new one drops the oldest
finished one. With all 64 pending, the new launch comes back `ended`, with the reason in its `onClosed`.

A window matches by application ID, compared lowercase with a trailing `.desktop` removed. A window starts only the oldest
pending launch it matches, and a window that was already there when the launch began does not start it until it takes
focus.

This is the shell's own record. The compositor keeps no launch state, so another client does not see these launches, and a
`Launch` is not linked to the window or process that answered it.

## What has no TypeScript counterpart

These pages of [the Lua shell API](/shell-api/) are not ported. Each is marked "not available yet" there, and the Lua host
does not implement them either; the table says what is missing.

| Lua page             | Why TypeScript has nothing                                                                                                                                                  |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `privacy.md`         | The compositor publishes no privacy state. The PipeWire monitor serves it on the session bus, which a shell does not reach.                                                 |
| `input-grab.md`      | The schema carries no key events. A seat claim takes a chord, never the keyboard, so there is nothing to lease.                                                             |
| `text-insertion.md`  | There is no text-input target or `text_target` grant in the schema.                                                                                                         |
| `tablet-pad-help.md` | The compositor sends no `pad-help-requested` event. `Seat.device` lists a pad's name and capability and nothing more.                                                       |
| `window-snapping.md` | There is no grant to spend: the shell owns the tree and writes a window's `bounds` itself. The pure edge-tiling functions are in `gnoblin/snap` (`snapClient`, `edgeZone`). |

Adding any of the first four takes compositor work first: a schema object or message, then the TypeScript over it. A script
can never hide a capture indicator, so a privacy API would be read-only.
