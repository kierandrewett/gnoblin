# Changelog

## Unreleased

### Added

- `gnoblin.commands.capture` accepts `prompt = request_id`. The command becomes
  the window of that waiting passphrase prompt. Its window takes keyboard
  focus under `focus_new_windows = "prevent"`, and Gnoblin stops the command
  when the prompt ends.

### Fixed

- The prompt broker now reports `password-strength` for a new password: 0 for
  an empty password and greater than 0 otherwise, as Gcr defines it. gnome-keyring treated every new
  password as blank and asked "Store passwords unencrypted?" in a loop.

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
