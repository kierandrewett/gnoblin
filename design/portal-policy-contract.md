# Move portal permissions out of Gnoblin

Status: proposal for review. Nothing here is implemented.

## Decision

Gnoblin cannot enforce permissions on a portal backend it does not control. The
`xdg-desktop-portal` frontend calls the chosen backend directly, and the
compositor never sees the request.

So Gnoblin stops owning permissions. The portal provider owns them. For Bingux
that means Bingux stores the rules, shows the settings and enforces them in its
own portal backend.

## What Gnoblin keeps

- Per-interface portal routing (`gnoblin.configure.portals`), so any provider
  can be chosen.
- Capture and input operations through Mutter's D-Bus interfaces.
- The default GTK portal backend, with its normal consent dialogs. It stops
  consulting any policy.

## What Gnoblin removes

From the inventory of files that mention permissions:

| Area            | Items                                                                                                                                                                                                                                                            |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Lua API         | `gnoblin.permission_rule`, `gnoblin.configure.permissions`, `gnoblin.portals.grants`                                                                                                                                                                             |
| Compositor      | `org.gnoblin.Compositor.CheckPermission`, native policy (`src/control`, `src/config`, Mutter patch 0019)                                                                                                                                                         |
| Backend         | `gnoblin-portal-policy.c`, `gnoblin-portal-identity.c`, permission calls in screenshot, screencast, remote desktop, input capture and access                                                                                                                     |
| Patches         | `xdg-desktop-portal-gnome/20-permissions`, `45-portal-grant-management`, `10-tooling/0002` (grants)                                                                                                                                                              |
| Control and CLI | permission and grant requests, `gnoblinctl` permission commands, the `gnoblin.permission.changed` event                                                                                                                                                          |
| Docs            | `docs/config/permission_rule.md`, `docs/config/configure/permissions.md`, `docs/guides/permissions.md`, `docs/shell-api/permissions-privacy.md`, `docs/bridge/permissions-and-privacy.md`, and the references in `docs/dbus`, `docs/events`, `design/lua-api.md` |
| Tests           | `tests/test-permissions-live.py`, `tests/fixtures/rustdesk-permissions.lua`, permission cases in the Lua and CLI tests and the QEMU scripts                                                                                                                      |

This is a breaking change to a documented public API. It follows the repo's
clean-cutover rule: no shims, no compatibility aliases. The API version and the
changelog must record it, and a config that still sets `permission_rule` must
fail with a clear message.

## What Bingux adds

- A permission store and rules, with a settings page.
- A portal backend that runs the identity check, applies the rules and
  enforces the result, starting with Screenshot (Bingux #10).
- Its own identity derivation (frontend owner, request handle, user check,
  `app-id:` or `host-exe:`), copied from the Gnoblin backend before removal.
- A route in `packaging/gnoblin/bingux.lua` so Bingux's backend comes first.

## Takeover for other shells

A shell ships a backend whose `.portal` file lists only the interfaces it
implements, and routes those interfaces to it. Permissions are that shell's
decision. Gnoblin offers no policy to share.

## Cost

- A stock Gnoblin session with the default backend has no rules. Remote desktop
  and screen sharing apps such as RustDesk get the normal consent dialogs each
  time.
- Nothing enforces a rule across providers, which was already true.

## Phases

1. Bingux: copy identity logic, build the permission store, rules and backend
   (#10). This must land first, so the policy is not lost.
2. Bingux: audit every portal interface in a Bingux guest and record the owner
   and the result.
3. Gnoblin: remove the permission surface listed above, including docs, API
   version and tests. Check the build, the Lua and CLI tests and the QEMU
   portal scripts.

## Decisions

1. Persistent grants (`gnoblin.portals.grants`, `gnoblin.portal.grant-added`,
   `gnoblin.portal.grant-removed`) are removed with permissions.
2. The Gnoblin backend keeps only the dialogs it shows today. It consults no
   policy.
3. Mutter patch 0019 and the monolithic patch need a patch audit during the
   removal, as in `design/patch-audit-2026-10-20.md`.
4. The Bingux work lands on `feat/bingux-portal` in the Bingux repo and merges
   into `feat/bingux-os`. The Gnoblin removal lands on
   `feat/remove-portal-permissions` and merges into `main` after the Bingux
   branch.
5. The Bingux backend is a Rust program, `xdg-desktop-portal-bingux`, in the
   Bingux repo.
