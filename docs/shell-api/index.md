# Lua Runtime API

> **Status:** not available yet: `gnoblin.animation`, `gnoblin.settings`, `gnoblin.settings.override`, `gnoblin.snapshot`, `gnoblin.version`. The sections that use them describe the target, not current behaviour.

Use the Lua runtime API to read compositor state, handle events, and request session actions from a shell integration.

The supervised Lua runtime is available in the standalone Gnoblin session. Its API version is `1.93` in the current source. Callback code runs outside Mutter's compositor thread; keep event callbacks short and nonblocking.

## API domains

- [Windows](/shell-api/windows): live window objects, window actions, menu requests and grants.
- [Find objects](/shell-api/find): select windows, workspaces, outputs and other objects by field or by predicate.
- [Focus and activation](/shell-api/focus-activation): focus policy, history, activation tokens, and trusted contexts.
- [Workspaces](/shell-api/workspaces): state and operations.
- [Outputs](/shell-api/outputs): display outputs, modes, and renderer state.
- [Idle watches](/shell-api/idle): run code when the session has had no input for a set time.
- [Overrides](/shell-api/overrides): layer a setting or a window effect over the configuration, and take it back.
- [Layer surfaces](/shell-api/layer-surfaces): shell surface snapshots and policy.
- [Input](/shell-api/input): devices, sources, and orientation lock.
- [Input handlers](/shell-api/input-handlers): handle local pointer, keyboard, and gesture events.
- [Keyboard grabs](/shell-api/input-grab): take every key for a shell, with composed text and a lease that expires.
- [Grants](/shell-api/grants): the proof of a user gesture that focus, move, resize, snap and text insertion need.
- [Text insertion](/shell-api/text-insertion): insert text into a focused Wayland client.
- [Window snapping](/shell-api/window-snapping): arrange a window from a trusted grant.
- [Tablet-pad help](/shell-api/tablet-pad-help): receive tablet help requests.
- [Keyboard shortcuts](/shell-api/keyboard-shortcuts): actions, capture, and bindings.
- [Launch feedback](/shell-api/launch-feedback): track application launch feedback.
- [Session and runtime health](/shell-api/session-runtime): reload, session, and worker state.
- [Privacy and location](/shell-api/privacy): privacy state and location policy.
- [Authentication prompts](/shell-api/authentication): polkit authentication requests.
- [Keyring passphrase prompts](/shell-api/passphrase-prompts): the keyring's unlock password request.
- [Appearance and animations](/shell-api/appearance-animations): color scheme and animation controls.
- [Shared types](/shell-api/types) and [errors and results](/shell-api/errors).

## Shared conventions

Read-only calls return live objects or detached snapshots. Runtime mutations return an `Operation` handle and complete asynchronously. See [Operations, errors and timers](/shell-api/errors) and [shared runtime types](/shell-api/types).

Subscribe with `gnoblin.on(name, callback)`, or `gnoblin.once(name, callback)` for one event. See the [event reference](/events/).

The Rust event bus is narrower than the legacy reference: `gnoblin.on` and `gnoblin.once` accept only schema-declared object-model signals. The [events reference](/events/) marks broader contracts unbuilt for Rust.

The local socket protocol for non-Lua clients is documented in the [compositor bridge](/bridge/). For command-line use, see [gnoblinctl](/cli/).

## Runtime identity and settings

`gnoblin.settings` is an immutable snapshot of active configuration values. Setting names use public snake_case. It becomes available after the initial configuration commits.

```lua
local blur = gnoblin.settings.compositor.blur
print(blur)
```

`gnoblin.settings.override(path, value)` layers a value over one committed setting without changing this snapshot. See
[overrides](/shell-api/overrides).

`gnoblin.snapshot()` returns a detached copy of the configuration that the files loaded so far have built. It works only while the config loads; at run time it raises an error. Use `gnoblin.settings` to read committed settings.

`gnoblin.config` is not a settings table. It holds `path()`, `default()` and `restore_default()`, and assigning to it raises an error. Declare settings with `gnoblin.configure`.

At run time, the declaration calls return an `Operation` that succeeds when the compositor commits the change. These are `gnoblin.configure`, `gnoblin.window_rule` and `gnoblin.animation`.

`gnoblin.version()` returns an immutable record with string fields `gnoblin`, `gnome`, `mutter`, `lua`, `api`, `git_remote`, `git_sha`, and `build_id`. Missing values are returned as `"unknown"`. User information, query strings, and fragments are removed from remote URLs.

Gnoblin reads `share/gnoblin/version.ini`. Set `GNOBLIN_VERSION_METADATA_FILE` to select another file. If it is unset or unreadable, Gnoblin checks the executable's installation prefix and then the system XDG data directories.
