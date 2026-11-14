# Compositor settings

The `compositor` key sets how the compositor draws and rings. Only `visualBell` and `visualBellType` apply on a
reload. The other fields are read when the session starts, so restart the session after changing them. A field you
leave out keeps the default shown below.

| Key                | Values                                          | Default              | Effect                                                         |
| ------------------ | ----------------------------------------------- | -------------------- | -------------------------------------------------------------- |
| `locatePointer`    | Boolean                                         | `false`              | Sends a pointer-locate request when the key is pressed.        |
| `locatePointerKey` | An XKB key name, `"disabled"`, or `""`          | `"Control_L"`        | Key that triggers the pointer locator.                         |
| `visualBell`       | Boolean                                         | `false`              | Flashes the screen when an app requests the visual bell.       |
| `audibleBell`      | Boolean                                         | `true`               | Plays a sound when an app requests the audible bell.           |
| `blur`             | `"auto"`, `"on"`, or `"off"`                    | `"auto"`             | Controls compositor background blur.                           |
| `renderer`         | `"auto"`, `"vulkan"`, `"gles"`, or `"software"` | `"auto"`             | Chooses the renderer that draws frames.                        |
| `debug`            | Boolean                                         | `false`              | Shows a mostly opaque FPS, frame-time, CPU and memory overlay. |
| `visualBellType`   | `"fullscreen-flash"` or `"frame-flash"`         | `"fullscreen-flash"` | Flash the full screen or the focused window.                   |

For example, enable the pointer locator and use a focused-window flash:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    compositor: {
        locatePointer: true,
        locatePointerKey: "F12",
        visualBell: true,
        visualBellType: "frame-flash",
    },
});
```

`blur: "auto"` enables blur when the renderer is hardware accelerated and disables it otherwise. `"on"` always
enables blur, and `"off"` disables it for panels, window rules and shell-requested background effects. The setting is
parsed, and the session does not apply it, so changing it has no effect.

Gnoblin draws nothing for the pointer locator. With `locatePointer: true`, pressing the key publishes the pointer
position and its monitor, and the shell decides how to show it.

- `locatePointerKey` accepts one XKB key name, such as `"Control_L"`, `"Super_R"` or `"F12"`.
- A modifier name without a side, such as `"Control"`, matches both left and right keys.
- `"disabled"` or an empty string removes the trigger.
- The key only has an effect while `locatePointer` is `true`.

## Choosing the renderer

`renderer: "auto"` keeps the renderer the selected backend draws with today: software for the headless and nested
backends, and GLES for the DRM backend. The other values choose one explicitly:

- `"vulkan"` draws with Vulkan. It needs a Vulkan loader and a device, and a software driver such as lavapipe is
  accepted.
- `"gles"` draws with OpenGL ES on a surfaceless context.
- `"software"` draws on the CPU and needs no GPU.

A named renderer is never replaced by another. A session that cannot create it is refused with a message that names
the setting, for example `compositor.renderer = vulkan: no Vulkan loader`. The nested backend draws in software only,
so it refuses `"vulkan"` and `"gles"` the same way.

Guide: [session settings](/guides/session_settings).
