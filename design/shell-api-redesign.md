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

**Round 2: finish the object model.** Section 1 of the review (no two Lua names for one action) and
section 2 (output, shortcut, launch, input source and process objects) land in four steps. Phase 5
and gap 7 are done (see the end of this list).

- Step (a), done. `gnoblin.on` and `gnoblin.once` replace the `events` namespace, including
  `events.mutter`.
- The workspace functions `rename`, `remove`, `activate`, `move_active` and `move_window`, and
  `Workspace:move_here`, are gone. A `Workspace` object renames, removes and activates itself, and
  `Window:move_to_workspace` moves a window.
- The `animations.seek`, `step`, `play`, `pause` and `stop` session functions are gone. An
  `AnimationPreview` has the methods.
- `Window:set_minimized(enabled)` replaces `minimize`, `unminimize` and `toggle_minimize`.
  `Window:restore` is gone: it is `set_minimized(false)` plus `set_maximized(false)`.
  `Window:restore_or_minimize` stays, because it restores the frame saved before a snap.
- `launches.snapshot` is gone from Lua. Each `Launch` in `launches.list` has its own revision.
- Socket clients keep every wire name.
- Step (b), done. `Output` is a live object (`alive`, `on`, `watch`, `changed` and `closed`)
  and `gnoblin.monitors` is gone from Lua. An `Output` carries its logical monitor's index,
  size and work area and a `privacy_screen` record. The control publishes merged records and
  `gnoblin.output.added`, `changed` and `removed` (API 1.88); `gnoblin.monitor.*` events and
  the `monitors.*` wire reads stay for socket clients only. Privacy screen is one global
  request: Mutter applies a single setting to every capable monitor and syncs the hotkey back
  to it, so a per-output request would need a Mutter patch. A method on one `Output` that
  changes the other outputs was misleading, so the write is `gnoblin.outputs.set_privacy_screen(value)`
  (`true`, `false` or `"inherit"`; `true` and `false` fail when no output has the hardware) and
  `Output` keeps only the `privacy_screen` read property, its `watch` and the `changed` signal.
- Step (c), done. `shortcuts.bind` returns a `Shortcut` and `launches.begin` returns a `Launch`
  (both interned, with `alive`, `on`; a `Launch` also has `watch`). `unbind`, `end_session` and
  `finish` are methods; `shortcuts.unbind`, `shortcuts.end_session` and `launches.finish` are
  gone from Lua. The objects exist before the compositor confirms them and end with the reason
  when it refuses. `shortcuts.list` lists this runtime's bindings; configured shortcuts moved to
  `shortcuts.configured`. A `Launch` does not link to a window or `Process`, because the
  compositor does not record the match. No wire change.
- Step (d), done. `InputSource:select()` replaces `input.select_source`; `input.sources` and
  `input.current_source` return live `InputSource` handles (keyed by type and id), moved by the
  existing `gnoblin.input.source-changed` and `sources-changed` events (`changed`, `selected`,
  `closed`). `Process` has `alive` and `on("closed")`; the end signal is `closed`, not `exited`,
  so every object ends the same way, and `gnoblin.process.exited` stays the wire event. No wire
  change.
- Gap 5, done. A throwing handler is logged with its event and stack and stays registered; the
  other handlers of the event still run. Before, `gnoblin.on` listeners and input handlers were
  disabled after one error (a listener by an `_active` flag, an input handler by removing it), and
  the input loop stopped at the failure. All handler sites now go through `call_handler`
  (`gnoblin-lua-dispatch.c`): a stack traceback, a full instruction budget for each call (the old
  budget was shared by one dispatch, so a runaway handler made every later handler fail) and a
  count of consecutive failures per handler that only limits the log (three, then one in 1000).
  A handler is never disabled. When an event handler fails, the configuration edits made during
  that event are rolled back as before; a per-handler rollback would need a config snapshot before
  every handler call. Answerable events (the other half of the gap table row) stay open.
