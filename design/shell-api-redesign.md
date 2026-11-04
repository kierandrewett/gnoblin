# Shell API redesign: gaps and plan

Status: implemented on `refactor/lua-api-table`: the `lua_api[]` descriptor table and phases 1 to 4, plus the object
model (live objects, window visual transforms and native tweens). Phase 5 below stays open by design. The findings come from six read-only audits of the Lua API stubs (`src/config/lua/api/*.h`), the runtime and
control code, and the docs; items the auditors could not confirm in source were checked before implementation.

## The pattern behind the original bug

A shell could not show a passphrase prompt that accepted typing. The shell runs a prompt program in response to
`gnoblin.prompt.requested`. The window never got keyboard focus, because focus authority exists only for key presses
(`FocusContext`, single use, five seconds). The first fix special-cased `commands.capture`. The audits found the same
shape everywhere: a general capability is missing, so each feature grows its own token, its own verb, or its own limit.

## Principles

1. A capability is general or it does not exist. No option on one call that exists for one feature.
2. Separate **attribution** (who started this window or process, proven by the compositor) from **authority** (what that
   party may do). Apps must never be able to forge attribution by choosing an app ID or title.
3. The shell is trusted code the user chose to run. Security boundaries protect the user from applications, not from
   their own shell. Keep the boundaries that stop applications stealing focus, reading secrets or bypassing the lock.
4. Positional functions. No `fn {a = 1}` calls. Optional arguments come last.
5. Everything asynchronous is an Operation with the same verbs: cancel, timeout, map. Time exists (`timers`).
6. Every request has the same lifecycle: offered, claimed, answered or ended, with one `finished` outcome.
7. Read, write and observe are symmetric for every piece of state.
8. One schema drives the Lua stub, the socket protocol and `gnoblinctl`.

## Gaps, ranked

| #   | Gap                                                                                                                                       | Missing primitive                                                                  | Size |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- | ---- |
| 1   | `commands.run` is fire and forget; `capture` caps stdin at 4096 bytes, stdout at 64 KiB, drops stderr, rejects non-UTF-8                  | Process object: pid, env, cwd, stdio modes, kill, wait, exit event                 | L    |
| 2   | No link from a launch or process to the windows it creates                                                                                | Compositor-verified attribution: `Window.pid`, `Window.launch`, `Launch:on_window` | M    |
| 3   | Focus authority comes only from key presses; system requests carry none; `Window:focus` reports "accepted", not "focused"                 | Authority for windows the shell itself launched; outcome-bearing `Window:present`  | M    |
| 4   | Each request event has its own answer verbs, timeout, ownership and cleanup (prompt, auth, location, lock, menu)                          | One `Request` with claim, positional answer, uniform `finished` outcome            | L    |
| 5   | Handler return values are discarded; a throwing handler is disabled for good                                                              | Per-handler fault isolation; answerable events                                     | M    |
| 6   | No timers, no clock, no Operation combinators or cancel                                                                                   | `timers.after`, `Operation:map/cancel`, `operations.timeout`                       | M    |
| 7   | `FocusContext` mixes gesture proof with capability; five operations share one single-use token                                            | Scoped, narrowable `Grant`                                                         | L    |
| 8   | Runtime overrides exist for two settings only; per-window effects are config only                                                         | `settings.override`, `effects.set` with revocable ownership                        | L    |
| 9   | Keyboard grabs end after 10 s, have one slot and no composed text; shortcut registry is four mechanisms                                   | Leased `input.grab`; one chord grammar                                             | L    |
| 10  | 38 of 128 calls take a single table; naming and aliases are inconsistent; some live functions are missing from the stubs                  | Positional conventions and one generated schema                                    | L    |
| 11  | Idle is one hard-coded 120 s watch; power actions report success before logind answers; lock requests never reach a config-only Lua shell | Parametric idle watches; honest power results; uniform lock request                | M    |

## Focus decision to confirm

Gap 3 is the original bug. Two ways to give a prompt window focus:

- **Attribution plus ownership (proposed).** The runtime spawns a process and gets a `Process`. The compositor
  attributes the process's windows to the launch (session leader and activation token, not app ID). The runtime may
  focus windows attributed to its own launches without a key press. Applications cannot use this: they cannot create a
  launch.
