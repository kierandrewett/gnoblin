# Changelog

## Unreleased

### Added

- Releases publish a pacman repository database, `gnoblin.db`, beside the Arch
  packages. Add `Server = https://github.com/kierandrewett/gnoblin/releases/latest/download`
  to a `[gnoblin]` section in `pacman.conf`. The release refuses to publish
  without it.
- Window rules accept `corners.inner_border_width` (0–40 logical pixels) and
  `corners.inner_border_color` (`#RRGGBB` or `#RRGGBBAA`). They draw a second
  line inward from the window's visible edge, under the existing outline set
  by `border_width` and `border_color`. A negative `border_width` with a
  positive `inner_border_width` gives one line on each side of the edge, as on
  macOS. A width outside 0–40 or a malformed colour makes the configuration
  fail to load.

### Changed

- `packaging/targets.json` and the platform support page list Arch, Fedora 45
  and openSUSE Tumbleweed as candidates. Release run 39855522942 passed their
  package build, install, stock GNOME coexistence and removal gates. Graphical
  session support is not claimed.

### Fixed

- The portal backend now destroys and releases each ScreenCast consent dialog
  and its hidden parent window when the session ends. Before, the dialog's
  widget tree stayed alive after every screen cast.

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

### Migration

1. Remove the `permissions` section, `gnoblin.permission_rule` calls, and
   `gnoblin.portals.grants` calls from your Lua config. A `permissions`
   section is ignored and the log names it as an unknown section. A call to a
   removed function is a Lua error and the config fails to load.
2. Write permission rules in the portal backend you use. Bingux reads its own
   rules.
3. Stop subscribing to the removed events and calling the removed commands.

### Changed

`org.gnoblin.Shell.GlobalShortcutsProvider` `BindShortcuts` and
`ConfigureShortcuts`, and `org.gnoblin.Shell.GlobalShortcutsRebind`
`RebindShortcuts`, take a trailing request handle (`o`), the caller's
`org.freedesktop.portal.Request` object path. The shell uses it, with the
application ID, to identify the requester. Calls with the old argument lists
are not supported. Deploy the portal backend and the shell provider together.