- Gap 11, done.
    - Idle. `gnoblin.idle.watch(seconds)` returns a live `IdleWatch` (`alive`, `idle`, `seconds`,
      `on` with `idle`, `active` and `closed`, `watch`, `cancel`). A watch is a pair of watches on
      Mutter's core idle monitor, owned by the runtime generation that made it; a reload, a worker
      restart and the compositor's stop remove it. At most 256 for each runtime. The compositor's
      own 120 s sample, which socket clients read as `session.activity`, is one in-process watch of
      the same kind. `gnoblin.session.activity()` is gone from Lua and `gnoblinctl lua`.
    - Power. `session.suspend`, `hibernate`, `reboot` and `poweroff` run in the compositor
      (`gnoblin-control-power.c`): read the logind capability, call the action, and finish the
      Operation with the answer. A capability of `no` or a polkit refusal is `denied`; `na` is
      `unsupported`. The Lua runtime no longer exits for a power action and the supervisor has no
      power exit path. Bridge API 1.89.
    - Lock. A Lua configuration that listens for `gnoblin.session.lock-requested` counts as a
      listener, so `gnoblinctl session lock` reaches a config-only shell and its `Request` of kind
      `"lock"` finishes with `locked` or `timeout`.
    - `org.gnoblin.SessionIdle` stays. The `gnoblin-idle` service still exports it with
      `org.freedesktop.ScreenSaver`, the session target still starts that service and its test
      still covers it. The compositor no longer reads it, because idle comes from the monitor
      directly. It is a published interface for other programs.

- Gap 8, done. `gnoblin.settings.override(path, value)` and `Window:set_effect(name, value)` return a live
  `Override` (`alive`, `on("closed")`, `revoke()`, bridge API 1.90).
    - Before: two settings had runtime overrides, each its own call and flag
      (`input.set_orientation_lock`, `monitors.set_privacy_screen`); per-window effects existed only
      as config window rules. Both flags lived in the compositor and a reload cleared only the
      orientation lock.
    - One list in the compositor (`gnoblin-control-overrides.c`) holds every override, oldest
      first, with an owner: a runtime generation, or none for a socket client. The latest override
      of one target wins and the older ends with `replaced`. A reload that commits ends all of them
      (`config_reloaded`), a worker restart ends the runtime's, and a closed window ends its effects
      (`window_closed`). The Lua runtime ends its own handles just before it frees the old runtime,
      so listeners of `closed` run once with the reason.
    - Settings that can change at runtime are the sections the compositor applies from the
      document (`window_management`, `compositor`, `cursor` theme and size, `layer_shell`
      `preserve_active_window`) and the two that have their own Mutter state (the orientation lock
      and the privacy screen). The list is one header shared by the runtime and the compositor
      (`gnoblin-overrides.h`), so a refusal names its reason at the call. The configuration
      validator checks the value against the committed document in the runtime.
    - A document-backed override is an overlay on the compositor's runtime cache: every reader sees
      it, `gnoblin.settings` and the worker's committed copy do not. `native_apply_runtime_preferences`
      applies it again from the overlay.
    - A window effect is one more rule after the configured ones in `native_apply_window_rules`, so
      it uses the rule engine for blur, opacity, shaders, frames and corners and invents no effect.
    - The two old Lua functions are gone, with their `gnoblinctl lua` forms. The wire methods stay
      for socket clients; they set an override with no owner.
    - Not done: overriding input device settings (mouse, touchpad, keyboard layout), which have
      their own apply paths, and animations and workspace placement rules, which are not effects.

