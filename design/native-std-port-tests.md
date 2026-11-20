# Native std port: golden fixtures and the test map

Lane NpOracle of `design/native-std-port.md`. It records what the TypeScript library under `data/js` does, as files the native lanes
match. It changes no production code and deletes nothing.

## 1. How it works

```text
cargo xtask oracle record              clear the fixture directories, run the TypeScript library, write every fixture
cargo xtask oracle check [--repeat N]  record into target/oracle/run-<n>, compare the whole tree with the committed one in both
                                       directions (missing, stale, changed), and the runs with each other
cargo xtask oracle list                the fixture directories and what writes each
just oracle-check                      check --repeat 3
```

The tests that write the fixtures also check them: `GNOBLIN_ORACLE` is `record` or `check` (the default), and a plain
`cargo test -p gnoblin-js --test oracle` fails on the first differing line of a fixture. `GNOBLIN_ORACLE_OUT` redirects what `record`
writes to a scratch tree laid out like the workspace; `check` uses it. The shared code is `gnoblin_testkit::oracle` (`Fixtures`,
`Writes`), which native lanes use to read the same files (`Fixtures::read_json`, `read_bytes`).

| Directory                                           | Written by                                                                                      | Holds                                                                  |
| --------------------------------------------------- | ----------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| `crates/gnoblin-std-impl/tests/fixtures/reference/` | `crates/gnoblin-js/tests/oracle/*` and `gnoblin-std-impl/tests/{tiling,examples,anim_style}.rs` | one JSON file per scenario: actions, log, transactions, tree snapshots |
| `crates/gnoblin-js/tests/fixtures/paint/`           | the same scenarios                                                                              | paint-list bytes of widgets in each state                              |
| `crates/gnoblin-core-impl/tests/fixtures/model/`    | `oracle/{model,input}.rs`                                                                       | listener call order, input matcher decisions and claims                |

The layout of a file is in the `README.md` of each directory. Three rules keep a recording stable: time is the injected `ManualClock`;
text is measured by a fixed rule (`answer_measures`), never a font; and a node the shell made is named by the order it was made in, so a
native shell that numbers its nodes differently compares equal. The lock screen's clock and a scenario's scratch directory are fixed or
replaced (`<root>`), and `xtask oracle` runs every producer with `TZ=UTC` and `Date.now` fixed, so the clocks the toolkit draws read the same everywhere.

`tests/{tiling,examples,anim_style}.rs` of `gnoblin-std-impl` still run the TypeScript beside the native std. In `record` mode they
only run the TypeScript side. In `check` mode they compare the TypeScript run with the fixture, then the native run with the TypeScript
run, as before. The lane that deletes the reference swaps the TypeScript side for `Fixtures::read_json`.

## 2. Coverage

| Area (TypeScript module)                                                                                                | Fixture                                                                                                          | What it holds                                                                                                                                                   | Lane that matches it      |
| ----------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------- |
| Tiling layouts, split drags, snap, swap, strip, arc, carousel, move/resize drags (`layouts/*`, `bindings/*`, `snap.ts`) | `reference/tiling_*.json` (18), `examples_*.json` (2)                                                            | rectangle, tiled, visible, depth, rotation, scale of each window per step                                                                                       | NpStdObjects, NpStdPolicy |
| Transitions, style writes, camera writes (`transition/*`, `style/*`, `scene/*`)                                         | `reference/anim_style_transitions.json`, `transition_*.json` (10)                                                | timed and plain writes, easing samples, tween nodes, ownership, held and reversed transitions, refusals                                                         | NpStdStyle                |
| Style cascade and refusals (`style/*`)                                                                                  | `reference/style_*.json` (10)                                                                                    | origins, copy-on-write, frozen snapshots, every refusal message, effects on the window node, shader filters with the seat property true, false and absent (absent means on)                    | NpStdStyle                |
| UI toolkit layout, text, paint, events, entry, clock, bar, toast (`ui/*`, `widgets.ts`, `panel.ts`)                     | `reference/ui_*.json` (8), `paint/ui_*.bin` (20)                                                                 | rectangles of nested rows, columns and grids, metrics before and after an answer, every paint op, bubbling and stop, entry in 10 states                         | NpStdUi                   |
| Window frame and titlebar (`frame/*`, `titlebar.ts`)                                                                    | `reference/frame_*.json` (19), `paint/frame_*.bin` (19)                                                          | room on each side, outlets, layers, zones, parts, buttons, close fade, clicks, drag and resize, rules, the three recipes of `docs/recipes/window-frame.md`      | NpStdFrame                |
| Window menu (`menus/*`)                                                                                                 | `reference/menus_*.json` (11), `paint/menus_*.bin` (7)                                                           | card geometry near edges, keys, pointer, workspace submenu, entries, lifecycle, secure cards, scripted items                                                    | NpStdMenus                |
| Alert, launcher, overview, lock screen (`alerts.ts`, `launcher.ts`, `overview.ts`, `lock.ts`)                           | `reference/parts_*.json` (10), `paint/parts_*.bin` (13)                                                          | arming, force quit, recovery, typing, every lock message                                                                                                        | NpStdShellUi              |
| Scene math, nodes, cameras, views, bindings (`gnoblin-core/math/*`, `scene/*`, `properties.ts`)                         | `reference/scene_*.json` (10)                                                                                    | vector, ray, plane, pose tables (rounded to ten decimals), parsing and refusals, node and view writes, camera defaults, binding dependency tracking             | NpStdScene                |
| Object model listeners (`host/*`, `subscriptions.ts`, `session/*`)                                                      | `model/model_*.json` (13)                                                                                        | order of object, collection and `gnoblin` listeners for 10 client and 4 output events, lifecycle, hotplug, `where`, `find`, focus history, timers, dead objects | NpCoreModel               |
| Input matcher (`bindings/*`, `input/*`)                                                                                 | `model/input_*.json` (3) and `reference/tiling_{binding_priority,directed_gesture,gesture_cancel_on_close}.json` | trigger grammar and refusals, claims, tiers, priority, decline, device claims, drags, gestures, cancel on close                                                 | NpCoreInput               |