- **Grant on request events (rejected for now).** Giving `gnoblin.prompt.requested` a focus grant ties authority to the
  lifetime of a request that an application can raise at any time. That does not show user intent and weakens the focus
  boundary.

Risk that remains under the proposal: an application can trigger a prompt at a moment the user is typing, so the prompt
window takes the keystrokes. The runtime decides when to launch, so the shell owns that policy. A short idle gate before
activation (no key press in the last 250 ms) should be the default behaviour of `Window:present`.

## Direction from the maintainer

The goal is a runtime that feels alive: handles you can hold and call, state you can read and observe in the same shape,
and time and cancellation you can use. Decisions so far:

- No builder objects such as `Process.new`. Options are trailing positional arguments.
- No commitment to coroutines or `await` yet. Handlers run through `lua_pcall` with event-action limits, so a yielding
  wait needs a scheduler with resume, reload and cleanup rules. Prove that against real shell tasks first. Until then,
  completion stays explicit (`Operation`, callbacks), and `timers.after` returns an Operation.
- Adding any new native method touches about eight registries (Lua stub, `gnoblin-lua.c` method and alias tables,
  `gnoblin-lua-install.c`, `gnoblin-lua-operations.c`, the runtime channel, the console, the socket protocol, the CLI).
  Do this once through a descriptor table (gap 10) before adding many methods.
- Gaps 8, 9 and 11 are opportunities, not defects. Build them only when a shell task needs them.
- `scripts/audit-lua-annotations.py` reads only the names in `gnoblin-lua.c`, so it missed functions registered in
  `gnoblin-lua-install.c`. Extend it.

## Phases

Each phase leaves the tree working and is its own pull request.

1. **Foundation. Done.** `gnoblin.process.spawn(argv, options?)` returns a `Process` with `pid`, `wait()` and `kill()`;
   its exit is a runtime event, so a long-lived child never holds a reload back. `gnoblin.timers.after`, `timers.now`,
   and `Operation:map`, `timeout` and `cancel`. `commands.run` and `capture` were removed, not wrapped:
   `design/lua-api-goal.md` rules out shims, and every caller moved to `process.spawn`.
2. **Attribution and focus. Done.** `Window.pid` and `Window.process` (the client is the spawned process or runs in its
   session; pid from the display server) and `Window:present()` with a 250 ms input-quiet gate. `Process` takes the role
   of `Launch`, so there is no separate `Launch:on_window`; shells match `window.process` in `gnoblin.window.created`.
   This replaces PR #314.
3. **Requests. Done.** One `Request` (`event.request`) for prompt, auth, location and lock, with `finished()` and the
   answer verbs only on the kinds that take them. The Lua verbs are gone; socket clients keep the wire methods, and
   the compositor now checks that a socket client received the request it answers. Auth ran through real polkit in a
   separate `ssh localhost` login session (`pkexec`), and location through a real GeoClue on a private bus with
   network off. GeoClue asks agents only for Flatpak apps, which it finds from `/proc/PID/cgroup`; this host's
   extra `net_cls` cgroup line hides them, so the test sends `AuthorizeApp` itself. Location requests now end
   with `gnoblin.location.authorization-finished` (API 1.87), so a Lua `Request` reports the outcome when a
   shell process answered it.
4. **Conventions. Done.** Positional signatures driven by the annotations (`named_arguments` in `lua_api[]`, one
   adapter); duplicate Lua names removed while their wire names stay for shells. `configure`, `animation`,
   `window_rule` and `array` keep a table because they declare data; filters stay trailing option tables.
5. **Overrides, input grabs, idle.** Gaps 8, 9, 11.

**Object model. Done.** Windows, workspaces and layer surfaces are interned live objects with `alive`, `on` and
`watch`; `watch` accepts the fields the annotated class declares. `Window:set_opacity/scale/rotation/offset` and
`reset_visuals` (one operation, `window.set_visual`); clicks follow the drawn position, a reload resets them and the
lock refuses them. `gnoblin.tween()` compiles to one description that the compositor plays on a stage timeline;
`gnoblin.ease` shares its curves with the compositor through one header.

## Decisions

- Hard cutover, no compatibility wrappers (`design/lua-api-goal.md`).
- Names: `gnoblin.process.spawn`, `Window:present`, `gnoblin.timers.after`, `Operation:cancel`.
- `Operation:cancel()` means stop waiting. A timer stops; an action already sent to the compositor still runs.

