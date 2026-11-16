# Tweens and easing

`client.set(values, options)` moves a window's properties to new values over time. The compositor plays the animation in
its render loop, so each frame follows the display and the script does not run per frame.

```ts
gnoblin.input.primary.bind("Super+o", () => {
    const client = gnoblin.clients.focused;
    client?.set({ opacity: client.opacity < 1 ? 1 : 0.5 }, { duration: 300 });
});
```

Put the file in `~/.config/gnoblin/init.ts`. `Super+o` fades the focused window to half opacity over 300 ms, and back
on the next press.

## Animate a window

`set(values, options)` takes the values to write and the options of the movement. It is `gnoblin.requestTransition` for one
object, so it has the same options and returns the same `Transition`. See [Animation](/js-transitions).

| Option     | Values                                                 | Default    | Meaning                                                   |
| ---------- | ------------------------------------------------------ | ---------- | --------------------------------------------------------- |
| `duration` | milliseconds, 0 or more                                | 250        | How long the movement takes. 0 writes at once.            |
| `easing`   | `linear`, `ease`, `ease-in`, `ease-out`, `ease-in-out` | `ease-out` | The curve between the start and the end, as CSS names it. |
| `delay`    | milliseconds, 0 or more                                | 0          | How long the compositor holds the start before it begins. |

Without `options` the values are written at once.

The properties the compositor moves over time:

| Property                    | Value               | Meaning                                    |
| --------------------------- | ------------------- | ------------------------------------------ |
| `x`, `y`, `width`, `height` | number              | The window geometry, no shadow, in pixels. |
| `opacity`                   | number, 0 to 1      | How opaque the window is.                  |
| `z`                         | number              | The window's place in depth.               |
| `rotation`                  | `[x, y, z]` numbers | Rotation about each axis, in radians.      |
| `scale`                     | `[x, y, z]` numbers | Scale along each axis.                     |

Any other writable property, such as `visible`, is written at once with the rest. Reading a property back right after the
call gives the end value; the compositor draws the way there. A property that is not writable, a value of the wrong kind or
a `duration` below 0 throws a `TypeError`, and nothing is written.

The curves are the CSS timing functions of the same names:

- `linear` moves at a constant speed.
- `ease` starts quickly, then slows down. It is the CSS default.
- `ease-in` starts slowly and speeds up.
- `ease-out` starts fast and slows down towards the end. It is the default here.
- `ease-in-out` speeds up in the first half and slows down in the second.

```ts
const client = gnoblin.clients.focused;
client?.set({ x: 0, y: 0, rotation: [0, 0, 0.1] }, { duration: 400, easing: "ease-out" });
```

The compositor draws a window's own open, close and minimise animations itself, and a value you set while one plays can be
overwritten when it ends.

A newer write of a property takes it from the animation that was moving it, and that animation ends `cancelled`.

To animate with a transition object that you can pause, reverse or cancel, see [Animation](/js-transitions).

## Workspace slides

`gnoblin.show(workspace, { duration })` slides the windows of the two workspaces over `duration` milliseconds. The stock keys
`Super+1` to `Super+9` slide over 250 ms. See [Workspaces](workspaces#show-a-workspace).

## Reduced motion

`gnoblin.motion(duration)` returns `duration` milliseconds, or 0 while the seat's `reducedMotion` setting is on. The stock animations call
it, so a workspace slide, `client.set` with a duration and the titlebar's close fade take no time for a user who asks for
reduced motion. Pass your own durations through it:

```ts
const duration = gnoblin.motion(300);
```

`client.set` and `gnoblin.show` apply it themselves, so call `motion` only for a duration you use elsewhere, such as
`gnoblin.timers.after`.

## Run something after an animation

`set` returns a `Transition`. Its `finished` promise resolves when the animation ends:

```ts
const client = gnoblin.clients.focused;
client?.set({ opacity: 0 }, { duration: 200 }).finished.then(() => client.close());
```

`gnoblin.timers.after(ms, fn)` runs at the first frame after the time. See [Operations, errors and timers](errors#timers).

## Turn animations off

Reduced motion makes every one of these instant: a workspace switch lands at once and a window is hidden at once. A
config can read the setting, as `gnoblin.input.primary.settings.reducedMotion`, but `init.ts` cannot set it. See [dropped settings](/js-settings#dropped-settings).

## Animate a window yourself

`client.set(values, options)` moves properties to `values` over `options.duration` milliseconds along a named curve.

- The properties that move over time are `x`, `y`, `width`, `height`, `opacity`, `z`, `rotation` and `scale`.
- The curves are the CSS names `linear`, `ease`, `ease-in`, `ease-out` (the default) and `ease-in-out`.
- A duration of `0`, or no options, writes at once.
- `gnoblin.motion(duration)` returns `0` while reduced motion is on, so a script that passes its duration through it
  respects the setting.

This fades each Text Editor window in when it opens:

```ts
import { CollectionListener } from "gnoblin";
import type { Client } from "gnoblin";

gnoblin.clients.addListener(
    new (class extends CollectionListener {
        onAdded(client: Client) {
            if (client.appId === "org.gnome.TextEditor") {
                client.opacity = 0;
                client.set({ opacity: 1 }, { duration: 300, easing: "ease-out" });
            }
        }
    })(),
);
```

Replace the app id with the value `gnoblinctl window list` prints.
