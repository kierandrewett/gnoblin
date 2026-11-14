# Make pointer feedback easier to see

Use a larger cursor, enable the compositor's locate-pointer action, and show a visual flash when a client requests the
audible bell.

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    cursor: {
        theme: "default",
        size: 32,
    },
    compositor: {
        locatePointer: true,
        audibleBell: false,
        visualBell: true,
        visualBellType: "frame-flash",
    },
});
```

The example uses your system's default cursor theme. Install other themes before selecting them. The options in this
example have these defaults and effects:

| Setting                     | Accepted values and default                                           | Effect                                                 |
| --------------------------- | --------------------------------------------------------------------- | ------------------------------------------------------ |
| `cursor.size`               | Integer `1`–`256`; default `24`                                       | Cursor size in logical pixels.                         |
| `compositor.locatePointer`  | Boolean; default `false`                                              | Enables the compositor's pointer-location key.         |
| `compositor.audibleBell`    | Boolean; default `true`                                               | Plays a sound when a client requests the audible bell. |
| `compositor.visualBell`     | Boolean; default `false`                                              | Shows a visual response to a client bell request.      |
| `compositor.visualBellType` | `"fullscreen-flash"` or `"frame-flash"`; default `"fullscreen-flash"` | Flashes the whole display or the focused frame.        |

The pointer-location key does not enlarge or constantly animate the pointer. The compositor draws nothing for it: it
publishes the pointer position and the shell decides how to show it.

Only the cursor size and the visual bell take effect on a reload. `locatePointer` and `audibleBell` are read when the
session starts. See the [cursor reference](/config/configure/cursor) and
[compositor reference](/config/configure/compositor) for details.

The visual bell only responds to client bell requests. It does not replace application notifications or guarantee that
every application sends a bell. Save the config, then test with a client that supports the bell. See
[cursor setup](/guides/cursors).
