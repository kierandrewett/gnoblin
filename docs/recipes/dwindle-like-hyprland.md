# Dwindle like Hyprland

`DwindleLayout` places windows the way Hyprland's dwindle layout does: each new window splits the cell of the focused
window, and a split goes side by side or top to bottom by the shape of its cell. This page lists its options under their
Hyprland names, the actions you bind to keys, and where Gnoblin differs. Put the code in `~/.config/gnoblin/shell.ts`.

## The setup

```ts
import { DwindleLayout, Workspace } from "gnoblin-std";

const input = gnoblin.input.primary;

const dwindle = new DwindleLayout({
    gap: 8,
    preserveSplit: true,
    // A layout has no pointer of its own: this is what `forceSplit: 0` and `smartSplit` read.
    pointer: () => input.pointer,
});
gnoblin.workspaces.add(new Workspace({ id: "main", layout: dwindle }));

type Action = (layout: DwindleLayout, workspace: Workspace) => unknown;

const keys: Record<string, Action> = {
    "Super+j": (layout, workspace) => layout.toggleSplit(workspace),
    "Super+Shift+j": (layout, workspace) => layout.swapSplit(workspace),
    "Super+r": (layout, workspace) => layout.rotateSplit(workspace, 90),
    "Super+Shift+r": (layout, workspace) => layout.rotateSplit(workspace, -90),
    "Super+p": (layout, workspace) => layout.pseudotile(workspace),
    "Super+Shift+Return": (layout, workspace) => layout.moveToRoot(workspace),
    "Super+equal": (layout, workspace) => layout.splitRatio(workspace, 0.05),
    "Super+minus": (layout, workspace) => layout.splitRatio(workspace, -0.05),
    "Super+Alt+Left": (layout) => layout.preselect("left"),
    "Super+Alt+Right": (layout) => layout.preselect("right"),
    "Super+Alt+Up": (layout) => layout.preselect("up"),
    "Super+Alt+Down": (layout) => layout.preselect("down"),
};

for (const [trigger, action] of Object.entries(keys)) {
    input.bind(trigger, () => {
        // The workspace on the display under the pointer, when its layout is a DwindleLayout.
        const display = input.activeDisplay as { viewport: { content: Workspace | null } } | null;
        const workspace = display?.viewport.content;
        if (workspace?.layout instanceof DwindleLayout) {
            action(workspace.layout, workspace);
        }
    });
}
```

These keys are a suggestion; the stock starter binds none of them. A trigger that is already bound at the same priority
is refused, so pick keys your file does not use (see [Keyboard shortcuts](/shell-api/keyboard-shortcuts)). The binds
need a live seat: `input.pointer` throws in a dry run.

## Options

Every option is a field of the layout too, so `dwindle.preserveSplit = false` takes effect at the next compute.

| Option                       | Hyprland                       | Default   | Meaning                                                                                        |
| ---------------------------- | ------------------------------ | --------- | ---------------------------------------------------------------------------------------------- |
| `split`                      | none                           | `aspect`  | `aspect` follows the cell's shape. `horizontal` and `vertical` fix the direction of splits.    |
| `ratio`                      | `default_split_ratio`          | 0.5       | The first child's share of a split, 0.1 to 0.9. See [Ratios](#ratios).                         |
| `gap`                        | none                           | workspace | Overrides the workspace's inner gap.                                                           |
| `forceSplit`                 | `force_split`                  | 0         | 0: the new window goes on the pointer's side of the cell. 1: left or above. 2: right or below. |
| `preserveSplit`              | `preserve_split`               | false     | A split keeps the direction it was made with instead of following its cell's shape.            |
| `smartSplit`                 | `smart_split`                  | false     | The pointer's triangle in the cell chooses the direction and side. Implies `preserveSplit`.    |
| `smartResizing`              | `smart_resizing`               | true      | A resize drag moves the divider nearest the press; false moves both dividers of the tile.      |
| `permanentDirectionOverride` | `permanent_direction_override` | false     | `preselect` stays for every new window until cleared or changed.                               |
| `splitWidthMultiplier`       | `split_width_multiplier`       | 1         | Scales the cell's height in the side-by-side test, 0.1 to 3.0.                                 |
| `useActiveForSplits`         | `use_active_for_splits`        | true      | A new window splits the focused window. False: the window under the pointer.                   |
| `splitBias`                  | `split_bias`                   | 0         | 0: the first (top or left) window gets `ratio`. 1: the window the new one was opened on.       |
| `pointer`                    | the cursor                     | none      | A function that returns the pointer's `{ x, y }`, or null.                                     |

