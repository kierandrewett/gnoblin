# Tweens and easing

`gnoblin.requestTransition(work, options)` moves a window's properties to new values over time. The callback `work` writes
the new values, and the compositor plays the animation in its render loop, so each frame follows the display and the
script does not run per frame.

```ts
gnoblin.input.primary.bind("Super+o", () => {
    const client = gnoblin.input.primary.focusedClient;
    if (client !== undefined) {
        gnoblin.requestTransition(
            () => {
                client.opacity = client.opacity < 1 ? 1 : 0.5;
            },
            { duration: 300 },
        );
    }
});
```

Put the file in `~/.config/gnoblin/shell.ts`. `Super+o` fades the focused window to half opacity over 300 ms, and back
on the next press.

## Animate a window

`gnoblin.requestTransition(work, options)` runs `work` once, now. Every write it makes to a window is staged, and the
options say how the window moves to the staged values. It returns a `Transition`. See [Animation](/js-transitions).

| Option     | Values                                                 | Default    | Meaning                                                   |
| ---------- | ------------------------------------------------------ | ---------- | --------------------------------------------------------- |
| `duration` | milliseconds, 0 or more                                | 250        | How long the movement takes. 0 writes at once.            |
| `easing`   | `linear`, `ease`, `ease-in`, `ease-out`, `ease-in-out` | `ease-out` | The curve between the start and the end, as CSS names it. |
| `delay`    | milliseconds, 0 or more                                | 0          | How long the compositor holds the start before it begins. |

Without `options` the movement takes 250 ms along `ease-out`.

The properties the compositor moves over time:

| Property                    | Value               | Meaning                                    |
| --------------------------- | ------------------- | ------------------------------------------ |
| `x`, `y`, `width`, `height` | number              | The window geometry, no shadow, in pixels. |
| `opacity`                   | number, 0 to 1      | How opaque the window is.                  |
| `z`                         | number              | The window's place in depth.               |
| `rotation`                  | `[x, y, z]` numbers | Rotation about each axis, in radians.      |
| `scale`                     | `[x, y, z]` numbers | Scale along each axis.                     |

Any other writable property, such as `visible`, is written at once with the rest. Reading a property back right after the
call gives the end value; the compositor draws the way there. A write the tree refuses or a `duration` below 0 throws, and
nothing is written.

What the callback may write:

- A property of a `Client` (`client.opacity = 0.5`, `client.x = 70`, `client.rotation = [0, 0.5, 0]`) is staged with the other writes, and
  reading it inside the callback gives the staged value. A timed write of a window (`client.setOpacityTimed(0.5, timing)`) is not
  staged: it plays by the timing it is given.
- A property of a workspace, a layout or the styles is staged by the library and tweens through the windows it moves.
- Staging a window's property needs a core that has the write stage (`set_write_stage`, see [ABI compatibility](/abi-compatibility)).
  On a core built before it the writes of a callback to a window's properties stay plain: they are immediate, untimed and not rolled back.

To move one window or workspace, call its `set(values, options)`. It is `requestTransition` for that one object:

- Every key of `values` must be a writable property, or the call throws a `TypeError` and writes nothing.
- It takes the same options and returns the same `Transition`.
- Without options it writes the values at once.

```ts
gnoblin.input.primary.focusedClient?.set({ x: 0, y: 0 }, { duration: 400, easing: "ease-out" });
```

The curves are the CSS timing functions of the same names:

- `linear` moves at a constant speed.
- `ease` starts quickly, then slows down. It is the CSS default.
- `ease-in` starts slowly and speeds up.
- `ease-out` starts fast and slows down towards the end. It is the default here.
- `ease-in-out` speeds up in the first half and slows down in the second.

```ts
const client = gnoblin.input.primary.focusedClient;
if (client !== undefined) {
    gnoblin.requestTransition(
        () => {
            client.x = 0;
            client.y = 0;
            client.rotation = [0, 0, 0.1];
        },
        { duration: 400, easing: "ease-out" },
    );
}
```

The compositor draws a window's own open, close and minimise animations itself, and a value you set while one plays can be
overwritten when it ends.

A newer write of a property takes it from the animation that was moving it, and that animation ends `cancelled`.

To animate with a transition object that you can pause, reverse or cancel, see [Animation](/js-transitions).

## Workspace slides

`gnoblin.workspaces.show(workspace, { duration })` slides the windows of the two workspaces over `duration` milliseconds. The stock keys
`Super+1` to `Super+9` slide over 250 ms. See [Workspaces](workspaces#show-a-workspace).

## Reduced motion

`gnoblin.motion(duration)` returns `duration` milliseconds, or 0 while the seat's `reducedMotion` setting is on. The stock animations call
it, so a workspace slide and the titlebar's close fade take no time for a user who asks for reduced motion. Pass your own
durations through it:

```ts
const duration = gnoblin.motion(300);
```

`gnoblin.workspaces.show` applies it itself, so call `motion` for the duration you give `requestTransition` and for one you
use elsewhere, such as `gnoblin.timers.after`.

## Run something after an animation

`requestTransition` returns a `Transition`. Its `finished` event runs when the animation ends, and tells whether it was
cancelled:

```ts
const client = gnoblin.input.primary.focusedClient;
if (client !== undefined) {
    const fade = gnoblin.requestTransition(
        () => {
            client.opacity = 0;
        },
        { duration: 200 },
    );
    fade.on("finished", (cancelled) => {
        if (!cancelled) {
            client.close();
        }
    });
}
```

`gnoblin.timers.after(ms, fn)` runs at the first frame after the time. See [Operations, errors and timers](errors#timers).

## Turn animations off

Reduced motion makes every one of these instant: a workspace switch lands at once and a window is hidden at once. A
config can read the setting, as `gnoblin.input.primary.settings.reducedMotion`, but `shell.ts` cannot set it. See [dropped settings](/js-settings#dropped-settings).

## Animate a window yourself

[Animate a window yourself](/guides/animations#animate-a-window-yourself) has a complete example that fades each window
in when it opens.
