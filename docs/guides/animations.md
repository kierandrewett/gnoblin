# Animations

The TypeScript shell has no declarations for named animations, keyframes or per-window animation rules, and the
`animations` section of the configuration is read by nothing. This page says what moves today and how to move a window
yourself.

## What animates

The shell and compositor animate these on their own:

- **Workspace switch.** The windows of the workspace being left slide off one side and the new workspace's windows slide
  in from the same side. Each window moves by one output width over 200 ms with an ease-out cubic curve, and the side
  follows the workspace order. A second switch during a slide starts from where the windows are drawn. A three-finger
  swipe on the touchpad drags the slide with the fingers.
- **Hide and close.** Hiding a window, as minimising does, fades the window's last picture out over 200 ms. Restoring it
  during that fade eases it back to opaque. A window that closes fades out the same way, from the last picture its
  client committed.
- **Titlebar.** The close fade of the titlebar is skipped with reduced motion.

A client that reuses a GPU buffer for its next frame while a fade runs can show through the fade.

## Turn animations off

Reduced motion makes every one of these instant: a workspace switch lands at once and a window is hidden at once.

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    appearance: { reducedMotion: true },
});
```

This also asks applications to stop animating. See [appearance settings](/config/configure/appearance).

## Animate a window yourself

`client.animate(values, options)` moves properties to `values` over `options.duration` seconds along a named curve.

- The properties are `x`, `y`, `width`, `height`, `opacity` and `z`.
- The curves are `linear`, `out` (the default), `out-cubic`, `in-cubic` and `in-out`.
- A duration of `0` writes at once.
- `gnoblin.motion(seconds)` returns `0` while reduced motion is on, so a script that passes its duration through it
  respects the setting.

This fades each Text Editor window in when it opens:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    on: {
        clientAdded(client) {
            if (client.appId === "org.gnome.TextEditor") {
                client.opacity = 0;
                client.animate({ opacity: 1 }, { duration: 0.3, easing: "out-cubic" });
            }
        },
    },
});
```

Replace the app id with the value `gnoblinctl window list` prints.
