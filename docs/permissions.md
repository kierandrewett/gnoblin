# Portal permissions

[Configuration reference](configuration-reference.md)

Apps use the desktop portal to ask for screen sharing, remote keyboard/mouse
control and other access. These rules decide whether Gnoblin asks you,
approves the request or rejects it. Without rules, the usual permission
dialogs apply.

These rules require Gnoblin's [patched portal backend](source-development.md#optional-components).
They have no effect on a stock portal backend. Changes apply to new requests;
an existing screen share stays connected.

## Choose a policy

| Level     | Result                                           |
| --------- | ------------------------------------------------ |
| `default` | Normal portal behavior, including restore tokens |
| `ask`     | Require consent or selection every time          |
| `allow`   | Approve the rule's specified capabilities        |
| `deny`    | Reject without a dialog                          |

The global default accepts `default`, `ask` or `deny`.
Automatic approval needs an explicit app rule. To ask every time by default:

```lua
gnoblin.configure {
    permissions = {default = "ask"},
}
```

Add this to `~/.config/gnoblin/init.lua`. Individual rules can override it.

A matching deny always wins. Otherwise the last matching rule wins as a whole;
device and monitor lists do not merge across rules.

## Example: allow a remote-desktop app

After your component includes, append a rule:

```lua
gnoblin.permission_rule {
    name = "rustdesk",
    match = [[^host-exe:/usr/bin/rustdesk$]],
    capabilities = {"screen-cast", "remote-desktop"},
    level = "allow",
    monitors = {"primary"},
    devices = {"keyboard", "pointer"},
    clipboard = true,
}
```

Use the identity for your installation. This grants portal access; it does not
configure RustDesk's separate `uinput` path.

## Match an identity

`match` is a case-sensitive JavaScript regex against one namespaced identity:

- `app-id:<id>`: supplied by the trusted portal frontend.
- `host-exe:<absolute-path>`: caller executable when the frontend has no app ID.

Use anchors for exact matches. Window titles and client Wayland app IDs are not
permission identities. Do not grant a broad temporary directory for an AppImage.

If the portal cannot verify the caller's identity, Gnoblin will not approve it
automatically. It rejects the request when the global default is `deny`;
otherwise it asks you.

These settings express user preferences. They do not isolate hostile processes
running as the same Unix user.

## Capabilities

| Capability       | Automatic approval covers                                 |
| ---------------- | --------------------------------------------------------- |
| `screen-cast`    | Listed monitors                                           |
| `remote-desktop` | Listed input devices, monitors and clipboard access       |
| `input-capture`  | Supported requested input-capture capabilities            |
| `screenshot`     | Consent; interactive selection still happens              |
| `access`         | Generic access consent; dialogs with choices still appear |

`monitors` uses `"primary"` or exact connectors such as `"DP-1"`.
Unattended capture needs an explicit selection. Missing monitors or incompatible
source types fail; the backend does not guess another source.

`devices` accepts `keyboard`, `pointer` and `touchscreen`; default is empty.
`clipboard` defaults to `false`. Requests exceeding these limits fail.

A remote-desktop request can include both video and input control. Video still
obeys the `screen-cast` policy: `deny` blocks it and `ask` requires consent.
Automatic video access also needs a monitor selected in an `allow` rule.

## Inspect a decision

```sh
gnoblinctl config reload
gnoblinctl permissions list
gnoblinctl permissions check screen-cast host-exe:/usr/bin/rustdesk
```

For the RustDesk rule above:

```sh
gnoblinctl permissions check screen-cast host-exe:/usr/bin/rustdesk --json
```

```json
{ "level": "allow", "rule": "rustdesk", "monitors": ["primary"], "devices": 3, "clipboard": true }
```

The response uses a device bitmask: keyboard `1`, pointer `2`, touchscreen `4`.
Here `3` means keyboard and pointer. Lua uses the readable device-name list.

`check` explains the policy for a supplied identity. It does not authenticate
a running process or check that monitors exist.

Invalid edits retain the last valid policy. At startup, an invalid policy or
unavailable policy service denies backend requests.

## What this does not control

File chooser, USB, background requests, global shortcuts, direct Mutter calls,
raw Wayland protocols and kernel devices are outside this policy.
Frontend-cached permissions may bypass the backend.

`access` cannot distinguish camera, microphone and location requests.
[Protocol settings](session-settings.md#protocol-settings) are separate.

## Older grants

Old `portal-grants` files no longer grant access. Inspect or remove them with
`gnoblinctl grant list` and `gnoblinctl grant revoke`.
There is no automatic conversion to regex rules.

Developer checks: `node tests/permissions.test.mjs` and
`tests/test-permissions-live.py` in a private session with the patched backend.
