# User scripts inside Gnoblin

User scripts are small GJS modules loaded inside Gnoblin's Shell process. Use
them for reactions that need Shell objects or events. External applications
should normally use [gnoblinctl](gnoblinctl.md), the
[compositor bridge](compositor-bridge.md) or a [Wayland protocol](wayland-protocols.md).

Place `*.js` files in `$XDG_CONFIG_HOME/gnoblin/scripts/` (normally
`~/.config/gnoblin/scripts/`). This directory is independent of the root Lua
file selected by `GNOBLIN_CONFIG`. Files load in filename order. Each module
must default-export a function that receives the script API:

```javascript
export default function (api) {
    api.on("workspace-changed", (index) => {
        api.log(`Active workspace index: ${index}`);
    });
}
```

Save that as `workspace-log.js`, then run `gnoblinctl reload`. Switch
workspaces and inspect the Shell log for `gnoblin-script[workspace-log.js]`.
The callback index comes from GNOME Shell's workspace manager and is
**zero-based**. `gnoblinctl workspace` uses **one-based** positions.

## Script API

| Call | Result |
| --- | --- |
| `api.log(...values)` | Write a log message prefixed with the script name |
| `api.version()` | Return Gnoblin Shell's version string |
| `api.getFeature(id)` | Read a Shell feature switch |
| `api.setFeature(id, enabled)` | Change a Shell feature switch |
| `api.reloadShell()` | Soft-reload config, theme and user scripts |
| `api.on(event, callback)` | Subscribe to an event; return a function that unsubscribes |
| `api.addCleanup(callback)` | Register cleanup for reload or unload; return a function that runs it early |

Find feature IDs with `gnoblinctl feature list`. Supported event names are:

| Event | Callback argument |
| --- | --- |
| `window-opened` | A GNOME Shell `Meta.Window` object |
| `workspace-changed` | Zero-based active workspace index |

The `Meta.Window` argument is an in-process GNOME object, not the bridge's JSON
window record. A script that calls GNOME internals may need adjustment when the
underlying GNOME version changes. Use the bridge when an external shell needs a
stable, serializable window record.

## Clean up resources

`api.on` subscriptions are removed on reload. Register any other long-lived
resource with `api.addCleanup`. For example, a GJS timer:

```javascript
import GLib from "gi://GLib";

export default function (api) {
    const timer = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 60, () => {
        api.log(`Gnoblin ${api.version()} is running`);
        return GLib.SOURCE_CONTINUE;
    });
    api.addCleanup(() => GLib.source_remove(timer));
}
```

Do not use a user script to replace a compositor service. In current source
builds the bridge starts as a core service. A file named
`compositor-bridge.js` in the user script directory is ignored. An older
installed build may still have used that file; check the running version when
migrating.

## Reload and recovery

`gnoblinctl script list` reports successfully loaded **user** scripts. A
failed import or callback is logged with its filename. `gnoblinctl reload`
reloads scripts without closing application windows. If the previous session
ended uncleanly while scripts were active, Gnoblin pauses them for recovery;
run `gnoblinctl reload` to retry after examining the log. The built-in bridge
remains separate from that user-script recovery path.
