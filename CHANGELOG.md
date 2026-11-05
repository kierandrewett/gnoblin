# Changelog

## Unreleased

### Added

- `Window.client_type` (`"wayland"` or `"x11"`, bridge API 1.93) in window records, `gnoblin.find("Window", {client_type = ...})`,
  the window rule `match.client_type` key and `gnoblinctl window list` and `window match`.

### Changed

- Xwayland windows get Gnoblin's own frame, rules and effects like Wayland windows. Mutter no longer launches
  `mutter-x11-frames` or reparents X11 clients into frame windows. An X11 window that does not turn decorations off is
  framed under `frame.mode = "auto"` (the default for X11 when no rule sets a mode), `_GTK_FRAME_EXTENTS` feeds the
  visible bounds, `_NET_FRAME_EXTENTS` and `_NET_REQUEST_FRAME_EXTENTS` report Gnoblin's border, and Mutter's stock
  shadow no longer draws for managed X11 windows. A border change keeps the client where it is.
  `mutter-x11-frames` is no longer built or installed. A request for frame extents before the window exists is answered
  from the rules that match only the window type and `client_type`. The Mutter code for a frame window (`MetaFrame`,
  its sync counter and borders, the `_MUTTER_FRAME_FOR`, `_MUTTER_FRAME_EXTENTS` and `_MUTTER_NEEDS_FRAME` properties,
  and the frame mask in the X11 window actor) is removed in patch `11-window-frames/0009`.

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
- A package test suite runs after the packages are built and before anything is released
  (`tests/packages`, `scripts/run-package-tests.sh`, [design/package-testing.md](design/package-testing.md)). Each
  target on each architecture, experimental and rolling ones included, gets four layers: static checks of the files
  (metadata, paths, modes, links, build paths, checksums, ABI), install, upgrade, reinstall and removal in a clean
  image of the release, a headless session of the installed compositor (Wayland globals, compositor bridge,
  `gnoblinctl`, Lua configuration, a screencopy capture, clean shutdown), and a signed repository tree. Each job
  uploads JUnit XML, a summary and the evidence to repeat the run. It replaces `scripts/check-package-install.sh`.
- The release runs one leg for each target and architecture of the generated target list
  (`.github/workflows/target.yml`): prefix build, packages, package suite, graphical VM test, verdict. A leg waits
  for no other leg; the targets that use the shared prefixes or the shared packages wait for the shared stage only.
  The release gate (`tests/packages/gate.py`) publishes only the packages whose every target and architecture passed
  every stage, the graphical VM test on that architecture included. Each target is released on its own: a failed
  target is left out and named in the release notes ("left out: <name> <arch>, <stage>") and holds no other back. A
  failed built target or an empty publish set still makes the run fail. A stage after a failed one is reported as "not run", never as a
  pass, and each planned release gets a verdict with its blocker. See
  [design/release-pipeline.md](design/release-pipeline.md).
- The graphical VM test runs on arm64 too, in an aarch64 guest (machine `virt`, UEFI, `virtio-gpu-pci`) on TCG,
  because the arm64 runners have no KVM. The guest of each architecture is data in `scripts/targets.json`. An arm64
  package is published only after its own arm64 graphical test passed; until the first one passes, no arm64 package
  is published. Arch Linux publishes no aarch64 cloud image, so the job builds one from the signed Arch Linux ARM
  archive (`scripts/build-arch-arm-image.sh`, pinned key `68B3537F39A313B3E574D06777193F152BDBE6A6`). The chroot
  stages of that build and the aarch64 guest have never run on an aarch64 host; the first CI run is their first proof. The harness
  takes `--deadline`, so a slow guest ends with a result before the job timeout, and prints the time of each check.
- One GitHub issue for each release that fails a stage or has no build and is not at its end of life
  (`tests/pipeline/issues.py`, label `release-pipeline`). It names the architecture, stage, check and detail, the
  run and what unblocks it, gets a comment only when the failures change, and is closed when the release passes
  again. An end-of-life release gets a notice only. Release tags and the nightly run change issues; other runs
  print what they would do. It replaces the `graphical-test` issues of `tests/graphical/report.py`, and the
  graphical workflow no longer runs on a schedule: the nightly run tests every target.
