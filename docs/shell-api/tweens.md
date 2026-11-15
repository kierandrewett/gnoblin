# Tweens and easing

`client.animate(values, options)` moves a window's properties to new values over time. The compositor plays the animation in
its render loop, so each frame follows the display and the script does not run per frame.

```ts
gnoblin.input.primary.bind("Super+o", () => {
    const client = gnoblin.clients.focused;
    client?.animate({ opacity: client.opacity < 1 ? 1 : 0.5 }, { duration: 300 });
});
```

Put the file in `~/.config/gnoblin/init.ts`. `Super+o` fades the focused window to half opacity over 300 ms, and back
on the next press.

## Animate a window

`animate(values, options)` takes the values to move to and the options of the movement.

| Option     | Values                                                 | Default    | Meaning                                                   |
| ---------- | ------------------------------------------------------ | ---------- | --------------------------------------------------------- |
| `duration` | milliseconds, 0 or more                                | none       | How long the movement takes. 0 writes at once. Required.  |
| `easing`   | `linear`, `ease`, `ease-in`, `ease-out`, `ease-in-out` | `ease-out` | The curve between the start and the end, as CSS names it. |

The properties you can animate:

| Property                    | Value               | Meaning                                    |
| --------------------------- | ------------------- | ------------------------------------------ |
| `x`, `y`, `width`, `height` | number              | The window geometry, no shadow, in pixels. |
| `opacity`                   | number, 0 to 1      | How opaque the window is.                  |
| `z`                         | number              | The window's place in depth.               |
| `rotation`                  | `[x, y, z]` numbers | Rotation about each axis, in radians.      |
| `scale`                     | `[x, y, z]` numbers | Scale along each axis.                     |

Reading a property back right after the call gives the end value; the compositor draws the way there. A property that is
not in the table throws a `TypeError`, as does a value of the wrong kind or a `duration` below 0.

The curves are the CSS timing functions of the same names, the ones `requestTransition` takes:

- `linear` moves at a constant speed.
- `ease` starts quickly, then slows down. It is the CSS default.
- `ease-in` starts slowly and speeds up.
- `ease-out` starts fast and slows down towards the end. It is the default here.
- `ease-in-out` speeds up in the first half and slows down in the second.

```ts
const client = gnoblin.clients.focused;
client?.animate({ x: 0, y: 0, rotation: [0, 0, 0.1] }, { duration: 400, easing: "ease-out" });
```

The compositor draws a window's own open, close and minimise animations itself, and a value you set while one plays can be
overwritten when it ends.

To animate with a transition object that you can pause, reverse or cancel, see [Animation](/js-transitions).

## Workspace slides

`gnoblin.show(workspace, { duration })` slides the windows of the two workspaces over `duration` milliseconds. The stock keys
`Super+1` to `Super+9` slide over 250 ms. See [Workspaces](workspaces#show-a-workspace).

## Reduced motion

`gnoblin.motion(duration)` returns `duration` milliseconds, or 0 while the seat's `reducedMotion` setting is on. The stock animations call
it, so a workspace slide, `Client.animate` and the titlebar's close fade take no time for a user who asks for reduced
motion. Pass your own durations through it:

```ts
const duration = gnoblin.motion(300);
```

`client.animate` and `gnoblin.show` apply it themselves, so call `motion` only for a duration you use elsewhere, such as
`gnoblin.timers.after`.

## Run something after an animation

An animation does not call back when it ends. Start a timer for its length:

```ts
const client = gnoblin.clients.focused;
client?.animate({ opacity: 0 }, { duration: 200 });
gnoblin.timers.after(200, () => client?.close());
```

`gnoblin.timers.after(ms, fn)` runs at the first frame after the time. See [Operations, errors and timers](errors#timers).
