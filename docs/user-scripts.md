# Scripts inside Gnoblin

Gnoblin scripts are GJS modules loaded inside the GNOME Shell process. The
script host imports each entry file, calls its default-exported function, and
keeps the module active until reload or session shutdown. A script can use
GNOME Shell and Mutter objects directly, so it has the same access as the
compositor process. Only install code you trust.

Use a script when code must run inside the compositor: for example, to observe
a Shell event or add a namespaced operation to the compositor bridge. Use
[gnoblinctl](gnoblinctl.md) for command-line actions, the
[compositor bridge](compositor-bridge.md) for an external shell, and Lua for
configuration. A script is not needed just to draw a panel or launcher.

The script host loads top-level `*.js` entries from these locations, in order
from lowest to highest precedence:

1. `gnoblin/scripts/` under each directory in `$XDG_DATA_DIRS`, usually
   `/usr/share/gnoblin/scripts/`. Packages use this for compositor-side
   integrations that they own.
2. `$XDG_DATA_HOME/gnoblin/scripts/`, normally
   `~/.local/share/gnoblin/scripts/`. User-installed integrations can live
   here.
3. `$XDG_CONFIG_HOME/gnoblin/scripts/`, normally
   `~/.config/gnoblin/scripts/`. Put personal scripts here.

If two locations contain the same filename, the higher-precedence file is
loaded. Helper modules can live in subdirectories and be imported relative to
the entry file; only top-level JavaScript files are loaded as scripts. The
resulting entry files load in filename order. This is independent of the root
Lua file selected by `GNOBLIN_CONFIG`.

Each entry file must default-export a function that receives the script API:

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

The entry function runs once per load. `gnoblinctl reload` unloads the module
state before importing it again, so register cleanup for every signal, timer,
socket and other resource that must not survive a reload.

## Script API

| Call                                           | Result                                                                      |
| ---------------------------------------------- | --------------------------------------------------------------------------- |
| `api.log(...values)`                           | Write a log message prefixed with the script name                           |
| `api.version()`                                | Return Gnoblin Shell's version string                                       |
| `api.getFeature(id)`                           | Read a Shell feature switch                                                 |
| `api.setFeature(id, enabled)`                  | Change a Shell feature switch                                               |
| `api.reloadShell()`                            | Soft-reload config, theme and scripts                                       |
| `api.on(event, callback)`                      | Subscribe to an event; return a function that unsubscribes                  |
| `api.addCleanup(callback)`                     | Register cleanup for reload or unload; return a function that runs it early |
| `api.handleCompositorOperation(name, handler)` | Register a namespaced bridge operation; return a cleanup function           |
| `api.onCompositorClientClosed(callback)`       | Observe bridge clients disconnecting; return a cleanup function             |

Find feature IDs with `gnoblinctl feature list`. Supported event names are:

| Event               | Callback argument                  |
| ------------------- | ---------------------------------- |
| `window-opened`     | A GNOME Shell `Meta.Window` object |
| `workspace-changed` | Zero-based active workspace index  |
| `overview.showing`  | None                               |
| `overview.shown`    | None                               |
| `overview.hiding`   | None                               |
| `overview.hidden`   | None                               |

The `Meta.Window` argument is an in-process GNOME object, not the bridge's JSON
window record. A script that calls GNOME internals may need adjustment when the
underlying GNOME version changes. Use the bridge when an external shell needs a
stable, serializable window record.

Overview events follow GNOME Shell's overview transitions. Lua listeners can
subscribe to the same transitions as `gnome.shell.overview.*`; see the
[Lua event reference](/config/lua-events).

Bridge operation names must match
`^[a-z][a-z0-9-]*\.[a-z][a-z0-9-]*$`, such as `example.inspect`, and be unique.
The handler receives `(request, peer)`. `peer.client` is an opaque token for
that connection; it is valid only while the client is connected. `peer.pid` is
the client's process ID, and `peer.send(record)` sends one event to that client.

| Peer member                 | Contract                                                                        |
| --------------------------- | ------------------------------------------------------------------------------- |
| `client`                    | Opaque per-connection token; use it to address this client from other handlers. |
| `pid`                       | Unix process ID reported for the bridge connection.                             |
| `isOpen()`                  | `true` while the connection is open.                                            |
| `send(record)`              | Send one event to this connection.                                              |
| `sendTo(token, record)`     | Send to another open connection; returns `true` if sent, otherwise `false`.     |
| `broadcast(record)`         | Send to all connected bridge clients.                                           |
| `broadcast(record, tokens)` | Send only to connections identified by their `peer.client` tokens.              |

`api.onCompositorClientClosed(callback)` calls `callback(token)` when a client
disconnects. Use it to discard per-client state; the disconnected token cannot
be used for a later send. The token is created when that connection first calls
a script-registered operation; clients that disconnect without doing so are
reported with `undefined`. Registering a handler returns a cleanup function,
and script unload removes it automatically.

A handler can use GJS directly. Validate each request, and keep shell-specific
policy in the package or script that owns it.

For example, a package can register a bridge operation without adding that
feature's name or behavior to Gnoblin's built-in bridge:

```javascript
export default function (api) {
    api.handleCompositorOperation("example.inspect", (request, peer) => {
        peer.send({ event: "inspection", value: String(request.value ?? "") });
    });
}
```

The core bridge owns the socket and general compositor operations. A package
can install its script in an XDG script directory and register its own
namespaced operations. For example, Bingux ships its text-input integration
with Bingux; Gnoblin does not include that shell-specific workflow.

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

Do not use a script to replace a compositor service. In current source
builds the bridge starts as a core service. A file named `compositor-bridge.js` in the user script directory is ignored.

## Reload and recovery

`gnoblinctl script list` reports successfully loaded scripts from all three
locations. A failed import or callback is logged with its filename.
`gnoblinctl reload`
reloads scripts without closing application windows. If the previous session
ended uncleanly while scripts were active, Gnoblin pauses them for recovery;
run `gnoblinctl reload` to retry after examining the log. The built-in bridge
remains separate from script recovery.