- Packages for Debian testing (`.deb`) and Alpine edge (`.apk`, release assets only), and arm64
  builds of every package. A single build
  now writes the rpm, deb, Arch and tar packages: `scripts/build-prefix.sh` builds Gnoblin
  into a prefix and pkgsmith writes the packages from it with the definitions in `pkgsmith/`.
  See [packaging and releases](docs/distribution.md).
- Fedora packages for every Fedora release from Fedora 41 that is out, also after its end of life. Each
  release is built on its own image, so its rpm asks for the glibc and libraries of that release. The
  signed repository has one tree per release. See [platform support](docs/platform-support.md).
- `scripts/targets.json` is the one list of what Gnoblin lists and builds. `scripts/supported-targets.py`
  reads it with the release data of endoflife.date and starts the jobs. A family lists its newest three
  releases and every release of the last three years (Ubuntu: long-term support releases only). End of
  life is shown and never removes a release. Each release is `built` (blocking CI job), `experimental`
  (job that cannot fail the run, for a release that no full run has verified yet) or `planned` (no
  job, with the reason). `--table` shows all of it. It replaces `scripts/package-targets.json` and its
  checker. Every family has the same data (package kind, image, build requirements script, package lane), and
  the workflows name no release, image or family: one matrix of release and architecture starts a prefix
  build, a package set and an install check for each target, on the image of that target. Debian and Ubuntu
  releases make debs that are install checks only. Their revision carries the release (`1~deb13`) and no
  repository takes them yet.
- CI jobs for Fedora 41 and 42, Debian 13, Ubuntu 24.04 and 26.04 LTS, Alpine 3.21 to 3.24 and openSUSE Leap 16.0.
  Each builds a prefix on its own image, writes its own packages and install-checks them. Each Alpine branch makes
  its own apks, because an apk binds the library sonames of the branch that built it. Leap 16.0 makes its own rpm
  with the new `opensuse` lane (`scripts/build-packages.sh --lanes opensuse --opensuse-release leap16.0`), named
  `1.leap16.0`. These packages, except the Fedora rpm, are install checks only. `scripts/install-opensuse-build-deps.sh`
  installs the openSUSE build packages. In `scripts/targets.json`, `verified_from` and `verified_through` give the
  releases that a full run has verified, and `planned_reason` gives the reason of each planned release, such as the
  libraries below the build floors.
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

- Fedora 41 to 45, Ubuntu 24.04 and 26.04 LTS, Debian 13, Alpine 3.22 to 3.24 and openSUSE Leap 16.0 are `built`
  CI targets, not experimental: a full release run passed every stage on them on amd64 and arm64. A failure there now
  fails the release run. Alpine 3.21 stays experimental: its Mesa 24.2.8 offers no
  `EGL_EXT_image_dma_buf_import_modifiers` to the software renderer, so the package suite finds no
  `zwp_linux_dmabuf_v1` global in a container.
- The source builds against the libraries of Debian 13 and Ubuntu 26.04 LTS, not only Fedora 43 and newer. The floors
  were then GLib 2.84, gsettings-desktop-schemas 48.0, libwayland 1.23, wayland-protocols 1.44, libinput 1.28, PipeWire
  1.4.2, IBus 1.5.32 and, for the portal backend, GTK 4.18. Glycin is optional. With these older libraries pens report
  no eraser button, no libinput Lua plugins load, clients get no `wl_fixes` global and repeat keys themselves, the IBus
  input method shows no preedit hints, the newer input settings come from the Lua configuration only, and without
  Glycin the portal backend decodes wallpaper previews with gdk-pixbuf in its own process. Builds against newer
  libraries keep every feature. `scripts/install-debian-build-deps.sh` installs the Debian and Ubuntu build packages,
  and a CI job compiles the compositor and the portal backend on the oldest release of each family. See
  [build from source](docs/install-source.md). Before this change a build against IBus 1.5.32 left out the IBus input
  method without a message.
