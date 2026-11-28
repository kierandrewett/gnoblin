# Dwindle like Hyprland

`DwindleLayout` places windows the way Hyprland's dwindle layout does: each new window splits the cell of the focused
window, and a split goes side by side or top to bottom by the shape of its cell. This page lists its options under their
Hyprland names, the actions you bind to keys, and where Gnoblin differs.

Put the setup in a TypeScript file, `shell.ts`, a Python file, `shell.py`, or a Rust shell library. The three spell the same
object the same way, with each language's naming: `forceSplit` in TypeScript is `force_split` in Python and Rust. See
[Write a shell in Rust](/rust-shell) for a Rust crate, and [Python shell](/python-shell) for a Python file.

## The setup

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";
import { DwindleLayout, Workspace, Workspaces } from "gnoblin-std";

const input = gnoblin.input.primary;

// `pointer: "seat"` is what `forceSplit: "by_pointer"` and `smartSplit` read.
const dwindle = new DwindleLayout({ gap: 8, preserveSplit: true, pointer: "seat" });
Workspaces.get().add(new Workspace("main", { layout: dwindle }));

const keys: Record<string, () => unknown> = {
    "Super+j": () => dwindle.toggleSplit(),
    "Super+Shift+j": () => dwindle.swapSplit(),
    "Super+r": () => dwindle.rotateSplit(90),
    "Super+Shift+r": () => dwindle.rotateSplit(-90),
    "Super+p": () => dwindle.pseudotile(),
    "Super+Shift+Return": () => dwindle.moveToRoot(true),
    "Super+equal": () => dwindle.splitRatio(0.05),
    "Super+minus": () => dwindle.splitRatio(-0.05),
    "Super+Alt+Left": () => dwindle.preselect("left"),
    "Super+Alt+Right": () => dwindle.preselect("right"),
    "Super+Alt+Up": () => dwindle.preselect("up"),
    "Super+Alt+Down": () => dwindle.preselect("down"),
};

for (const [trigger, action] of Object.entries(keys)) {
    input.bind(trigger, () => {
        action();
    });
}
```

```python
from gnoblin import Gnoblin
from gnoblin.std import DwindleLayout, Workspace, Workspaces

# `pointer="seat"` is what `force_split="by_pointer"` and `smart_split` read.
dwindle = DwindleLayout(gap=8, preserve_split=True, pointer="seat")
Workspaces.get().add(Workspace("main", layout=dwindle))

keys = {
    "Super+j": lambda: dwindle.toggle_split(),
    "Super+Shift+j": lambda: dwindle.swap_split(),
    "Super+r": lambda: dwindle.rotate_split(90),
    "Super+Shift+r": lambda: dwindle.rotate_split(-90),
    "Super+p": lambda: dwindle.pseudotile(),
    "Super+Shift+Return": lambda: dwindle.move_to_root(True),
    "Super+equal": lambda: dwindle.split_ratio(0.05),
    "Super+minus": lambda: dwindle.split_ratio(-0.05),
    "Super+Alt+Left": lambda: dwindle.preselect("left"),
    "Super+Alt+Right": lambda: dwindle.preselect("right"),
    "Super+Alt+Up": lambda: dwindle.preselect("up"),
    "Super+Alt+Down": lambda: dwindle.preselect("down"),
}


def bind_action(trigger, action):
    def run(_event):
        action()

    Gnoblin.get().input.primary.bind(trigger, run)


for trigger, action in keys.items():
    bind_action(trigger, action)
```

```rust
use std::rc::Rc;

use gnoblin_core::{BindingEvent, InputContext, Gnoblin};
use gnoblin_std::{
    DwindleDirection, DwindleLayout, DwindleLayoutOptions, DwindlePointer, Workspace, WorkspaceOptions, Workspaces,
};
use gnoblin_std::prelude::*;

