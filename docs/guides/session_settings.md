# Session settings

[Configuration reference](/config/configure)

Configure how layer-shell surfaces affect focus, how windows respond to pointer input, and which Wayland interfaces
are available. Add the examples to `~/.config/gnoblin/init.ts`.

## Layer-shell keyboard focus

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    layerShell: { preserveActiveWindow: true },
});
```

Default: `true`. The setting is accepted and checked, and nothing reads it; the compositor logs `not applied: ...` for it. See the [layer-shell reference](/config/configure/layer_shell).

When a launcher requests exclusive keyboard focus, typing goes to the launcher. With this setting enabled, the
application underneath keeps its active appearance: its titlebar and effects do not switch to their unfocused style.
The launcher still receives all typing.

Set `false` to let exclusive layers deactivate the application beneath them. On-demand focus, passive bars and normal
application menus are unchanged.

## Window drag boundary

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    windowManagement: { constrainDragToWorkArea: true },
});
```

Default: `true`. The compositor accepts this setting but does not read it, so `true` and `false` behave the same.
To change how far a dragged window may leave the work area, use the `dragConstraint` key of `defineConfig`: see the
[window management reference](/config/configure/window_management#the-shells-own-keys).

## Protocol settings

Wayland protocols let desktop tools request features such as screen capture, clipboard access and display power
control. The interfaces below are enabled by default. Most users can leave them alone.

For example, this disables the `wlr-screencopy` screen-capture interface:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    protocols: { wlrScreencopy: false },
});
```

Changes apply when the compositor next starts. Disabling an interface stops new clients from binding to it; clients
that already bound it keep their connection until they disconnect.

Disabling `extSessionLock` while the session is locked makes the reload fail and keeps the previous configuration.
Unlock before changing that setting.

| Name                           | Used for                           |
| ------------------------------ | ---------------------------------- |
| `wlrLayerShell`                | Bars, docks and launchers          |
| `wlrScreencopy`                | Screen capture                     |
| `extForeignToplevelList`       | Window enumeration                 |
| `wlrForeignToplevelManagement` | Window controls and dock targets   |
| `extDataControl`               | Clipboard managers                 |
| `extIdleNotify`                | Idle detection                     |
| `wlrGammaControl`              | Gamma and colour temperature       |
| `wlrOutputPowerManagement`     | Display power                      |
| `extBackgroundEffectV1`        | Client-requested background blur   |
| `xdgDecoration`                | Client/server titlebar negotiation |

Each protocol name accepts `true` or `false` and defaults to `true`. Set a protocol to `false` to keep Gnoblin from
advertising that interface to new clients.

This does not block all screen sharing. Portal backends decide what apps may do; see
[portals](/config/configure/portals). Disabling protocols your shell uses can prevent its features from working. See
the [protocol catalog](/wayland-protocols) for the advertised interface names and related guides, and the
[protocols reference](/config/configure/protocols) for the rest of the names.
