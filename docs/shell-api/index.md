# Lua Runtime API

Use the Lua runtime API to read compositor state, handle events, and request session actions from a shell integration.

The supervised Lua runtime is available in the standalone Gnoblin session. Its API version is `1.81` in the current source. Callback code runs outside Mutter's compositor thread; keep event callbacks short and nonblocking.

## API domains

- [Windows](/shell-api/windows): snapshots, window actions, menu and focus contexts.
- [Focus and activation](/shell-api/focus-activation): focus policy, history, activation tokens, and trusted contexts.
- [Workspaces and monitors](/shell-api/workspaces-monitors): state, operations, and privacy screens.
- [Outputs](/shell-api/outputs): display outputs, modes, and renderer state.
- [Layer surfaces](/shell-api/layer-surfaces): shell surface snapshots and policy.
- [Input](/shell-api/input): devices, sources, and orientation lock.
- [Input handlers](/shell-api/input-handlers): handle local pointer, keyboard, and gesture events.
- [Text insertion](/shell-api/text-insertion): insert text into a focused Wayland client.
- [Window snapping](/shell-api/window-snapping): arrange a window from a trusted focus context.
- [Tablet-pad help](/shell-api/tablet-pad-help): receive tablet help requests.
- [Keyboard shortcuts](/shell-api/keyboard-shortcuts): actions, capture, and bindings.
- [Launch feedback](/shell-api/launch-feedback): track application launch feedback.
- [Session and runtime health](/shell-api/session-runtime): reload, session, and worker state.
- [Privacy and location](/shell-api/privacy): privacy state and location policy.
- [Authentication prompts](/shell-api/authentication): polkit authentication requests.
- [Keyring and GPG prompts](/shell-api/passphrase-prompts): secret prompts.
- [Appearance and animations](/shell-api/appearance-animations): color scheme and animation controls.
- [Shared types](/shell-api/types) and [errors and results](/shell-api/errors).
- [Console](/cli/lua): inspect the session with `gnoblinctl lua`.

## Shared conventions

Read-only calls return detached snapshots. Runtime mutations return an `Operation` handle and complete asynchronously. See [Errors and results](/shell-api/errors) and [shared runtime types](/shell-api/types).

Subscribe with `gnoblin.events.on(name, callback)` or the `gnoblin.on` alias. See the [event reference](/events/).

The local socket protocol for non-Lua clients is documented in the [compositor bridge](/bridge/). For command-line use, see [gnoblinctl](/cli/).

## Runtime identity and settings

`gnoblin.settings` is an immutable snapshot of active configuration values. Setting names use public snake_case, and `revision` identifies the committed snapshot. It becomes available after the initial configuration commits.

```lua
local focus_mode = gnoblin.settings.window_management.focus_mode
print(focus_mode, gnoblin.settings.revision)
```

`gnoblin.snapshot()` returns a detached copy of the Lua configuration view. Use `gnoblin.settings` for immutable reads of committed settings.

`gnoblin.version()` returns an immutable record with string fields `gnoblin`, `gnome`, `mutter`, `lua`, `api`, `git_remote`, `git_sha`, and `build_id`. Missing values are returned as `"unknown"`. User information, query strings, and fragments are removed from remote URLs.

Gnoblin reads `share/gnoblin/version.ini`. Set `GNOBLIN_VERSION_METADATA_FILE` to select another file. If it is unset or unreadable, Gnoblin checks the executable's installation prefix and then the system XDG data directories.
