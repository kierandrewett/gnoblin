# Changelog

## Unreleased

### Added

- The schema format accepts `base` on an object (a class in the same namespace it inherits from; its properties, methods and signals come first in the derived class, and a property may not change type), `default` on a property (checked against its type), and `readonly` and `animated` booleans on a property (default false). Every existing schema parses unchanged. Node-core step 3, slice 1; the tree classes come in slice 2.
- The Lua module `gnoblin.shell.floating` (`data/lua/gnoblin/shell/floating.lua`) places floating windows in free space, cascade and centre, as the `Floating`, `Cascade` and `Centred` classes on the node class system. Its placement matches the 212 committed placement goldens. The compositor still places floating windows natively; the module is not loaded yet.
- The shipped `gnoblin.layouts.strip` layout places windows as columns in a strip wider than the screen, with a view
  offset, stacked or tabbed columns, and the `width`, `consume`, `expel`, `move_column`, `fit_all`, `toggle_tabbed`,
  `focus` and `scroll_to` commands. It replaces the scrolling layout, which is removed.
- The window manager accepts up to `MAX_WINDOWS` (256) windows. A window past the cap is refused with
  `TooManyWindows` before any state changes. Each provider in the layer registry has its own limit; extension
  providers keep the default of 32.
- Named workspace sets: the `workspace_sets` field of `gnoblin.configure` declares sets that name the workspace each
  output shows, checked as a whole list when the session starts. `gnoblin.workspaces.activate_set(name)` and
  `gnoblinctl workspace activate-set NAME` make a set live; an unknown name is refused with `unknown-set`. A named
  output that is not connected falls back to the primary output with a logged warning.
- `Window.client_type` (`"wayland"` or `"x11"`, bridge API 1.93) in window records, `gnoblin.find("Window", {client_type = ...})`,
  the window rule `match.client_type` key and `gnoblinctl window list` and `window match`.
- The services role starts the GeoClue agent next to the polkit agent, with the policy of the `location` table
  in the configuration. Until now the role claimed the `location.agent` slot but never served it. With no system
  bus the agent is skipped and the role carries on.
- A Vulkan renderer for the Rust compositor (`gnoblin-backend`, feature `vulkan`), drawn with `ash` and the shared WGSL
  passes compiled by `naga`: solid and `wl_shm` surface layers, opacity, tint, corner radius, the portal overlay and
  the cursor, readable back as an image and compared pixel for pixel with the software renderer. It runs on a GPU or
  on a software driver such as lavapipe. Presentation, dmabuf import and effects are not done yet. The new
  `compositor.renderer` setting (`auto`, `vulkan`, `gles` or `software`) chooses the renderer; `auto` keeps today's
  choice, and a named renderer the session cannot create is refused with the setting named.
- Text a child prints without the `gnoblin-log` facade (a panic, a Lua `print`, output from a C library or Xwayland)
  is logged as component `raw`, tagged with the child's role and stream. Only the compositor, runtime and portal roles
  are captured by default, and the services role only with `GNOBLIN_LOG_RAW_SERVICES=1`; other children are read and
  discarded. Each line is scrubbed of secret-looking text on a best-effort basis that cannot recognise every secret, see
  `docs/guides/logging.md`.
- The screensaver service watches logind. A `PrepareForSleep(true)` (suspend or hibernate) locks the session while the
  sleep inhibitor is still held, then releases the inhibitor. A `Lock` signal on the session's own logind object locks
  the session. The service takes the inhibitor again on resume. A logind lock on another session is ignored. Without a
  system bus the watch is skipped.
- Window rules accept a workspace name declared in `workspaces`, in the `workspace = {id = ...}` target and the `workspace-id` match. A declared name resolves to its position. An undeclared name that is not a number is refused when the configuration is committed, with the field named. The declared list also sets the persistent workspace count.

### Fixed

- Window records past the 32nd window do not trip a debug assertion, because the window manager's layer provider has
  its own limit.
- The `layout_failed` signal's documented `reason` values now match the ones it emits.
- Pointer picking goes through the scene: a press or pointer motion over a window that a layout turned or moved
  reaches the window where it is drawn, not the unrotated rectangle it was placed in. Stacking ties still go to the
  last-drawn window.

### Changed

- Log records are written to `gnoblin.jsonl` in the state directory, one JSON object per line. `gnoblinctl log` and
  `gnoblinctl log --follow` read that file. The `gnoblin-store` crate, the SQLite log store, `gnoblinctl store`, the
  store IPC verbs and the `--store` flag are removed. `gnoblin.jsonl` and the legacy `store` files are not deleted.
- Log messages no longer start with a `[tag]` prefix. A crate's logger names its component, so a tag equal to
  that component is dropped, and any other tag becomes an `area = "tag"` field on the event. `xtask logging` fails a
  level macro whose message still starts with `[tag] `.
- Layouts with `extent = "unbounded"` must declare `kind = "tiling"`; a floating or kiosk layout that asks for it is
  refused when it loads. The wm now enforces `extent` when it settles a reply: a placement whose rectangle (before its
  transform) lies outside the output, or past the coordinate bound when unbounded, refuses the whole reply. The
  `carousel3d` layout declares `extent = "unbounded"`.

