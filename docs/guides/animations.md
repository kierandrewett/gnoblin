# Animations

The TypeScript shell has no declarations for named animations, keyframes or per-window animation rules. This page says
what moves today and how to move a window yourself.

## What animates

The shell and compositor animate these on their own. The compositor's part (the fades below) is the desktop's look, which the std
library sets: a session that runs a shell with no std library (a core-only shell) has none of it, and the shell animates its own windows.

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

Reduced motion makes every one of these instant: a workspace switch lands at once and a window is hidden at once. A
config can read the setting, as `gnoblin.input.primary.settings.reducedMotion`, but `shell.ts` cannot set it. See [dropped settings](/js-settings#dropped-settings).

## Animate a window yourself

`gnoblin.requestTransition(work, options)` runs `work` once and moves what it changed to the new values over
`options.duration` milliseconds along a named curve. See [animation in TypeScript](/js-transitions) for what the callback
may do.

- The properties of a window that move over time are `x`, `y`, `z` and `opacity`. The others land at once.
- The curves are the CSS names `linear`, `ease`, `ease-in`, `ease-out` (the default) and `ease-in-out`.
- A duration of `0` writes at once.
- `gnoblin.motion(duration)` returns `0` while reduced motion is on, so a script that passes its duration through it
  respects the setting.

This fades each Text Editor window in when it opens:

```ts
import { ClientCollectionListener } from "gnoblin-core";
import type { Client } from "gnoblin-core";

gnoblin.clients.addListener(
    new (class extends ClientCollectionListener {
        onAdded(client: Client) {
            if (client.appId === "org.gnome.TextEditor") {
                client.opacity = 0;
                gnoblin.requestTransition(
                    () => {
                        client.opacity = 1;
                    },
                    { duration: gnoblin.motion(300), easing: "ease-out" },
                );
            }
        }
    })(),
);
```

Replace the app id with the value `gnoblinctl window list` prints.