## Open decisions

- A change made while a lifecycle animation plays is overwritten when that animation restores its saved state.
- `gnoblinctl lua` has the visual methods but no tweens and no live objects: tween callbacks and object signals
  run in the configuration runtime, which outlives a console script.

## Windows as objects you can transform

The base primitive is not an animation API. It is a window you can change. `Window` and layer surface records are
detached, read-only snapshots, and Lua runs behind a separate channel. So a write must look like what it is: an explicit
call that returns an Operation, never a hidden assignment.

```lua
window:set_opacity(0.5)
window:set_scale(0.9)
window:set_rotation(5)
window:set_offset(40, 0)
window:reset_visuals()
```

- Reads come from the snapshot (`window.opacity`) and carry a revision, as every other field does. A write may land
  after a newer snapshot, so code that needs the result uses the Operation.
- These are meant as visual transforms only: not layout, workspace, focus or what the client sees. Open constraints to
  verify in Mutter before any implementation: whether input hit testing follows an actor transform, how a transformed
  window interacts with stacking and with lock surfaces, and whether a runtime reload can reliably clear overrides it
  owns.
- Tweens and named animations are conveniences on top. Anyone can also build their own animation from these calls.
- Verified: input hit testing follows the transform, a reload clears the overrides, and changes are refused while the
  session is locked.
- Calling these every frame crosses the runtime channel, so it will not match native frame timing. Tweens therefore
  compile to one description that the compositor plays itself. A frame callback for custom loops is a separate decision,
  to be measured before it is added.

## Animation: generic tweens

Animation is not a window feature. A tween changes a declared property of any supported target over time: a window
transform, a layer surface, an output setting, an effect parameter such as a blur radius. Interpolating plain Lua values
is a separate step, below. Godot's useful ideas are a one-off chain you build in code (Tween), a reusable named
animation (AnimationPlayer), and the fact that any object with properties can be a target. Gnoblin takes those ideas,
not the Godot API.

Three layers, each usable alone:

1. **Math.** `gnoblin.ease(name, t)` and `gnoblin.lerp(a, b, t)` are pure functions. Any code can use them, with no
   runtime or target involved.
2. **Tween.** A timeline of steps with seconds as the unit, like Godot:

    ```lua
    local tween = gnoblin.tween()
    tween:property(window, "opacity", 0.0, 0.25, "cubic", "out")
    tween:parallel():property(layer, "opacity", 0.4, 0.25)
    tween:interval(0.1)
    tween:callback(function() window:close() end)
    tween:on_finished(function() end)
    tween:pause() tween:play() tween:seek(0.5) tween:kill()
    ```

3. **Targets.** One timeline model, with an adapter per kind of target. An adapter declares which properties it supports
   and their types, such as number, 2D vector or colour, and which interpolation applies. Windows, layer surfaces,
   outputs and effect parameters are adapters that the compositor runs in its render loop. A property or type that no
   adapter declares fails with a clear error. Nothing is animated by guess.

    A separate step, `method(from, to, seconds, fn)`, calls a Lua function with the current value. It is not part of the
    native path. It stays out of the first version until the runtime has a scheduler and its frame timing is measured.

- Native steps in one tween compile to one description that the compositor plays. First-frame timing and frame pacing do
  not depend on the Lua runtime.
- A timeline runs on one backend. Native tracks and Lua-driven steps are not mixed in one tween, because they would
  visibly diverge. Order them with `on_finished` instead.
- Verified: a tween owned by the runtime is cancelled when the runtime reloads, and a tween whose targets all close
  ends early and still reports `finished`.
- Named lifecycle animations (`gnoblin.animation`) stay declarative and run natively, so a window opening never waits on
  Lua. They share the easing and property names above. A later `tween:play(name)` can reuse their tracks.
- The preview session API (`preview`, `seek`, `step`, `play`, `pause`, `stop`) and `inspect` stay as they are for now.
  They appear to inspect a configured animation on a visible target, not to run a user-owned animation, but that is not
  verified. Check the lifecycle in `patches/mutter/99-typed-window-api/0024-native-animation-runtime.patch` before
  merging any of it with tweens. Runtime tweens are a separate, additive capability.