- Xwayland windows get Gnoblin's own frame, rules and effects like Wayland windows. Mutter no longer launches
  `mutter-x11-frames` or reparents X11 clients into frame windows. An X11 window that does not turn decorations off is
  framed under `frame.mode = "auto"` (the default for X11 when no rule sets a mode), `_GTK_FRAME_EXTENTS` feeds the
  visible bounds, `_NET_FRAME_EXTENTS` and `_NET_REQUEST_FRAME_EXTENTS` report Gnoblin's border, and Mutter's stock
  shadow no longer draws for managed X11 windows. A border change keeps the client where it is.
  `mutter-x11-frames` is no longer built or installed. A request for frame extents before the window exists is answered
  from the rules that match only the window type and `client_type`. The Mutter code for a frame window (`MetaFrame`,
  its sync counter and borders, the `_MUTTER_FRAME_FOR`, `_MUTTER_FRAME_EXTENTS` and `_MUTTER_NEEDS_FRAME` properties,
  and the frame mask in the X11 window actor) is removed in patch `11-window-frames/0011`.

### Changed

- `patches/` is organised into topic directories (`patches/mutter/<NN>-<topic>/<NNNN>-<slug>.patch`) with dense numbers,
  and each patch project has a `series` file that is the only source of apply order. The patch bodies, their order and
  the patched trees are unchanged (`design/patches-layout-mapping.tsv` maps old paths to new). CMake, the Nix package,
  `scripts/check-build-deps.py`, `scripts/list-tarball-sources.sh` and the tests read the series instead of sorting a
  glob. `scripts/manage-patches.py check` now fails when a patch file is missing from its series or the series names
  a missing file, and `renumber` and `path` are new. `patches/README.md` describes the layout.
- One table of wire operations (`src/control/gnoblin-api-operations.h`, round 2; no wire change, API stays 1.92).
  The version gates and their errors, the hello method list, the `gnoblinctl` minors, the supervisor packet flags and
  the compositor dispatch read it instead of repeating names in chains. Record methods of `lua_api[]` declare their
  positional arguments, and the runtime packs them from that schema with the same errors as before.
- `window:set_effect(name, value)` declares its arguments in `lua_api[]` like the other record methods, so a wrong
  call fails with `window:set_effect takes a name and a value`. `scripts/audit-lua-annotations.py` now also reads
  the schemas that `clang-format` wraps onto a second line, which it skipped before.
- `gnoblinctl lua` has the names and call shapes of a Lua configuration (round 2; no wire change).
    - The console reads `lua_api[]`, the table the runtime installs, instead of a prefix chain of its own. A name
      that the table does not hold fails with `gnoblin.NAME is not part of the Lua API`; the table of removed
      names (`removed[][]`) is gone. A name that needs a runtime (`gnoblin.timers`, `gnoblin.process`,
      `gnoblin.tween`, `gnoblin.idle.watch`, `gnoblin.input.grab`, `gnoblin.input.on`,
      `gnoblin.settings.override`, `gnoblin.load`, `gnoblin.array`, `gnoblin.snapshot`, `gnoblin.configure`,
      `gnoblin.window_rule`, `gnoblin.animation`, `Window:set_effect`, `Window:present` and the `on` and
      `watch` methods of records) raises one error: `NAME is not available in gnoblinctl lua because the
console has no runtime`. The generic call that accepted one table for any unknown name is gone, so a call
      is always positional. `scripts/audit-lua-annotations.py` checks that every console name is in
      `lua_api[]` and that every wire operation of a console record has a console method.
    - `gnoblin.find`, `gnoblin.ease` and `gnoblin.lerp` work in the console. `gnoblin.windows.list` and
      `gnoblin.layers.list` take no filter, as in a configuration; `gnoblin.config` holds only `path()`,
      `default()` and `restore_default()` and cannot be assigned to.
    - `gnoblin.animations.preview`, `inspect` and `surfaces` are gone from the console. `window:inspect_animation`
      and `window:preview_animation` (and the same on a `LayerSurface`) replace them. `SnapContext:commit`
      takes `(monitor, x, y, width, height)`, `Window:move_to_workspace` takes a `Workspace` and
      `Window:move_to_monitor` takes an `Output`, as in a configuration.
    - `gnoblin.location.authorize_app {request_id, allow, accuracy}` is gone. The `gnoblin.location.authorization-requested`
      event carries `event.request`, a `Request` of kind `location` with `answer(allow, accuracy?)` and `cancel()`.
- `gnoblin.find`, a read-only `gnoblin.config` and handle arguments (round 2; no wire change).
    - `gnoblin.find(kind, filter?)` lists the live objects of `Window`, `Workspace`, `LayerSurface`, `Output`,
      `Shortcut`, `Launch`, `InputSource` or `Process`. A table filter matches declared scalar fields
      (`gnoblin.find("Window", {app_id = "notes", focused = true})`); a function filter is a predicate. An unknown
      kind, an unknown field or a field that is not a scalar is an error that names what works.
      `gnoblin.windows.list` and `gnoblin.layers.list` take no argument in Lua now. `gnoblin.focus.history`
      keeps its filter, because its result has an order and a limit. Socket and `gnoblinctl` reads keep theirs.
      See `docs/shell-api/find.md`.
    - `gnoblin.config` holds only `path()`, `default()` and `restore_default()`. Assigning to it, or reading any
      other key, raises an error that points to `gnoblin.configure` and `gnoblin.settings.override`, while the
      config loads as well as at run time. `gnoblin.snapshot()` works only while the config loads and raises an
      error at run time; `gnoblin.settings` reads the committed values. Assigning to a named shortcut or
      autostart entry at run time is an error. At run time `gnoblin.configure`, `gnoblin.window_rule` and
      `gnoblin.animation` return an `Operation` that succeeds when the runtime commits the change, and fails with
      `invalid_argument` when the change is rejected.
    - `Window:move_to_workspace` takes a `Workspace`, `Window:move_to_monitor` takes an `Output`, and
      `gnoblin.layers.animation_policy` takes a `LayerSurface`; IDs and numbers still work.
      `gnoblin.animations.surfaces`, `inspect` and `preview` and the `AnimationSurface` record are gone: use
      `window:inspect_animation(name, event?)` and `window:preview_animation(name, event?, autoplay?)`, or the
      same methods on a `LayerSurface`. The wire methods `animation.surfaces`, `animation.inspect` and
      `animation.preview` stay.
    - `SnapContext:commit(monitor, x, y, width, height)` replaces `commit({monitor_id = ..., frame = {...}})`.
      `monitor` is an `Output` or a monitor ID.
    - `Window.monitor` is annotated as what it holds: the origin `{x, y}` of the monitor that holds the window, not a
      monitor index (`monitor_index` and `monitor_id` name the monitor). The name stays, because the socket
      `windows.list` returns the same record as Lua and `window.list` keeps `monitor`. `Window.offset` is `{x, y}` in
      the Lua record.
    - The layer animation rule loses the `in`, `out` and `ease` aliases. Use `layer_open`, `layer_close` and
      `easing`.
    - The generated stub is corrected: `Workspace.id` is required, `create` accepts an ID, and the window count of a
      `Workspace` is `window_count`, the key the record always had. The stub said `windows`, and
      `workspace:watch("windows", fn)` never fired.
