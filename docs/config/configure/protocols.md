# Protocol settings

The `protocols` key enables or disables a Wayland protocol that Gnoblin exposes to clients. Disabling a protocol stops
new clients from discovering it; clients already bound to it keep their existing object until they disconnect. The
change applies when the compositor next starts.

| Setting            | Values  | Default                       | Effect                                                                  |
| ------------------ | ------- | ----------------------------- | ----------------------------------------------------------------------- |
| `protocols.<NAME>` | Boolean | `true` in the Gnoblin session | Exposes the named protocol (`true`) or hides it from clients (`false`). |

Write the protocol name in camelCase, as the `NAME` key. `wlr-screencopy` is `wlrScreencopy`. The names are:

- **Shell surfaces:** `wlrLayerShell`.
- **Window management:** `extForeignToplevelList`, `wlrForeignToplevelManagement`, `xdgDecoration`,
  `kdeServerDecoration`.
- **Capture and effects:** `wlrScreencopy`, `extImageCopyCapture`, `extBackgroundEffectV1`.
- **Session controls:** `extDataControl`, `extIdleNotify`, `wlrGammaControl`, `wlrOutputPowerManagement`,
  `wlrOutputManagement`, `extSessionLock`.

For example, disable screen capture protocol advertisement:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    protocols: { wlrScreencopy: false },
});
```

See the [protocol catalog](/wayland-protocols), and the [defineConfig reference](/typescript-config#protocols) for
what each protocol lets a client do.

A protocol you leave out stays on. A value that is not a boolean is refused: the reload fails and the previous
configuration stays in force.