Without a `pointer`, `forceSplit: 0` puts the new window second (right or below), `smartSplit` does nothing, and
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
- Hyprland's `splitratio -0.4` is `layout.splitRatio(workspace, -0.2)`.
- Hyprland's `splitratio 1.2 exact` is `layout.splitRatio(workspace, 0.6, { exact: true })`.
- Hyprland's range reaches 0.05 and 0.95 of the cell; Gnoblin stops at 0.1 and 0.9, as it always has.

A split that a drag or `splitRatio` has set keeps its ratio while windows come and go around it. Splits that nobody set
follow `ratio`.

## Actions

Each action is a method of `DwindleLayout`. Those that act on a window take the workspace and use the window that has
focus. All return a boolean, false when there was nothing to do (no focused window, no split above it), and a successful
one makes the workspace compute again.

| Method                                     | Hyprland layout message | Does                                                                                           |
| ------------------------------------------ | ----------------------- | ---------------------------------------------------------------------------------------------- |
| `toggleSplit(workspace)`                   | `togglesplit`           | Turns the split above the focused window between side by side and top and bottom.              |
| `swapSplit(workspace)`                     | `swapsplit`             | Swaps the two halves of that split.                                                            |
| `rotateSplit(workspace, angle = 90)`       | `rotatesplit`           | Turns the split clockwise by a multiple of 90 degrees; negative turns the other way.           |
| `preselect(direction)`                     | `preselect`             | Sets where the next window opens: `left`, `right`, `up`, `down`, or `none` to clear.           |
| `moveToRoot(workspace, { stable })`        | `movetoroot`            | Makes the focused window a child of the root split. `stable: false` puts it on the other side. |
| `splitRatio(workspace, amount, { exact })` | `splitratio`            | Changes the ratio of the split above the focused window, or sets it with `exact`.              |
| `pseudotile(workspace, size?)`             | `window.pseudo`         | Toggles a window keeping its own size, centred in its cell.                                    |
| `pseudotiled(node)`                        | none                    | Whether a window is pseudotiled.                                                               |

- `toggleSplit` and the turn of `rotateSplit` need `preserveSplit`, as in Hyprland: without it the cell's shape sets the
  direction again at once. `swapSplit` and a turn of 180 work either way.
- `preselect` is used once, by the next window that splits a cell; the first window of an empty workspace does not use it.
  The split it makes keeps its direction even without `preserveSplit`, which is what Hyprland's documentation says; its
  source recomputes the direction at once, when the window has opened.
- A pseudotiled window keeps the size it had when you toggled it (its `width` and `height` with its frame), held to the
  size of its cell. Give a size, `pseudotile(workspace, { width: 800, height: 600 })`, to choose one.

## Where Gnoblin differs

- Ratios are fractions, as the [Ratios](#ratios) section says.
- `smartResizing: true` moves one divider, the one nearest the press. Hyprland moves up to two, one for each axis, chosen
  by the nearest corner. With `smartResizing: false` Gnoblin moves two, as Hyprland does: the split above the window and the
  nearest split of the other direction above it.
- The layout has no pointer unless you give it one, so `forceSplit: 0` and `smartSplit` follow the pointer only through
  `pointer`.
- Gaps are taken off before the ratio, as every Gnoblin tiling layout does: with a gap of 20 on 1000 pixels, a half is 490.

For the layout's place among the others see [Workspaces](/shell-api/workspaces#layouts), and for the drags
[Input handlers](/shell-api/input-handlers#tiling-drags).