- Gap 9, one chord grammar. Every mechanism already used the GTK and Mutter accelerator syntax
  (`<Super>q`), but each checked it on its own and none checked it strictly.
    - Before: `configure.shortcuts` and `shortcuts.bind` checked the length and passed the string to
      `meta_display_grab_accelerator`, so a mistake reported "already claimed". `keybindings.keyboard`
      and `input.on` called `meta_parse_accelerator`, which skips an unknown `<Foo>` and binds the bare
      key. Pointer chords had a fourth token list (`<Mod1>` to `<Mod5>`), and the stylus keybindings
      checked nothing.
    - One parser, `src/control/gnoblin-chord.h`, shared like `gnoblin-easing.h` (the Lua runtime and
      the config validators include it; the control manifest copies it into Mutter). It reads
      modifiers in angle brackets and one key, refuses everything else, and answers with the form to
      write. It reads `<Primary>`, `<Ctrl>`, `<Ctl>`, `<Shft>` and `<Mod1>` as the canonical
      modifiers, because the pinned keybinding catalogue itself uses `<Primary>`; `<Mod2>` to
      `<Mod5>` depend on the keymap and are refused. The written form puts the modifiers in the
      order of `meta_accelerator_name`.
    - It is called by the config validators (`shortcuts`, `keybindings.*`, input handlers, stylus
      keybindings), by the Lua runtime at the call (`shortcuts.bind`, `input.on`, keyboard and
      pointer bindings) and by the compositor (`shortcut.bind`, configured shortcuts, built-in action
      bindings, handler accelerators). The compositor passes Mutter the written form.
    - The four mechanisms stay: `configure.shortcuts` runs in the compositor and survives a stopped
      Lua runtime; `keybindings` is a named declaration that later files can override; `input.on`
      handles any input event; `shortcuts.bind` is a run-time object with held-modifier sessions.
      `keybindings.keyboard` and `.pointer` are sugar over `input.on`, kept for their names.

- Gap 9, leased keyboard grab. `gnoblin.input.grab(options?)` returns a live `Grab` (`alive`, `timeout`, `on`
  with `key` and `closed`, `renew()`, `release()`; bridge API 1.91).
    - Before: the only keyboard grab was the shortcut session (`modal` or `capture_input`): one slot
      (`control->active_shortcut_session`), a fixed 10 s timer in two places, and key events with a
      keysym and no text.
    - New `gnoblin-control-grabs.c`: one grab slot, owned by a runtime generation, on Mutter's one native key
      capture slot. A lease: it expires after `timeout` idle seconds; each key event and `input.grab.renew`
      restart the timeout. Key events carry the keysym name, key code, modifier mask, press or release,
      repeat, and `text`, from the keysym and the compose table of the locale (libxkbcommon). A dead key or
      Compose sequence gives `composing = true`, then one composed character on the completing key. Control,
      Alt and Super make a press a command with no text.
    - Ends: `released`, `expired`, `replaced` (a second grab), `locked` (the lock state change), `preempted`
      (Mutter cancels the capture, for example for a popup grab), `config_reloaded` and `runtime_stopped`.
      The reload reason comes from the Lua runtime before it frees the old state, as for overrides, and the
      worker ends its grabs with `runtime_stopped` when its main loop returns. A restarted worker loses its
      grab with no signal; the compositor gives the keyboard back.
    - Boundaries kept. The call refuses while the session is locked (Mutter does not call the capture
      handler during a lock either). Grab events carry no `focus_context`, so a script cannot focus or move a
      window with held keys. A grab never takes the capture slot from a shortcut session or
      `shortcuts.capture`, and they cannot start while it holds it.
    - Decision: shortcut sessions do not move to the lease. A session is a user gesture: it starts and ends
      with a held modifier, nothing owns it to renew it, and its 10 s guards a lost release. Its key events
      carry a one-use `focus_context`, which a script-renewed lease must not carry. They share the capture
      slot and nothing else.
    - Not done: a grab that also takes pointer input, and a key the grab lets through to the focused window.