- `Tween`, `Process` and `AnimationPreview` are live objects, and every subscription has one shape (no wire change;
  the tween track gains an optional `target_kind`).
    - Each has `id`, `alive`, `revision`, `on` and `watch`, and one handle per id. `tween:on("finished")` runs when
      the timeline ends; `on("closed")` runs for every end, with the outcome (`"finished"`, `"killed"`,
      `"cancelled"`) as its reason. `Tween:on_finished` is gone. `Tween:property` reads the properties it may drive
      from the `animatable` list of each kind, not from the word "Window".
    - `gnoblin.process.list()` and `gnoblin.process.by_id(id)` find a process again;
      `gnoblin.process.by_id(window.process)` resolves a window's `process`. `process:watch` follows `pid`,
      `status`, `exit_code` and `signal`. The process `closed` signal still runs once at the end. An
      `AnimationPreview` has no `session` field (use `id`) and ends when `stop()` succeeds.
    - `gnoblin.input.on`, `Request:on` and every object's `on` and `watch` return a `Subscription`; call
      `sub:unsubscribe()`. `gnoblin.input.on` returned a handler ID string before and could not be undone.
    - `watch` takes only fields that change while the object is alive (`@observable` in the annotations).
      `id`, `alive`, `revision` and fixed fields raise an error that lists the fields that work. `Workspace` has
      `revision`.
    - A config reload ends every object the runtime owns, not only `Override` and `Grab`: `Shortcut`,
      `IdleWatch`, `Launch`, `Process`, `Tween` and `AnimationPreview` run their `"closed"` listeners with the
      reason `"config_reloaded"` (`"runtime_stopped"` when the worker stops), and a pending `Request` takes
      `on("closed")` with the same reasons. See `docs/shell-api/types.md`.

- `FocusContext` and `MenuContext` are one scoped `Grant` (bridge API 1.92). A shortcut, pointer binding,
  shortcut session key, input event or window menu request carries `event.grant`. `grant:narrow(scope, window?)`
  returns a grant for fewer operations (`focus`, `begin_move`, `begin_resize`, `snap_context`, `text_target`)
  and one window; nothing widens a grant. Each operation of a gesture can run once, so one key press can focus a
  window and then begin a move; before, the first operation used up the whole gesture. Expiry (five seconds),
  reload and lock rules stay, and timers and other events have no grant. `Window:focus`, `begin_move`,
  `begin_resize`, `gnoblin.windows.snap_context` and `gnoblin.input.text_target` take a `Grant`;
  `event.focus_context` and `event.menu_context` are gone from Lua and from `gnoblinctl lua`. The default
  pointer bindings use `event.grant`. Socket clients keep `focus_context` and the method names; their token now
  lasts for the gesture and each operation uses it once. A Lua window menu move or resize failed before
  because the session demanded a gesture context for it; it now works. See `docs/shell-api/grants.md`.

