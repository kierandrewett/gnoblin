# Changelog

## Unreleased

### Added

- `gnoblin --hosted --wayland-display NAME` runs a headless session for an
  external session manager. It opens no viewer, takes no seat and starts with no
  monitors; the manager adds virtual monitors through ScreenCast
  `RecordVirtual`. Standalone, `--greeter`, `--devkit` and `ext-session-lock-v1`
  behaviour is unchanged. See [hosted sessions](docs/hosted.md).
- `org.gnoblin.Hosted1` on the session bus reports when the compositor serves
  session clients (`Ready`) and carries logout, restart, power off, suspend and
  hibernate requests as `ActionRequested`. In a hosted session
  `gnoblinctl session` and the Lua session operations report those requests
  instead of ending the session or calling logind.

### Fixed

- A hosted session waits up to 60 seconds, instead of 10, for each step of a slow
  start, so a loaded machine no longer ends up on built-in defaults with no login
  autostart. `GNOBLIN_HOSTED_STARTUP_WAIT_MS` changes the wait.