- The source also builds against the libraries of Fedora 41 and Alpine 3.21. The floors are now GLib 2.82,
  gsettings-desktop-schemas 47.1, wayland-protocols 1.38, libinput 1.27, PipeWire 1.2.7, libei 1.3.0, IBus 1.5.31 and,
  for the portal backend, GTK 4.16, libadwaita 1.6 and xdg-desktop-portal 1.18.4. With libadwaita 1.6 the screen
  sharing dialog shows the older view switcher. With xdg-desktop-portal 1.18 sandboxed apps cannot ask for USB devices.
  Below libei 1.5 Gnoblin does not wait for queued input before it answers a sync request from a remote desktop
  client. Builds against newer libraries keep every feature.
- The source also builds against the libraries of Ubuntu 24.04 LTS, which is an experimental CI target. The floors are
  now GLib 2.80, gsettings-desktop-schemas 46.1, libwayland 1.22, libdisplay-info 0.1.1, libinput 1.25, PipeWire 1.0.5,
  libei 1.2.1, IBus 1.5.29 and, for the portal backend, GTK 4.14 and libadwaita 1.5. Ubuntu 24.04 loses the most:
  tablet pad dials do nothing, the pen pressure range is not applied, tap-and-drag lock ends after a timeout, screen
  sharing has no explicit sync (torn or partly drawn frames with a driver that needs it), each app connection has a 4
  KB message buffer instead of 1 MB, the switch-monitor and keybinding stylus actions come from the Lua configuration
  only, and apps get no accent colour from the Settings portal. The colour and HDR data of a display come from a copy
  of the libdisplay-info 0.2 code. Nobody has started a session on Ubuntu 24.04. Builds against newer libraries keep
  every feature. `scripts/install-debian-build-deps.sh` installs Meson 1.5.2 from PyPI when the release has an older
  Meson. The deb packages ask for GLib 2.80, gsettings-desktop-schemas 46.1 and GTK 4.14, and for `libdisplay-info1`
  when the prefix links `libdisplay-info.so.1`. See [platform support](docs/platform-support.md#older-system-libraries).
- The build dependency scripts for Fedora, Alpine and openSUSE name no versions. The floors come from the pinned Meson
  files, which `scripts/check-build-deps.py` checks before the build. Glycin is installed only where the release has
  it. Before this change the Fedora script failed on every release before Fedora 43.
- `gnoblin-geoclue-integration` requires GeoClue 2.7.0, the first version that reads the drop-in folder
  `/etc/geoclue/conf.d`. It required 2.7.2, which stopped the install on openSUSE Leap 16.0 (GeoClue 2.7.1).
- `scripts/check-package-install.sh` checks what the Gnoblin packages require, not what else is installed: `gnoblin`
  requires no GTK and no libadwaita, and no Gnoblin package requires GTK 3 or gnome-desktop. On Fedora 42 the
  distribution's own `gtk4` pulls GTK 3.
- The libinput floor of the packages comes from the libinput symbol versions in the programs, as the glibc floor does.
  `scripts/build-packages.sh --lanes deb --deb-release <name>` writes debs from a prefix built on a Debian or Ubuntu
  release, for install checks.
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
- The platform support page lists Arch, Fedora 45 and openSUSE Tumbleweed as candidates. Release run
  39855522942 passed their package build, install, stock GNOME coexistence and removal gates. Graphical
  session support is not claimed.
- The `gnoblin` package asks for no GTK 4 and no libadwaita in any format. The compositor draws the frames of X11
  apps itself, so Mutter's GTK frames client (`mutter-x11-frames`) is gone and there is no `gnoblin-x11-frames`
  package. Only the optional portal backend, `xdg-desktop-portal-gnoblin`, needs GTK 4 and libadwaita. The package
  suite checks this in every format: `gnoblin` asks for no GTK (`S3`), and installing it alone brings none (`I2`).
  See [optional packages](docs/optional-packages.md#gtk-4).

### Fixed

- The packaged programs no longer hold paths of the build machine: `scripts/build-prefix.sh` maps the checkout to `.`
  (`-ffile-prefix-map`), and the system layout no longer builds the build-tree fallback path of the version file into
  `gnoblinctl`, which an installed `gnoblinctl` would have read.
- Every deb carries `/usr/share/doc/<package>/copyright` in the machine-readable Debian format, with the licence of
  the package (`pkgsmith/copyright/`).
- `gnoblin` depends on PyGObject in every format, so `/usr/lib/gnoblin/bin/gdctl` runs on a clean install.
- The graphical VM test reads the checksum file of an Alpine cloud image, which holds the digest only.
- The graphical VM test starts its session on Fedora 45, where `kmsconvt@tty1` (kmscon) holds tty1 instead of
  `getty@tty1`. The session unit now stops kmscon too, so the hang-up of tty1 no longer ends the session.
- The graphical VM test logs in to Alpine guests. Alpine's OpenSSH has no PAM and refuses a key login to a locked
  account, so the seed now gives the test user and root the password field `*` (no password, not locked) instead of
  `!`. Before, Alpine 3.21 to 3.24 and edge never got past the `boot` check.
- The graphical VM test starts the Gnoblin session on Alpine. Busybox login has no PAM, so no logind session was
  opened and Gnoblin refused to start. The guest now logs in with util-linux login and `pam_elogind`, runs eudev so
  that mutter finds the GPU, and starts a D-Bus session bus.
- An emulated graphical guest waits 10 minutes for its disks (systemd credentials, without changing the image), also
  on Fedora 41 and 42, where SELinux kept the drop-in out of the real root. A guest that goes to emergency mode or
  freezes fails `boot` at once, with a picture of its screen, and is booted once more from a fresh disk. This also
  covers a guest whose serial console shows only sulogin or the audit record of the emergency service, and a guest
  with a soft lockup of 300 s or more (the longest that recovered in CI lasted 106 s).
- The graphical VM test fails `session` at once when Gnoblin logs that its Lua runtime is lost for the session,
  instead of after the 600 s session timeout, and says why gnoblinctl cannot reach the session. A failed `session`
  keeps a picture of the screen and where each compositor thread is (`compositor-threads.txt`, with an `eu-stack`
  backtrace), and its excerpt now holds Gnoblin's own log lines.
- When a graphical check after the session fails (`globals` to `shutdown`), its detail says whether the compositor
  still answers `gnoblinctl status`, and the first such failure keeps the screen and `compositor-threads.txt`. A failed
  `shutdown` also gives the exit status and message of `gnoblinctl logout`.
- The package suite runs an apk command again, at most 3 times in all, when a fetch from the repository fails in
  transit (TLS, DNS, a closed or reset connection, an HTTP timeout or server error), and prints the error each time.
  Any other apk failure, and a fetch error that stays, still fails the case.
- The graphical VM test starts arm64 guests. Their network device asked for an option ROM (`efi-virtio.rom`) that
  QEMU on an Ubuntu arm64 runner does not have, so QEMU stopped at start. The device has no ROM now, and a QEMU that
  stops at start is reported with its own message.
- The runtime layer of the package suite passes the background colour to `swaybg` as `#ff00ff`. swaybg 1.2.0
  (Ubuntu 24.04) rejects `ff00ff`, so the output stayed black and the R6 check failed.
- `scripts/build-prefix.sh` fetches the upstream tags of the Mutter and portal checkouts only when the pinned tag is
  missing, so a checkout that has the tag builds without access to gitlab.gnome.org.
- `xdg-desktop-portal-gnoblin` no longer asks for libsecret, which it does not link or use.
- The touchpad disable-while-typing timeout now takes effect with libinput 1.31 or newer. The build had left the setting
  out because the generated `config.h` did not carry its define.
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
