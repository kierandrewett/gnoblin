# Changelog

## Unreleased

### Changed

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

### Added

- Outputs are live objects (bridge API 1.88). See [outputs](docs/shell-api/outputs.md).
    - Every read of one display returns the same `Output`, with `alive`, `on` and `watch`.
    - An output also has its logical monitor's index, size and work area, and its privacy
      screen state.
    - `output:set_privacy_screen(enabled)` sets the privacy screen request. The compositor
      applies one request to every output that has a privacy screen.
    - New events tell Lua and shell processes when a display is added, changes or is removed.
    - Monitor events and monitor reads stay for shell processes only.
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
| `gnoblin.monitors.set_privacy_screen(value)`             | `output:set_privacy_screen(value)`                              |
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