## 3. Not covered, and why

- **Golden scripts** (`tests/modular/golden-scripts/{shell.ts,shell.py}` and their recorded trace). Not done. They need the generated SDKs
  and a real-session run of the modular harness, so they belong to the lane that generates the SDKs (NpGenOut/NpObjectAbiB), which can
  record the trace from the released catalog.
- **The Python half.** The scenarios run the TypeScript library only. A Python scenario that drives the same actions is each native lane's own
  proof (section 6.5 of the plan).
- **Core services** (`session`, `process`, `idle`, `launches`, `dialogs`, `authentication`, virtual outputs, output config). The existing
  `gnoblin-js/tests/{host_*,lock,outputs}.rs` are their TypeScript half; no fixture is recorded.
- **Window policy** (`focus.ts`, `window_state.ts`, `reposition.ts`), **displays**, **XR**, **workspace slide**, **hot reload and claim
  handover**. The existing `stock_focus`, `window_policy`, `client_requests`, `reposition`, `displays`, `xr` and `input` (reload) tests are the cases the
  lanes replay; only the tiling fixtures above record policy results.
- **Text widths from a real font.** Native text is measured in process (decision O9); the fixtures use the fixed rule above, so a native lane
  injects the same measurer in its tests.
- **Reload and retirement behaviour of nodes** (`window_frame` reload, `lock` retire).

## 4. Which tests exercise which TypeScript module

This is read from the tests' own headers and imports; it was not taken from a loader import log. It names the files whose subject is the
module; a module also runs, unasserted, in any test that imports `gnoblin-std`.

| Module                                                           | Tests                                                                                                       |
| ---------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `ui/*`, `widgets.ts`                                             | `ui.rs`, `stock_ui.rs`, `wake_sources.rs`, `frame_gating.rs`, oracle `ui`                                   |
| `frame/*`, `titlebar.ts`                                         | `window_frame.rs`, `ui.rs`, `stock_ui.rs`, `frame_gating_window_frame.rs`, oracle `frame`                   |
| `menus/*`, `alerts.ts`, `launcher.ts`, `overview.ts`, `panel.ts` | `stock_ui.rs`, oracle `menus`, `parts`                                                                      |
| `lock.ts`                                                        | `lock.rs`, `write_refusals.rs`, oracle `parts`                                                              |
| `scene/*`, `math/*`, `scene_mirror.ts`                           | `scene_spec.rs`, `xr.rs`, oracle `scene`                                                                    |
| `style/*`, `transition/*`                                        | `style.rs`, `transition.rs`, `stage_effects.rs`, `time_units.rs`, `anim_style.rs`, oracle `style`           |
| `layouts/*`, `layout.ts`, `workspace.ts`                         | `stock_layouts.rs`, `stock_layouts2.rs`, `dwindle_hyprland.rs`, `workspaces.rs`, `tiling.rs`, `examples.rs` |
| `bindings/*`, `gestures.ts`, `gesture_actions.ts`, `snap.ts`     | `tiling_bindings.rs`, `input.rs`, `window_policy.rs`, `tiling.rs`, oracle `input`                           |
| `focus.ts`, `window_state.ts`, `reposition.ts`                   | `stock_focus.rs`, `client_requests.rs`, `window_policy.rs`, `reposition.rs`                                 |
| `display/*`, `display_manager/*`                                 | `displays.rs`, `outputs.rs`                                                                                 |
| `extensions/xr/*`                                                | `xr.rs`                                                                                                     |
| `gnoblin-core/host/*`, `subscriptions.ts`, `session/*`           | `subscriptions.rs`, `host_api.rs`, `host_global.rs`, `client_requests.rs`, oracle `model`                   |
| `gnoblin-core/bindings/*`, `input/*`, `devices.ts`               | `input.rs`, `input_devices.rs`, oracle `input`                                                              |

## 5. Declaration compatibility

`tests/ts-compat/old-surface/` is the declaration emit of the library as it stands (`tsc --declaration --emitDeclarationOnly`, TypeScript
7.0.2, strict; 229 files). `tests/ts-compat/compat.ts` asserts, for 34 classes a shell extends or constructs, that the class of
`gnoblin-std-next` is assignable to the old one. Until the generator produces declarations, `tsconfig.json` maps `gnoblin-std-next` to the old
tree, so the file type-checks against the old declarations alone (`tests/ts-compat/check.sh`). The lane that generates `gnoblin-std.d.ts`
changes that one `paths` entry. NpGate deletes the directory after the owner signs off the new surface.
