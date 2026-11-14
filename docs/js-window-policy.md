# Window policy in TypeScript

The window policy of the TypeScript shell: moving and resizing with the keyboard, touchpad
gesture actions, reduced motion, binding-only devices, focus hand-off, workspace removal and workspace sets. Each section
names the test that proves it (`crates/gnoblin-js/tests/{reposition,window_policy,input}.rs`).

## Move and resize with the keyboard

```ts
gnoblin.clients.at(0)?.moveWithKeys();
gnoblin.clients.at(0)?.resizeWithKeys();
```

The arrow keys move the window by 16 pixels inside the work area. For a resize, Right and Down grow it and Left and Up
shrink it, never below 120 by 80.

- Return, Enter and Space keep the result; Escape puts the window back.
- The mode holds the keyboard while it runs and gives it back to the window.
- It ends when the window closes and when the session locks.
- Both methods return `false` for a window that is fullscreen, maximised, tiled or not showing.
- A snapped window is unsnapped first, and Escape snaps it again.

The window menu's Move and Resize start these.

## Gesture actions

```ts
defineConfig({
    gestureActions: {
        Swipe4: { up: "window.toggle-maximise", left: "workspace.next", right: "workspace.previous" },
        Pinch3: { in: "window.minimise", out: (event) => console.log(event.scale) },
    },
});
```

A gesture is `Swipe` or `Pinch` and a finger count. An action runs once, when the fingers lift, for the direction the
gesture went; a cancelled gesture runs nothing.

- The named actions are `workspace.next`, `workspace.previous`, `window.close`, `window.minimise` and
  `window.toggle-maximise`.
- `gnoblin.gestures.actions` holds them as functions.
- `onSwipe(map, threshold?)`, `onPinch(map, threshold?)` and `direction(event, threshold?)` of `gnoblin.gestures` build
  bindings for `gnoblin.input.primary.bind`.
- A gesture here replaces the stock three-finger slide when it has the same trigger.
- A gesture event carries the raw `dx`, `dy`, `scale` and `rotation` beside `progress`.

## Reduced motion

While the seat's `reduced_motion` is true, `gnoblin.motion(seconds)` is 0.

- Workspace slides and `Client.animate` take no time.
- The titlebar's close fade is skipped.
- The slide back of a short swipe is instant.

## Binding-only devices and the `false` action

`device.bind(trigger, action, { only: true })` makes the device binding-only: its keys reach no client and fire only the
bindings made for it or for every device. `device.bindingOnly = false` gives the claim up. An action of `false` claims
the trigger and does nothing with it; a function that returns `false` declines and lets the next binding have it.

## Focus and workspaces

- A focused window that closes hands the keyboard to its neighbour in the workspace (the next, else the previous; a
  dialog hands it to its parent) in every layout that has no `onClientRemoved` of its own.
- `Workspace.walk(step)` moves the focus along the workspace's windows.
- `gnoblin.workspaces.remove(ws)` moves the windows to the neighbour before the workspace (the one after, for the
  first), shows it where the removed one was shown, and refuses the last workspace (returns `false`).
- A left press on a titlebar focuses and raises its window as a press on the window does.
- Maximised, snapped and floating windows are refitted when a bar changes the work area.
- A reload keeps the rectangles a maximised, fullscreen or snapped window returns to.

## Workspace sets

```ts
defineConfig({
    workspaceSets: [
        {
            name: "work",
            entries: [
                { workspace: "2", output: "DP-1" },
                { workspace: "3", output: "HDMI-1" },
            ],
        },
    ],
});
gnoblin.workspaces.activateSet("work");
```

Each entry shows its workspace on its output; one with no output, or whose output is not connected (reported), shows on the
primary output. A list that does not check refuses the config. `activateSet` returns `false` and reports for an unknown
set; `gnoblin.workspaces.sets.activate` throws instead.