fn bind_action<E>(
    input: &InputContext,
    trigger: &str,
    mut action: impl FnMut() -> Result<bool, E> + 'static,
) -> gnoblin_std::Result<()> {
    input.bind(
        trigger,
        // An action that fails declines the press, so it goes on to the next binding.
        Some(move |_event: BindingEvent| Ok(action().unwrap_or(false))),
        None,
    )?;
    Ok(())
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;

    // `pointer: Seat` is what `force_split: ByPointer` and `smart_split` read.
    let dwindle = Rc::new(DwindleLayout::new_with(
        &objects,
        DwindleLayoutOptions {
            gap: Some(8.0),
            preserve_split: Some(true),
            pointer: Some(DwindlePointer::Seat),
            ..Default::default()
        },
    )?);
    let main = Workspace::new_with(
        &objects,
        "main",
        WorkspaceOptions { layout: Some(dwindle.as_layout()?), ..Default::default() },
    )?;
    Workspaces::get(&objects)?.add(&main)?;

    let input = Gnoblin::get(&g.objects()?)?.input()?.primary()?;

    let d = Rc::clone(&dwindle);
    bind_action(&input, "Super+j", move || d.toggle_split())?;
    let d = Rc::clone(&dwindle);
    bind_action(&input, "Super+Shift+j", move || d.swap_split())?;
    let d = Rc::clone(&dwindle);
    bind_action(&input, "Super+r", move || d.rotate_split(90.0))?;
    let d = Rc::clone(&dwindle);
    bind_action(&input, "Super+Shift+r", move || d.rotate_split(-90.0))?;
    let d = Rc::clone(&dwindle);
    bind_action(&input, "Super+p", move || d.pseudotile(None, None))?;
    let d = Rc::clone(&dwindle);
    bind_action(&input, "Super+Shift+Return", move || d.move_to_root(true))?;
    let d = Rc::clone(&dwindle);
    bind_action(&input, "Super+equal", move || d.split_ratio(0.05))?;
    let d = Rc::clone(&dwindle);
    bind_action(&input, "Super+minus", move || d.split_ratio(-0.05))?;
    let d = Rc::clone(&dwindle);
    bind_action(&input, "Super+Alt+Left", move || d.preselect(DwindleDirection::Left).map(|_| true))?;
    let d = Rc::clone(&dwindle);
    bind_action(&input, "Super+Alt+Right", move || d.preselect(DwindleDirection::Right).map(|_| true))?;
    let d = Rc::clone(&dwindle);
    bind_action(&input, "Super+Alt+Up", move || d.preselect(DwindleDirection::Up).map(|_| true))?;
    let d = Rc::clone(&dwindle);
    bind_action(&input, "Super+Alt+Down", move || d.preselect(DwindleDirection::Down).map(|_| true))?;

    Ok(())
}
```

:::

These keys are a suggestion; the stock starter binds none of them. A trigger that is already bound at the same priority
is refused, so pick keys your file does not use (see [Keyboard shortcuts](/shell-api/keyboard-shortcuts)).

An action runs on the layout you call it on, and acts on the window that has focus on the workspace that holds the layout.

## Options

Every option is also a property of the layout, so setting it later takes effect at the next compute:

- TypeScript: `dwindle.preserveSplit = false`
- Python: `dwindle.preserve_split = False`
- Rust: `dwindle.set_preserve_split(false)?`

The Rust constructor takes the same names as an options struct, `DwindleLayoutOptions`, whose fields are `Option`s, so a
field you leave `None` keeps the library default.

| Option                                                        | Hyprland                       | Default      | Meaning                                                                                                                                                                         |
| ------------------------------------------------------------- | ------------------------------ | ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `split`                                                       | none                           | `aspect`     | `aspect` follows the cell's shape. `horizontal` and `vertical` fix the direction of splits. Rust: `DwindleSplit::Aspect`, `Horizontal`, `Vertical`.                             |
| `ratio`                                                       | `default_split_ratio`          | 0.5          | The first child's share of a split, 0.1 to 0.9. See [Ratios](#ratios).                                                                                                          |
| `gap`                                                         | none                           | workspace    | Overrides the workspace's inner gap.                                                                                                                                            |
| `forceSplit` / `force_split`                                  | `force_split`                  | `by_pointer` | `by_pointer`: the new window goes on the pointer's side of the cell. `first`: left or above. `second`: right or below. Rust: `DwindleForceSplit::ByPointer`, `First`, `Second`. |
| `preserveSplit` / `preserve_split`                            | `preserve_split`               | false        | A split keeps the direction it was made with instead of following its cell's shape.                                                                                             |
| `smartSplit` / `smart_split`                                  | `smart_split`                  | false        | The pointer's triangle in the cell chooses the direction and side. Implies `preserveSplit`.                                                                                     |
| `smartResizing` / `smart_resizing`                            | `smart_resizing`               | true         | A resize drag moves the divider nearest the press; false moves both dividers of the tile.                                                                                       |
| `permanentDirectionOverride` / `permanent_direction_override` | `permanent_direction_override` | false        | `preselect` stays for every new window until cleared or changed.                                                                                                                |
| `splitWidthMultiplier` / `split_width_multiplier`             | `split_width_multiplier`       | 1            | Scales the cell's height in the side-by-side test, 0.1 to 3.0.                                                                                                                  |
| `useActiveForSplits` / `use_active_for_splits`                | `use_active_for_splits`        | true         | A new window splits the focused window. False: the window under the pointer.                                                                                                    |
| `splitBias` / `split_bias`                                    | `split_bias`                   | `first`      | `first`: the first (top or left) window gets `ratio`. `opened`: the window the new one was opened on. Rust: `DwindleSplitBias::First`, `Opened`.                                |
| `pointer`                                                     | the cursor                     | `none`       | `none`, `seat` (the seat's pointer) or `fixed` (the point `pointerX` and `pointerY` name). Rust: `DwindlePointer::None`, `Seat`, `Fixed`.                                       |
| `pointerX` / `pointer_x`, `pointerY` / `pointer_y`            | the cursor                     | none         | The point `pointer: "fixed"` reads.                                                                                                                                             |

With `pointer: "none"`, `forceSplit: "by_pointer"` puts the new window second (right or below), `smartSplit` does nothing, and
`useActiveForSplits: false` splits the first window opened. Hyprland's `precise_mouse_move` and `special_scale_factor` have
no counterpart.

### How a split picks its direction

Hyprland does not remember a split's direction: every layout pass reads the cell the split divides.

- A split that follows its cell goes top and bottom when the cell's height times `splitWidthMultiplier` is above its
  width, and side by side otherwise. A square cell with the default multiplier is side by side.
- With `preserveSplit` or `smartSplit` the direction is chosen once, when the window opens, by the opposite test: a cell
  is side by side only when its width is above its height times the multiplier, so a square cell opens top and bottom.
- Both rules are read from `DwindleAlgorithm.cpp` on Hyprland's main branch.

The multiplier scales the height, as Hyprland's source does, although its option text calls it a width multiplier. Above 1,
more splits go top and bottom, which keeps a wide monitor from making ever narrower columns.

Closing a window collapses its split, and the remaining windows split by the shape of their new cell.

## Ratios

A Gnoblin `ratio` is the first child's fraction of the split, 0.1 to 0.9. Hyprland's `splitRatio` is that fraction times
two, from 0.1 to 1.9, so its default 1.0 is 0.5 here and its 1.2 is 0.6.

- `ratio` and `splitRatio` take the Gnoblin numbers.
- Hyprland's `splitratio -0.4` is `splitRatio(-0.2)` in TypeScript, `split_ratio(-0.2)` in Python and Rust.
- Hyprland's `splitratio 1.2 exact` is `setSplitRatio(0.6)` in TypeScript, `set_split_ratio(0.6)` in Python and Rust.
- Hyprland's range reaches 0.05 and 0.95 of the cell; Gnoblin stops at 0.1 and 0.9, as it always has.

A split that a drag or `splitRatio` has set keeps its ratio while windows come and go around it. Splits that nobody set
follow `ratio`.

## Actions

Each action is a method of the layout. Those that act on a window use the window that has focus on the workspace of the
layout. All but `preselect` return a boolean, false when there was nothing to do (no focused window, no split above it), and
a successful one makes the workspace compute again. In Rust each returns `Result<bool>`, and `preselect` returns `Result<()>`.

| Action                | TypeScript                    | Python                      | Rust                                   | Hyprland layout message | Does                                                                                   |
| --------------------- | ----------------------------- | --------------------------- | -------------------------------------- | ----------------------- | -------------------------------------------------------------------------------------- |
| Toggle the split      | `toggleSplit()`               | `toggle_split()`            | `toggle_split()`                       | `togglesplit`           | Turns the split above the focused window between side by side and top and bottom.      |
| Swap the halves       | `swapSplit()`                 | `swap_split()`              | `swap_split()`                         | `swapsplit`             | Swaps the two halves of that split.                                                    |
| Rotate the split      | `rotateSplit(angle)`          | `rotate_split(angle)`       | `rotate_split(angle)`                  | `rotatesplit`           | Turns the split clockwise by a multiple of 90 degrees; negative turns the other way.   |
| Preselect a direction | `preselect(direction)`        | `preselect(direction)`      | `preselect(DwindleDirection::Left)`    | `preselect`             | Sets where the next window opens: `left`, `right`, `up`, `down`, or `none` to clear.   |
| Move to root          | `moveToRoot(stable)`          | `move_to_root(stable)`      | `move_to_root(stable)`                 | `movetoroot`            | Makes the focused window a child of the root split. `false` puts it on the other side. |
| Change the ratio      | `splitRatio(amount)`          | `split_ratio(amount)`       | `split_ratio(amount)`                  | `splitratio`            | Changes the ratio of the split above the focused window by `amount`.                   |
| Set the ratio         | `setSplitRatio(ratio)`        | `set_split_ratio(ratio)`    | `set_split_ratio(ratio)`               | `splitratio … exact`    | Sets the ratio of that split to `ratio`.                                               |
| Toggle pseudotiling   | `pseudotile(width?, height?)` | `pseudotile(width, height)` | `pseudotile(Option<f64>, Option<f64>)` | `window.pseudo`         | Toggles a window keeping its own size, centred in its cell.                            |
| Ask if pseudotiled    | `isPseudotiled(client)`       | `is_pseudotiled(client)`    | `is_pseudotiled(&client)`              | none                    | Whether a window is pseudotiled.                                                       |

- `toggleSplit` and the turn of `rotateSplit` need `preserveSplit`, as in Hyprland: without it the cell's shape sets the
  direction again at once. `swapSplit` and a turn of 180 work either way.
- `preselect` is used once, by the next window that splits a cell; the first window of an empty workspace does not use it.
  The split it makes keeps its direction even without `preserveSplit`, which is what Hyprland's documentation says; its
  source recomputes the direction at once, when the window has opened.
- A pseudotiled window keeps the size it had when you toggled it (its `width` and `height` with its frame), held to the
  size of its cell. Give a size to choose one: `pseudotile(800, 600)` in TypeScript and Python, and
  `pseudotile(Some(800.0), Some(600.0))` in Rust.

## Where Gnoblin differs

- Ratios are fractions, as the [Ratios](#ratios) section says.
- `smartResizing: true` moves one divider, the one nearest the press. Hyprland moves up to two, one for each axis, chosen
  by the nearest corner. With `smartResizing: false` Gnoblin moves two, as Hyprland does: the split above the window and the
  nearest split of the other direction above it.
- The layout has no pointer unless you give it one, so `forceSplit: "by_pointer"` and `smartSplit` follow the pointer only
  through `pointer`.
- Gaps are taken off before the ratio, as every Gnoblin tiling layout does: with a gap of 20 on 1000 pixels, a half is 490.

For the layout's place among the others see [Workspaces](/shell-api/workspaces#layouts), and for the drags
[Input handlers](/shell-api/input-handlers#tiling-drags).
