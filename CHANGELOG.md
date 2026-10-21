# Changelog

## Unreleased

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

1. Remove the `permissions`, `permission_rule`, and `portals.grants` keys from
   your Lua config. A config that still uses them fails at load time with the
   usual unknown-key error.
2. Write permission rules in the portal backend you use. Bingux reads its own
   rules.
3. Stop subscribing to the removed events and calling the removed commands.