- Object completeness (round 2, audit items 3, 7, 8, 9, 12, 13, 14, 17), done.
    - `Tween`, `Process` and `AnimationPreview` are live kinds in `gnoblin-lua-objects.c`: interned by `id`, with
      `alive`, `on`, `watch`, `revision` and a `closed` signal. A tween has `"finished"` and `"closed"` (the outcome is
      the reason) and no `on_finished`. `Tween:property` takes its allowed `(kind, property)` pairs from the
      `animatable` table of the kinds, and the wire track carries `target_kind`. A process has
      `gnoblin.process.list()` and `by_id(id)`; `Window.process` is that id. A preview has no `session` field, only `id`.
    - One subscription shape: `on`, `watch`, `gnoblin.on`, `Request:on` and `gnoblin.input.on` return a
      `Subscription` with `unsubscribe`. `input.on` removes the handler from the configuration document.
    - `watch` takes only fields the annotation lists with `@observable`; others raise an error that names the
      fields that work. `Workspace` has `revision`. `Shortcut`, `Override` and `Grab` have none, because their
      fields never change, and a `Request` has none, because its `status` is its own outcome.
    - A reload (and a stopping worker) ends `Shortcut`, `IdleWatch`, `Launch`, `Process`, `Tween` and
      `AnimationPreview` as it already ended `Override` and `Grab`, and runs the `closed` listeners of pending
      `Request`s with `config_reloaded`. A reload waits for the pending `Operation` of a playing tween, so only an
      unplayed tween is alive at a commit.
    - `scripts/audit-lua-annotations.py` derives the record classes from the annotations and checks the hand-built
      methods (`Operation`, `Request`, `Grant`, `TextTarget`, `SnapContext`) in both directions.

- Gap 7, done. `FocusContext` and `MenuContext` are one `Grant` (bridge API 1.92): `event.grant` on a
  shortcut, pointer binding, shortcut session key, input event and WM menu request.
    - Source facts. The compositor made one `NativeFocusContext` per gesture (key press, pointer binding,
      matched input event) and the first of five operations (`window.focus`, `window.begin_move`,
      `window.begin_resize`, `window.snap_context`, `input.text_target`) removed it. It lasted five seconds,
      died at a reload and a lock, and a socket token for it was also removed at first use. A WM menu
      request had its own `MenuContext` (one move or resize). `Window:present` is separate: it needs no
      gesture and keeps its own attribution rule.
    - Now. The context keeps a `used_scopes` mask; each operation takes its own scope once
      (`native_focus_context_claim`), a failed operation still spends its scope, and the context ends with
      its fifth scope, at five seconds, at a reload and at a lock. A script holds a `Grant` userdata, not
      the proof. `grant:narrow(scope, window?)` returns a Grant for fewer operations and one window; it never
      widens, and every Grant of a gesture shares what was spent (`LuaConfig.grant_spent`). The Lua runtime
      checks scope and window; the compositor checks the window again (`grant_window` in the operation packet)
      and owns expiry and the lock. Timers and events that are not gestures carry no Grant.
    - Menu requests. A menu Grant has `begin_move` and `begin_resize` for its window and takes one of them.
      The Lua operation sends the menu capability, not a gesture context; the packet writer used to demand
      a gesture context for every `window.begin_move`, so a Lua menu move could not reach the compositor.
    - Wire. Socket clients keep the `focus_context` field and the method names. Their token now lasts for
      the gesture and each operation uses it once (API 1.92); the connection that uses a gesture first keeps
      it. The wire has no narrowing. `gnoblinctl lua` gives the same `Grant` and narrows in its own process.
    - Order. The compositor checks focus identity for `snap_context` and `text_target`, so take them before
      `focus`.

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
    tween:on("finished", function() end)
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
  visibly diverge. Order them with the `"finished"` signal instead.
- Verified: a tween owned by the runtime is cancelled when the runtime reloads, and a tween whose targets all close
  ends early and still reports `finished`.
- Named lifecycle animations (`gnoblin.animation`) stay declarative and run natively, so a window opening never waits on
  Lua. They share the easing and property names above. A later `tween:play(name)` can reuse their tracks.
- The preview session API (`preview`, `seek`, `step`, `play`, `pause`, `stop`) and `inspect` stay as they are for now.
  They appear to inspect a configured animation on a visible target, not to run a user-owned animation, but that is not
  verified. Check the lifecycle in `patches/mutter/99-typed-window-api/0024-native-animation-runtime.patch` before
  merging any of it with tweens. Runtime tweens are a separate, additive capability.