- One chord grammar for every shortcut mechanism (no wire change). A key combination is zero or
  more modifiers in angle brackets, then one key, such as `<Super><Alt>F9` or `XF86AudioMute`.
  One parser (`src/control/gnoblin-chord.h`) reads it in the configuration validators, in the
  Lua runtime (`gnoblin.shortcuts.bind`, `gnoblin.input.on`, `keybindings.keyboard`,
  `keybindings.pointer`, `keybindings.wm`, `mutter` and `wayland`), in `configure.shortcuts`, in
  the stylus keybindings and in the compositor. Before, each place checked a chord on its own:
  dynamic binds only checked the length, the compositor said "already claimed" for a mistake, and
  Mutter skipped an unknown `<Foo>` and bound the bare key.
    - A chord in another form, such as `Super+Alt+F9` or `ctrl-q`, is an error that names the
      form to write, for example `Write <Alt><Super>F9`. So are `<Windows>q`, a second key, a
      repeated modifier and `<Mod2>` to `<Mod5>`. Mutter's spellings `<Primary>`, `<Ctrl>`,
      `<Ctl>`, `<Shft>` and `<Mod1>` still work and read as `<Control>`, `<Shift>` and `<Alt>`.
    - Gnoblin gives a chord back with the modifiers in the order
      `<Shift><Control><Alt><Meta><Super><Hyper>`: `Shortcut.accelerator`, the `shortcut.bind`
      reply and the events use that form.
    - A pointer chord needs at least one modifier and ends with `Button1` to `Button3`.
    - An empty stylus keybinding still means none.
    - The four mechanisms stay, because they do different jobs. See
      [Which mechanism to use](docs/guides/shortcuts.md#which-mechanism-to-use).
- A Lua handler that throws now keeps running on later events. Before, Gnoblin disabled a `gnoblin.on`
  listener or an input handler after its first error, and an input handler that threw stopped the
  handlers after it. Now Gnoblin logs the error with the event name and the Lua stack, and every
  other handler of the event still runs. This applies to `gnoblin.on`, the `on` and `watch`
  callbacks of objects, `Process` and `Request` signals, `Operation` callbacks, tween callbacks and
  input handlers.
    - Each handler call has its own instruction budget, so a loop that never ends in one handler no
      longer makes the handlers after it fail.
    - An input handler that throws or returns an invalid value decides nothing, and only its own
      queued operations are dropped.
    - A handler that keeps failing is logged for its first three failures in a row and then once for
      every 1000. The user notice appears once for each run of failures.
    - See [Handler errors](docs/shell-api/errors.md#handler-errors).
- Shortcuts and launches are live objects in Lua; the functions that took IDs or tokens are gone
  (hard cutover; the wire names `shortcut.bind`, `shortcut.unbind`, `shortcut.session.end`,
  `launch.begin` and `launch.end` stay for socket clients, and no wire change was needed).
    - `gnoblin.shortcuts.bind(id, accelerator, options?)` returns a `Shortcut`: interned by ID,
      with `alive`, `on` and the signals `activated`, `deactivated`, `session_activated`,
      `session_key`, `session_ended` and `closed`. A refused binding ends at once and `closed`
      gets the reason.
    - `shortcut:unbind()` replaces `shortcuts.unbind(id)`, and `shortcut:end_session(session_id)`
      replaces `shortcuts.end_session(id, session_id)`: the shortcut owns the session.
    - `shortcuts.list()` now returns the shortcuts this runtime bound, as the same objects. The
      configured shortcut records move to `shortcuts.configured()`; the socket read
      `shortcuts.list` is unchanged.
    - `gnoblin.launches.begin(token, application, options?)` returns a `Launch` with `alive`,
      `on`, `watch` and the signals `changed`, `started`, `timed_out` and `closed`.
      `launch:finish()` replaces `launches.finish(token)`. `launches.list()` returns the same
      objects. A `Launch` has no link to a window or `Process`: the compositor does not record
      which window answered a launch.
    - `gnoblinctl lua` returns `Shortcut` and `Launch` records with the same methods.
- `gnoblin.input.select_source(type, id)` is gone from Lua: `InputSource:select()` replaces it.
  `input.sources()` and `input.current_source()` return `InputSource` objects, one for each type
  and ID, with `alive`, `on` and `watch` and the signals `changed`, `selected` and `closed`. The
  events `gnoblin.input.source-changed` and `gnoblin.input.sources-changed` move them. The wire
  method `input.select` stays; no API bump.
- A `Process` has `alive` and `on("closed", fn)`. The end signal is `closed`, as on Window,
  Workspace and Output, so code can treat every object alike; `status`, `exit_code` and `signal`
  say how it ended, and `wait()` and `kill()` stay.
- `gnoblin.session.suspend()`, `hibernate()`, `reboot()` and `poweroff()` finish with logind's real
  answer (bridge API 1.89). Gnoblin reads the capability (`CanSuspend` and its siblings), then asks
  logind for the action, and the Operation ends with `{accepted = true}` or with an error. See
  [session and runtime health](docs/shell-api/session-runtime.md).
    - `denied`: the capability answer is `no`, or logind or polkit refuses the action.
    - `unsupported`, `busy`, `unavailable`, `timed_out` and `internal` name the other failures.
    - The Lua runtime keeps running after the call. Before, the call stopped the runtime and the
      supervisor asked logind afterwards, so the Operation could not report a refusal.

- The orientation lock and the privacy screen use the new override call (bridge API 1.90). The
  Lua functions `gnoblin.input.set_orientation_lock(value)` and
  `gnoblin.outputs.set_privacy_screen(value)` are gone from Lua and from `gnoblinctl lua`. Use
  `gnoblin.settings.override("input.orientation_lock", value)` and
  `gnoblin.settings.override("monitors.privacy_screen", value)`, and `override:revoke()` in place
  of `"inherit"`. The wire methods `input.set_orientation_lock` and `monitors.set_privacy_screen`
  stay for socket clients, with two changes: a configuration reload that commits ends the value
  that a socket client set (the orientation lock already did), and `monitors.set_privacy_screen`
  fails with `true` or `false` when no output has a privacy screen. A newer value replaces an older
  one, whoever set it.

### Removed

- The external prompter for keyring unlock passwords is removed. The built-in keyring asks for its unlock password
  through the `prompt.secret` slot, like any passphrase request, and fails closed when no shell claims the slot. See
  `docs/shell-api/passphrase-prompts.md`.
- The `prompts` configuration section and its `prompts.enabled` key are removed. A configuration that sets them is
  refused when it loads. `keyring.enabled` still turns the built-in keyring on and off.
- The `when` key of `touchpad_gestures` entries is removed, with its `"unlocked"`, `"locked"` and `"any"` values and the
  legacy `"normal"` and `"unlock-screen"` spellings. A configuration that sets `when` is refused when it loads. Gesture
  bindings run only while the session is unlocked and never reserve a gesture on the lock screen, so a binding meant for
  the lock screen can no longer be written. Gestures there are left to the lock screen client.

### Fixed

- A `config.commit` that removes window rules is answered only after the compositor loop has applied the removal. Before,
  a window the client mapped right after the answer could still be placed under the removed rule, for example maximised
  at the output size, depending on the order the loop drained its queues.

### Added

- `gnoblin.input.grab(options?)` takes the keyboard and returns a live `Grab` (bridge API 1.91). See
  [keyboard grabs](docs/shell-api/input-grab.md). Before, the only keyboard capture was the
  shortcut session of a `modal` or `capture_input` shortcut: one slot, a fixed 10 second timer,
  and key events without text.
    - The grab is a lease. It expires after `timeout` idle seconds (1 to 60, default 10) unless
      `grab:renew()` runs or a key event arrives; each key event starts the timeout again, because
      someone who types still needs the grab.
    - `grab:on("key", fn)` gets `{key, keysym, keycode, modifiers, pressed, repeated, text,
composing, time}`. `text` is the character a press types. Gnoblin feeds each press to the
      compose table of the locale (xkbcommon), so a dead key or a Compose sequence gives one
      composed character on the key that completes it, and `composing` is true inside the sequence;
      a key that breaks the sequence has its own text.
    - `grab:on("closed", fn)` gets the reason: `released`, `expired`, `replaced` (a second grab
      takes the one slot), `locked`, `preempted` (Mutter gave the keyboard to a popup or other
      grab), `config_reloaded`, `runtime_stopped`, or the message of the compositor when it refused.
    - The grab refuses to start while the session is locked and ends when it locks. It never takes
      the keyboard from a shortcut session or `shortcuts.capture`, and they cannot start while it
      holds the keyboard. Its events carry no `focus_context`.
    - The grab refuses to start while any key is down. Mutter gives the release of a held key to
      the grab, so the focused application, which saw the press, would never see the key lift.
      The refusal names the held key. Start a grab from a key release or a later event.
    - Shortcut sessions keep their fixed 10 second timer. It guards a user gesture against a lost
      key release and nothing owns it, so nothing can renew it; a script-renewed lease would let
      a script hold a session open, and a grab that carries a `focus_context` would let a script
      move windows without a gesture.
    - `gnoblin.input.grab` raises an error in `gnoblinctl lua`: a console script has no runtime
      to receive keys.
- `gnoblin.settings.override(path, value)` and `window:set_effect(name, value)` return a live
  `Override` that layers a value over the committed configuration, or an effect over the window
  rules of one window, until it is taken back (bridge API 1.90). See
  [overrides](docs/shell-api/overrides.md).
    - Settings that can change at runtime are `window_management.*`, `compositor.*`,
      `cursor.theme`, `cursor.size`, `layer_shell.preserve_active_window`,
      `monitors.privacy_screen` and `input.orientation_lock`. Any other path raises an error that
      says why it cannot change. The value is checked as the configuration checks it.
    - The effects are those of window rules: `opacity`, `blur`, `blur_ignore_shadows`, `shader`,
      `shader_uniforms`, `frame` and `corners`. The effect goes over the matching rules as one more
      rule and changes only what it sets.
    - `override:revoke()` returns the target to the next older layer, and
      `override:on("closed", fn)` gets the reason: `revoked`, `replaced`, `window_closed`,
      `config_reloaded` or the compositor's message.
    - The latest override of one setting, or of one effect of one window, wins; the older one ends
      with `replaced`. A reload, a Lua worker restart and the close of the window end overrides.
    - `gnoblin.settings` keeps showing the committed values.
- Outputs are live objects (bridge API 1.88). See [outputs](docs/shell-api/outputs.md).
    - Every read of one display returns the same `Output`, with `alive`, `on` and `watch`.
    - An output also has its logical monitor's index, size and work area, and its privacy
      screen state.
    - The privacy screen request covers the whole session, because the compositor keeps one request and applies it
      to every output that has a privacy screen. An `Output` has no method for it.
      `output.privacy_screen` and `output:watch("privacy_screen", fn)` read the state. Bridge API 1.90 replaced
      the first write call, `gnoblin.outputs.set_privacy_screen(value)`, with
      `gnoblin.settings.override("monitors.privacy_screen", value)`. The wire call
      `monitors.set_privacy_screen` is unchanged.
    - New events tell Lua and shell processes when a display is added, changes or is removed.
    - Monitor events and monitor reads stay for shell processes only.
- `gnoblin.idle.watch(seconds)` returns a live `IdleWatch` for any idle time from above 0 up to
  86400 seconds, with `alive`, `idle`, `seconds`, `on("idle" | "active" | "closed")`, `watch` and
  `cancel()`. Each runtime can have up to 256 watches. A reload ends every watch. See
  [idle watches](docs/shell-api/idle.md).
- The `gnoblin.location.authorization-finished` event (bridge API 1.87) reports how each location
  request ended: `allowed`, `denied` or `timeout`. A Lua location `Request` now finishes from it,
  so `finished()` reports the real outcome when a shell process answered the request instead of
  `timeout`.
- Lua timers and Operation methods. `gnoblin.timers.after(seconds)` returns an Operation that
  succeeds after the given time, and `gnoblin.timers.now()` reads a monotonic clock. Every
  Operation now has `map(fn)`, `timeout(seconds)` and `cancel()` beside `on_complete`. Pending
  timers end when the config reloads and do not delay the reload. See
  [operations, errors and timers](docs/shell-api/errors.md).
- `gnoblin.process.spawn(argv, options?)` starts a program and returns a `Process` with its
  `pid` and `status`, `wait()` for its exit and `kill(signal?)`. Options set the working
  directory, environment, standard input, where each output stream goes and how much of it to
  capture, and a timeout. Output is returned as raw bytes. A process keeps running across a
  config reload. See [processes](docs/shell-api/processes.md).
- Window records carry `pid`, the client's process ID from the display server, and `process`,
  the `Process.id` when `gnoblin.process.spawn` started the program. `window:present()` focuses
  such a window without a key press, after the user has not typed or clicked for 250 ms. A
  shell can now open a passphrase prompt that takes typing straight away. See
  [focus and activation](docs/shell-api/focus-activation.md).
- Keyring and GPG prompts, polkit authentication, location authorization and session-lock
  requests reach Lua as one `Request` object, `event.request`. It has `id`, `kind`, `status`,
  the request's detail fields and `finished()`, an Operation that succeeds once with the
  outcome. `answer()`, `cancel()`, `claim()` and `on()` exist only on the kinds that use them.
  See [requests](docs/shell-api/requests.md).
- The Lua configuration receives `gnoblin.session.lock-requested` when it listens for it, so
  a config can start a locker without a separate shell process. See
  [session lock](docs/session-lock.md).
- Windows, workspaces and layer surfaces are live objects in Lua. Every read of one returns
  the same object, updated with each change, with `alive`, `on(signal, fn)` for its
  `changed`, `closed` and other signals, and `watch(field, fn)` for one declared field. Before,
  each read returned a detached copy. See [live window objects](docs/shell-api/windows.md#live-window-objects).
- `window:set_opacity`, `set_scale`, `set_rotation`, `set_offset` and `reset_visuals` change
  how the compositor draws a window without moving its frame. Clicks follow the drawn
  position, a config reload resets them, and they fail while the session is locked. Window
  objects show the values in `opacity`, `scale`, `rotation` and `offset`, and `gnoblinctl lua`
  has the same methods. See
  [change how a window is drawn](docs/shell-api/windows.md#change-how-a-window-is-drawn).
- `gnoblin.tween()` builds a timeline of window opacity, scale, rotation and offset changes,
  intervals and callbacks, and the compositor plays it in its render loop. `play`, `pause`,
  `seek` and `kill` control it; a reload or a session lock cancels it. `gnoblin.ease` and
  `gnoblin.lerp` give the same easing math in Lua. See [tweens and easing](docs/shell-api/tweens.md).
- Packages for Debian testing (`.deb`) and Alpine edge (`.apk`, release assets only), and arm64
  builds of every package. A single build
  now writes the rpm, deb, Arch and tar packages: `scripts/build-prefix.sh` builds Gnoblin
  into a prefix and pkgsmith writes the packages from it with the definitions in `pkgsmith/`.
  See [packaging and releases](docs/distribution.md).
- Signed package repositories for Fedora, openSUSE Tumbleweed, Debian testing and Arch, each with
  a `stable` and a `nightly` channel. A release tag publishes the stable channel.
  A daily build of the main branch publishes the nightly channel when main has
  moved. `scripts/pkgrepo` builds, signs and uploads the repositories, and
  `design/package-repositories.md` describes the layout. See
  [package repositories](docs/package-repositories.md).
- A small notice at the top centre of the screen when a config reload finishes (`Config reloaded`,
  gone after about 2 seconds), when a reload is rejected (the first line of the error, until it is
  clicked or a reload succeeds), and when a shortcut or command does not start
  (`<name> could not start: <reason>`). See
  [reload and error notices](docs/guides/files_and_load_order.md#reload-and-error-notices).

- System drop-ins load on their own. Lua files in `gnoblin/conf.d` under each `XDG_DATA_DIRS` entry
  (such as `/usr/share/gnoblin/conf.d`) and in `/etc/gnoblin/conf.d` run before your `init.lua`, or
  after the built-in defaults when you have no `init.lua`, so a package no longer needs a `gnoblin.load`
  line in your config. A drop-in that fails is skipped and shown in the recovery notice. See
  [system drop-ins](docs/guides/files_and_load_order.md#system-drop-ins).

### Changed

- The RPM specs, the PKGBUILDs, the openSUSE chain and `packaging/` are gone. Package
  names, the Fedora epoch and the `Provides` and `Obsoletes` entries are unchanged, so
  existing installs upgrade. The portal backend is now version `51.0.0` with a release
  number that follows the Gnoblin release. `gnoblin-ananicy-rules` and
  `gnoblin-geoclue-integration` are built for every format and take the Gnoblin version.
- COPR no longer receives releases, because pkgsmith writes binary packages and COPR builds
  source RPMs. The release no longer carries source RPMs, PKGBUILDs or `gnoblin.db`. Use
  the package repositories or the release packages. `scripts/install-system.sh`, which
  installed from COPR, is removed. The `gnoblin-ananicy-v*` releases are replaced by the
  package of the same name in every Gnoblin release.
- `scripts/prepare-release.sh` sets the version in `src/gnoblin-version.json` only.
- The Nix package and module moved from `packaging/nix/` to `nix/`. The flake's outputs are unchanged.
- Packages accept a pre-release version. The source bundle keeps the SemVer
  spelling (`0.1.13-nightly.…`) and pkgsmith spells the package version for the
  package manager (`0.1.13~nightly.…` for RPM). Release versions are unchanged.
- `design/package-targets.json` (formerly `packaging/targets.json`) and the platform support page list Arch, Fedora 45
  and openSUSE Tumbleweed as candidates. Release run 39855522942 passed their
  package build, install, stock GNOME coexistence and removal gates. Graphical
  session support is not claimed.

### Fixed

- `gnoblin.input.grab` was refused for ever, naming a key that was up, after the release of that key was swallowed.
  The compositor refuses a grab while a key is held and tracked the held keys behind the diagnostic console, which
  takes every key while it is open: a key pressed before the console opened and released under it stayed held until
  the user tapped it again. Held keys are now tracked ahead of the console (Mutter patch
  `0132-track-held-keys-before-the-console`), so a key released under the console is up, and a key pressed under it
  and still down afterwards refuses a grab until it lifts. Held keys are kept per keyboard, and a keyboard that is
  removed drops the keys it held, so the removal of a device with a key down no longer refuses grabs. The refusal
  for a key that is really down is unchanged.
- A playing tween held a configuration reload back for 15 seconds, and the reload then failed. A reload now waits
  for every other pending operation but not for a tween. The compositor stops the tween when the new configuration
  takes effect, the `Operation` of `play()` succeeds with the outcome `"cancelled"`, and the `"closed"` signal of
  the `Tween` runs with the reason `"config_reloaded"`. A reload that fails leaves the tween playing.
- `gnoblin.workspaces.create`, `next`, `previous` and `Workspace:activate` and `rename` resolved to a plain record
  with a `windows` key. They resolve to the live `Workspace` that `gnoblin.workspaces.by_id` returns, with
  `window_count`.
- A `gnoblin.configure` call that the validator rejected inside an `Operation` callback failed its `Operation` and
  also logged `deferred Lua operation callback failed`. It fails the `Operation` only.
- `gnoblinctl config reload` accepted `input.keyboard.xkb_options = {"caps:escape"}` in the first
  load but refused it on reload with `input.keyboard.xkb-options: invalid value`, because the
  second normalisation of the input document read only the unnormalised string list. The same code
  is on `main`. A reload now keeps the option.
- The portal backend now destroys and releases each ScreenCast consent dialog
  and its hidden parent window when the session ends. Before, the dialog's
  widget tree stayed alive after every screen cast.
- The Lua API reference now describes `gnoblin.windows.focused`, `windows.by_id`,
  `windows.snap_context`, `gnoblin.outputs.*`, and `gnoblin.capabilities.list`.
  These functions already worked, but editor hints and the generated
  `gnoblin.d.lua` did not list them.
- The Lua API reference no longer lists `Window:match`. The runtime never installed it; the
  window-rule fields stay available from `gnoblinctl window match`.
- A callback added with `Operation:on_complete` after the operation had finished now runs.
  Before, the runtime called the Operation table instead of the callback and logged an error.
- A shell process that called `prompt.respond`, `prompt.cancel`, `auth.*` or
  `location.authorize_app` could answer a request that it never received. The call reached
  the compositor without the client's identity, so the recipient check did not run. The
  compositor now checks it.
- The `monitor.list` bridge method failed with "Lua monitor record has no integer index". The
  reply now lists each monitor again.

### Removed

Gnoblin no longer manages app permissions or persistent portal grants. Bingux
owns permission rules and enforcement.

| Surface      | Removed                                                                                    |
| ------------ | ------------------------------------------------------------------------------------------ |
| Lua          | `gnoblin.permission_rule`, `gnoblin.configure.permissions`, `gnoblin.portals.grants`       |
| `gnoblinctl` | `permissions` and `grant` commands                                                         |
| Events       | `gnoblin.permission.changed`, `gnoblin.portal.grant-added`, `gnoblin.portal.grant-removed` |
| D-Bus        | `CheckPermission`                                                                          |

Gnoblin still routes portal interfaces to the selected backend. The default GTK
backend keeps its normal consent dialogs.

`gnoblin.commands.run` and `gnoblin.commands.capture` are removed; use `gnoblin.process.spawn`. Calling a removed
function is a Lua error, so a config that still calls one fails to load.

| Before                                             | After                                                                    |
| -------------------------------------------------- | ------------------------------------------------------------------------ |
| `gnoblin.commands.run(argv)`                       | `gnoblin.process.spawn(argv)`                                            |
| `gnoblin.commands.capture {argv = argv}`           | `gnoblin.process.spawn(argv, {stdout = "capture"}):wait()`               |
| `capture` option `stdin = text`                    | `stdin = text`, now any bytes up to 1 MiB                                |
| `capture` option `timeout = seconds`               | `timeout = seconds`; the result then has `timed_out = true`              |
| result `exit_code` below 0 after a signal          | result `signal`, such as `"term"`; `exit_code` is `nil`                  |
| failure past 64 KiB of output, or non-UTF-8 output | `output_limit` chooses the size; `stdout_truncated`; output is raw bytes |

`gnoblin.prompt.respond`, `gnoblin.prompt.cancel`, `gnoblin.auth.begin`, `gnoblin.auth.respond`,
`gnoblin.auth.cancel` and `gnoblin.location.authorize_app` are removed from Lua; answer through
`event.request`. Shell processes keep these methods on the compositor bridge.

| Before                                                                         | After                           |
| ------------------------------------------------------------------------------ | ------------------------------- |
| `gnoblin.prompt.respond {request_id = id, password = p}`                       | `event.request:answer(p)`       |
| `gnoblin.prompt.respond {request_id = id, confirmed = true}`                   | `event.request:answer(true)`    |
| `gnoblin.prompt.cancel {request_id = id}`                                      | `event.request:cancel()`        |
| `gnoblin.auth.begin {request_id = id, identity = n}`                           | `event.request:claim(n)`        |
| `gnoblin.auth.respond {request_id = id, response = p}`                         | `request:answer(p)`             |
| `gnoblin.auth.cancel {request_id = id}`                                        | `request:cancel()`              |
| `gnoblin.location.authorize_app {request_id = id, allow = true, accuracy = a}` | `event.request:answer(true, a)` |
| `gnoblin.on("gnoblin.prompt.finished", ...)`                                   | `event.request:finished()`      |

Lua functions that took one table of named fields now take positional arguments, with an
optional `options` table last. The same applies to `gnoblinctl lua`. Socket clients keep
sending named fields.

| Before                                                                | After                                                       |
| --------------------------------------------------------------------- | ----------------------------------------------------------- |
| `workspaces.create {name = n, id = i, activate = a}`                  | `workspaces.create(n, {id = i, activate = a})`              |
| `workspaces.rename {number = 2, name = n}`                            | `workspace:rename(n)` on a `Workspace` object               |
| `workspaces.activate {number = 2}` and `remove`                       | `workspace:activate()` and `workspace:remove()`             |
| `workspaces.move_active {workspace = {number = 2}, follow = f}`       | `window:move_to_workspace(2, {follow = f})`                 |
| `workspaces.move_window {window = w, workspace = s, follow = f}`      | `w:move_to_workspace(s, {follow = f})`                      |
| `animations.inspect {name = n, target = t}` and `preview`             | `animations.inspect(n, {target = t})`                       |
| `animations.seek {session = s, progress = p}`                         | `preview:seek(p)` on an `AnimationPreview`; also `step(ms)` |
| `animations.play {session = s}`, `pause`, `stop`                      | `preview:play()`, `pause()` and `stop()`                    |
| `input.select_source {type = t, id = i}`                              | `input.select_source(t, i)`                                 |
| `launches.begin {token = t, application = a, timeout_ms = m}`         | `launches.begin(t, a, {timeout_ms = m})`                    |
| `shortcuts.bind {id = i, accelerator = k, hold = h}`                  | `shortcuts.bind(i, k, {hold = h})`                          |
| `shortcuts.unbind {id = i}`, `end_session {id = i, session_id = s}`   | `shortcuts.unbind(i)`, `end_session(i, s)`                  |
| `window:move {x = x, y = y}`, `resize`, `thumbnail`                   | `window:move(x, y)`, `resize(w, h)`, `thumbnail(w, h)`      |
| `window:move_to_workspace({number = 2})`, `move_to_monitor({id = m})` | `window:move_to_workspace(2)`, `move_to_monitor(m)`         |

Duplicate Lua names are removed:

| Removed                                      | Use instead            |
| -------------------------------------------- | ---------------------- |
| `gnoblin.monitor.list`                       | `gnoblin.outputs.list` |
| `layer.list`                                 | `layers.list`          |
| `input.list`                                 | `input.sources`        |
| `input.current`                              | `input.current_source` |
| `launches.snapshot`, `gnoblin.launch.status` | `launches.list`        |
| `targetType`                                 | `target_type`          |

Shell processes keep the bridge methods `monitor.list`, `layer.list` and `launch.*`.

A second pass removes every Lua function that had another name for the same action:

| Removed                                                  | Use instead                                                     |
| -------------------------------------------------------- | --------------------------------------------------------------- |
| `gnoblin.events.on`, `gnoblin.events.once`               | `gnoblin.on`, `gnoblin.once`                                    |
| `gnoblin.events.mutter.on`, `gnoblin.events.mutter.once` | `gnoblin.on`, `gnoblin.once` with a `mutter.` event name        |
| `workspace:move_here(w)`                                 | `w:move_to_workspace(workspace)`                                |
| `window:minimize()`, `unminimize()`, `toggle_minimize()` | `window:set_minimized(enabled)`                                 |
| `window:restore()`                                       | `window:set_minimized(false)` and `window:set_maximized(false)` |
| `gnoblin.launches.snapshot()`                            | `gnoblin.launches.list()`; each `Launch` has its own `revision` |
| `gnoblin.monitors.list()`, `gnoblin.monitors.primary()`  | `gnoblin.outputs.list()`, `gnoblin.outputs.primary()`           |
| `gnoblin.monitors.privacy_screen()`                      | `output.privacy_screen`                                         |
| `gnoblin.monitors.set_privacy_screen(value)`             | `gnoblin.settings.override("monitors.privacy_screen", value)`   |
| `gnoblin.outputs.set_privacy_screen(value)`              | `gnoblin.settings.override("monitors.privacy_screen", value)`   |
| `gnoblin.input.set_orientation_lock(value)`              | `gnoblin.settings.override("input.orientation_lock", value)`    |
| `gnoblin.session.activity()`                             | `gnoblin.idle.watch(seconds)`                                   |
| Lua `gnoblin.monitor.*` events                           | `gnoblin.output.added`, `changed`, `removed`                    |

`window:restore_or_minimize()` stays: it restores the frame saved before a snap, which no other
call does. The bridge keeps every wire method name.

### Migration

1. Remove the `permissions` section, `gnoblin.permission_rule` calls, and
   `gnoblin.portals.grants` calls from your Lua config. A `permissions`
   section is ignored and the log names it as an unknown section. A call to a
   removed function is a Lua error and the config fails to load.
2. Write permission rules in the portal backend you use. Bingux reads its own
   rules.
3. Stop subscribing to the removed events and calling the removed commands.
4. Replace `gnoblin.commands.run` and `gnoblin.commands.capture` with
   `gnoblin.process.spawn` as the table above shows.
5. Answer prompts, authentication and location requests through `event.request`, as the
   table above shows.
6. Rewrite table calls to the positional forms above, and replace the removed duplicate
   names.

### Changed

`org.gnoblin.Shell.GlobalShortcutsProvider` `BindShortcuts` and
`ConfigureShortcuts`, and `org.gnoblin.Shell.GlobalShortcutsRebind`
`RebindShortcuts`, take a trailing request handle (`o`), the caller's
`org.freedesktop.portal.Request` object path. The shell uses it, with the
application ID, to identify the requester. Calls with the old argument lists
are not supported. Deploy the portal backend and the shell provider together.
