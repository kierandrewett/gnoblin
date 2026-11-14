# Animation in TypeScript

`gnoblin.requestTransition(callback, options)` draws a change of state as one animation. The callback runs once, now, and
stages the changes; the compositor's tween player moves what is drawn from where it is to where the callback put it.

```ts
import { gnoblin, MasterStackLayout } from "gnoblin";

const workspace = gnoblin.workspaces.get("dev")!;
const client = gnoblin.clients.get("42")!;

gnoblin.requestTransition(
    () => {
        client.opacity = 0.85;
        client.x = 120;
        workspace.layout = new MasterStackLayout({ ratio: 0.7 });
    },
    { duration: 250, easing: "ease-out" },
);
```

An ordinary assignment, `client.opacity = 0.85`, stays immediate. A transition is for changes that belong together.

## What the callback does

- It runs once and synchronously, in the call. It is not called again on a frame.
- Every write to a window's properties is staged. Reading a property inside the callback gives the staged value.
- A layout change inside it (`workspace.layout = other`, `layout.ratio = 0.7`, `workspace.gaps = { inner: 8 }`) is
  computed once, against the staged state, and each window that moves tweens from where it is drawn to its new place. A
  window is paired by its id. A window with no place to start from (no size yet, or hidden) is placed at once.
- A style change (`gnoblin.clients.style.opacity = 0.6`) becomes a write of the window's `opacity`, so it tweens too.
- All of it travels in the shell's one transaction of the frame, as timed writes: a duration and an easing curve of 33
  samples for each property.
- The state is committed when the transition begins. Reading `client.x` after the call gives the end value; the
  compositor draws the way there.

The callback has three limits, and each is an error that changes nothing:

- It may not be `async` or return a promise.
- It may not call `requestTransition` again.
- It may not call anything with a side effect. `client.raise()`, `lower()`, `focus()` and `close()`, `gnoblin.spawn()`,
  `Process.kill()` and `gnoblin.session.*()` throw inside a callback, because a failed transition is rolled back and a
  spawned process cannot be.

## Options

| Option     | Meaning                                                                                                                                                             |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `duration` | Milliseconds, 0 or more. Default 250. 0 commits the change at once, still as one transaction. `Client.animate` and `gnoblin.after` take seconds.                    |
| `easing`   | `linear`, `out`, `out-cubic`, `in-cubic` and `in-out` (the names `Client.animate` takes), and the CSS names `ease-out`, `ease-in-out` and `ease-in`. Default `out`. |

`ease-out` is `out`, `ease-in-out` is `in-out` and `ease-in` is `in-cubic`. Any other option is a `TypeError` that names the
known ones. `autoplay: false` and `delay` are refused too, and the error says why (see "Not built").

## The `Transition`

`requestTransition` returns a `Transition`.

| Member           | Meaning                                                                                                         |
| ---------------- | --------------------------------------------------------------------------------------------------------------- |
| `running`        | True until it ends or is cancelled.                                                                             |
| `cancelled`      | True once `cancel()` or a newer transition took every property it animated.                                     |
| `duration`       | The milliseconds it takes: the duration asked for, or 0 under reduced motion.                                   |
| `finished`       | A promise that resolves once with `{ cancelled }` and never rejects.                                            |
| `onFinished(fn)` | Calls `fn({ cancelled })` once as a queued call, like any listener. Returns a subscription to stop it.          |
| `cancel()`       | Writes the end values of what it still animates at once, which ends the tracks. Resolves `{ cancelled: true }`. |

Cancelling puts what is drawn on the end state, which the transition committed when it began. It does not go back. The end
is the shell's clock reaching the duration, so a transition ends when the shell's timers say so.

## One object: `set`

`client.set(props, options)` and `workspace.set(props, options)` are one transition over one object. A key that is not a
property with a setter is a `TypeError` before anything is written. Without `options` the change is atomic and immediate.

```ts
client.set({ opacity: 0.92, rotation: [0, 0.12, 0] }, { duration: 250, easing: "ease-out" });
workspace.set({ layout: new MasterStackLayout({ ratio: 0.7 }) }, { duration: 300 });
```

## When transitions overlap

Two transitions can write one property of one node.

- The one that began second owns the property, and the compositor ends the other track on it.
- The earlier transition goes on with the properties it still owns, and ends at its own time.
- One that has lost every property ends at once, with `{ cancelled: true }`.
- A plain assignment to a property takes it from a transition the same way.

## When it fails

A write the tree would refuse, a destroyed client and a layout that throws are errors in the call. The error names the
property, as in `Client#3.opacity: must be a finite number` and `Client#4.opacity: the client is gone`. Nothing is sent.

What is put back:

- each workspace's layout, gaps and name
- the own fields of the layout the workspace had when the transition began
- every style

What is not: a layout object the callback assigned and changed, because it was not the workspace's layout when the
transition began, and an array edited in place.

## Reduced motion

While the seat asks for reduced motion (`gnoblin.motion(1) === 0`) no write is timed: the change lands as one transaction of
plain writes and `finished` still resolves, with `{ cancelled: false }`.

## What tweens

The tween player animates the properties its scene can interpolate. The shell sends a timed write for each property the
tree takes a curve for; what the compositor can draw is smaller.

| Property                                             | On the compositor                 |
| ---------------------------------------------------- | --------------------------------- |
| `x`, `y`, `z`, `opacity`                             | Tweens from the drawn value.      |
| `width`, `height`, `rotation`, `scale`               | Lands at once (no track to play). |
| `visible`, `tiled`, effects (shadows, backdrop blur) | Written at once, as any write is. |

A style `cornerRadius`, `borders` and `background` are not written to the tree for a window at all
(`client.styleWarnings`).

## Not built

These need work in the compositor and its tween player first. The shell does not fake them.

- **Scrubbing** (`autoplay: false`, `progress`, `reverse()`, `play()`, `pause()`, `finish()`). `TweenPlayer::advance` takes
  the elapsed milliseconds and derives every track's progress from its own elapsed time (`gnoblin-render/src/tween.rs`).
  There is no way to set a track's progress, to hold it, or to run it backwards. It needs `TweenPlayer::seek`, a pause and a
  direction, a tree write that names a transition and carries its progress, and hit testing against the presented
  transforms.
- **`delay`.** A timed write carries a duration and samples. `TrackSpec` has a `delay_ms`; the tree op has no field for it.
- **Tweening a size, a rotation or a scale.** `visual_property` maps only `opacity`, `x`, `y` and `z`, and a timed write
  that is not a number lands at once. It needs tracks for the width and height (there is no `VisualProperty` for them), and
  the rotation and scale lists split into their axis tracks.
- **Effects and corner radius.** A shadow, a blur and a corner radius are not tween tracks. A style change of them lands at
  once.
- **A camera.** `Camera2D` and `Camera3D` do not exist yet, so a projection blend has nothing to interpolate.
- **The end of a track.** The tree has no event for a track that ended, so `finished` follows the shell's clock and not the
  compositor's.

## Proof

`crates/gnoblin-js/tests/transition.rs` runs the staging, the transaction and the failures over a fake compositor, and
`crates/gnoblin/tests/js_shell.rs` runs a layout change inside a transition against the real compositor with two mapped
windows and an injected clock. The compositor's tracks are tested in `crates/gnoblin-wayland/tests/tree_live.rs`.
`design/js-research/spec-review.md` lists the acceptance rows AN1 to AN8, and `design/js-runtime.md`, "Transitions status",
says which are proven.
