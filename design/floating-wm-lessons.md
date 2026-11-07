# Floating window lessons: Sanctum and COSMIC

Status: research note, not a decision. It records how two other desktops handle floating windows, what Gnoblin does today, and the gaps. Nothing here is implemented. Decisions stay in [rust-compositor.md](rust-compositor.md).

Citations are `path:line`. Gnoblin paths are in this repository. COSMIC paths are in `pop-os/cosmic-comp` on `master`, read raw on 2026-11-07. `[INFERENCE]` marks a claim I did not check against code or a primary source.

Sanctum's source is closed. `github.com/marc2332/sanctum` returns 404 [INFERENCE: private or renamed]. Its section rests on the author's blog post only.

## Summary

- COSMIC is the useful reference for floating behaviour. Its source is public and specific.
- Sanctum shows a different shape: one author, every shell surface drawn with Freya, and no code to study.
- Gnoblin has the policy skeleton: placement modes, a work-area constraint, snap zones and a frame protocol. It lacks the interactive grab that feeds them, and it does not read client size limits.
- COSMIC is `GPL-3.0-only`. Gnoblin is `GPL-2.0-or-later`. Take ideas from COSMIC, not code.

## Where each idea lives

Layer key: **A** is Gnoblin core, a compositor mechanism. **B** is user Lua, a layout or config the user writes. **C** is a shell config or package, for presentation and policy. **D** is a mix, and the cell names the split.

Rule applied: Gnoblin owns compositor and session services. Shell presentation belongs to a shell integration (`AGENTS.md:30-31`). Each idea takes the lowest layer that works. Lua gaps are checked against `crates/gnoblin-schema/schema/` and `crates/gnoblin-lua/host_members.toml`.

| Rank | Idea                                                  | Layer | Why                                                                         | Minimal core hook                                                                                        | Lua API (gaps marked)                                                                                                                                                        |
| ---- | ----------------------------------------------------- | ----- | --------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1    | Resize edges and cursor                               | C     | The frame renderer draws edges and cursors; core only receives actions.     | None new. Frame protocol reports actions (`gnoblin_window_frame_v1.rs:124-149`).                         | GAP: no Lua frame hook.                                                                                                                                                      |
| 2    | Stacks and tabs                                       | D     | Grouping is layout (B); the tab bar is shell drawing (C).                   | Hidden members use placement `visible` (`crates/gnoblin-lua/src/layout/reply.rs`).                       | Layout can hide members. GAP: no tab bar hook.                                                                                                                               |
| 3    | Sticky flag                                           | A     | Sticky changes workspace membership, so the model must hold it [INFERENCE]. | Sticky flag on the workspace model.                                                                      | GAP: no `sticky` property or method in `window.toml`.                                                                                                                        |
| 4    | Client min and max size                               | A     | Clamping needs client hints, which only core receives.                      | Store hints on `Window`; clamp in placement and `move_resize`.                                           | GAP: no `min_size` or `max_size`.                                                                                                                                            |
| 5    | Anchored placement (and the anchored cascade gap row) | D     | Core knows the parent; the placement rule is a user choice.                 | Pass the parent as an anchor into placement.                                                             | `new_window_placement` is config. GAP: no anchor field.                                                                                                                      |
| 6    | Urgency instead of deny                               | D     | Core records the request; the shell draws the badge.                        | `urgent` flag, set where `Prevent` denies (`wm.rs:1253-1259`).                                           | GAP: no `urgent` in `window.toml`.                                                                                                                                           |
| 7    | Window animation through transforms                   | D     | Lua picks transforms; core plays them; curves are shell style.              | Per-frame driver applies transform and opacity. [INFERENCE]                                              | Placements set `transform` and `opacity` per arrange (`reply.rs:81-101`). GAP: no per-window setter or tween. Effects are blur, shadow, shader only (`window.toml:116-120`). |
| 8    | Move and resize grab                                  | A     | Pointer deltas come from the input path, which core owns.                   | New pointer grab in `gnoblin-input` (today chords only, `chord.rs:30`) feeds `MoveResize` (`wm.rs:429`). | GAP: no grab API; no `move` or `resize` on `window`.                                                                                                                         |
| 9    | Edge snapping during drag                             | D     | Zones are shell policy; feeding them during a drag is core.                 | Grab emits `SnapRequested` (`wm.rs:426`), gated by `edge_tiling` (`wm.rs:1476`).                         | GAP: snap is event-only. Zones are shell config.                                                                                                                             |
| 10   | Frame composition                                     | D     | Core composes surfaces; the shell owns look and policy.                     | Compose frame surfaces in the compositor. Not done (`gnoblin_window_frame_v1.rs:33`).                    | GAP: no Lua frame API.                                                                                                                                                       |
| -    | Float toggle in tiling (gap row)                      | A     | Floating is layout state; D25 removed the per-window flag.                  | `allows_toggle_float` (`layout/kind.rs:46-48`) decides.                                                  | `ToggleFloat` event (`wm.rs:427`); no Lua method.                                                                                                                            |
| -    | Click to raise (gap row)                              | A     | Stacking is core state, so the click must reach core [INFERENCE].           | `raise_on_click` config (`wm.rs:108-109`).                                                               | GAP: no `raise` method; only `focus`.                                                                                                                                        |

Verified facts behind the gaps: `window.toml` has no move, resize, sticky or urgent property or method. Layouts see `Place` and `Move` requests for tiled windows only (`crates/gnoblin-wm/src/layout/placement.rs:203-215`). `host_members.toml` has `configure`, `load` and `timers`, and no input or animation member.

## What this means for Gnoblin core

Core must add only these mechanisms:

- A pointer grab in `gnoblin-input`, feeding `MoveResize` and `SnapRequested` (rows 8 and 9). Input routing is core.
- Client size hints on `Window`, used by placement and `move_resize` (row 4).
- An anchor field on the new-window request, passed to placement (row 5).
- Urgent and sticky state on the window model, exposed in `window.toml` (rows 3 and 6).
- A per-frame driver that applies Lua-set transform and opacity (row 7). [INFERENCE: layout replies already set these per arrange; the per-frame feed is new.]
- Frame composition in the compositor (row 10). The protocol does not compose yet (`gnoblin_window_frame_v1.rs:33`).

Not core: snap zones, animation curves, tab bars, resize cursor shapes and window menus. The menu stays a signal (`menu_requested`, `window.toml:206`; D29 in `design/rust-compositor.md:1630`). The float toggle needs no new core: D25 decides it.

## Sanctum

### What it is

- A Linux desktop environment written in Rust, by the author of Freya (`https://mespin.me/posts/sanctum`, 24 October 2026).
- The author has daily-driven it on Ubuntu for about six months. The post calls it unstable and says the source is "closed until I find it stable enough to be shared".
- The post says it has "its own wayland compositor". It does not name the library underneath. [INFERENCE] Smithay is likely, but unverified.

### How its UI is built

- The post says all UI is Freya. That covers the Dock, Control Panel, floating notifications and the Alt+Tab switcher, plus the Settings, File Explorer, Monitor and Image viewer apps.
- Freya is a declarative Rust GUI library drawn with Skia (`https://freyaui.dev/posts/0.5`). Shell and apps share one toolkit.
- Window placement, decorations and config format are not public. Nothing about them is verified.

### What it teaches

- One toolkit for shell and apps keeps theming and state in one place. Gnoblin draws its shell outside the compositor (`design/native-ui-removal.md`), so the trade-off runs the other way.
- The author says the code is "not so about the code quality", and says there is "no open source project (for now?)". Treat it as a product note, not a design reference.

## COSMIC (cosmic-comp)

### What it is

- The compositor of the COSMIC desktop from System76. Licence `GPL-3.0-only`, Rust, edition 2024 (`Cargo.toml`).
- Built on Smithay `0.7.0`, with a patch pinned to `rev = "8a27023"` (`Cargo.toml`, `[dependencies.smithay]` and `[patch.crates-io]`).
- Imports `cosmic_config` and `cosmic_comp_config` for settings (`src/shell/mod.rs`, imports block).
- The brief says the title bars are iced-based. I did not re-check that in this pass.

### Structure

- `src/shell/` holds the model: `element/`, `focus/`, `grabs/`, `layout/`, `workspace.rs`, `seats.rs`, `zoom.rs` and `mod.rs` (`src/shell/mod.rs`, module list).
- `src/shell/grabs/` holds `MoveGrabState` (`moving.rs`) and the resize and menu grabs.
- `src/shell/layout/floating/` holds `mod.rs`, which owns `map_anchored` and sizing (`mod.rs:451-668`), and `grabs/` for resize.
- `src/wayland/handlers/` has one file per protocol, for example `decoration.rs`, `xdg_activation.rs`, `toplevel_management.rs` and `session_lock.rs`, per the file list in the brief.

### Floating behaviour, as read

- **Placement.** `map_anchored` (`src/shell/layout/floating/mod.rs:451-560`) sizes a new window with no size given. If it is larger than two-thirds of the output, it is capped to two-thirds. Client `min_size` and `max_size` are read, with a 320x240 default minimum (`mod.rs:467-481`).
- **Cascade.** Without a position, the window steps 48 px down from the last spawned window, and keeps a 16 px bottom margin (`mod.rs:532-560`). Anchored windows are centred on their anchor (`mod.rs:520`).
- **Decorations.** `src/wayland/handlers/decoration.rs` picks server-side decorations for a stack. Otherwise it uses the config preference, `cosmic_conf.decoration_preference`. A client's requested mode is stored per window in `PreferredDecorationMode`.
- **Snap zones.** `SnappingZone` in `src/shell/grabs/moving.rs:292-303` has Maximize, Top, four corners and four sides. The ranges are `SNAP_RANGE` 32, `SNAP_RANGE_MAXIMIZE` 22 and `SNAP_RANGE_TOP` 16 (`moving.rs:305-307`).
- **Decoration sizes.** `SSD_HEIGHT` 36 and `RESIZE_BORDER` 10, from the earlier research notes [INFERENCE: not re-checked this pass].
- **Animation.** `ANIMATION_DURATION` is 200 ms, with ease-in-out cubic fades for overview and resize mode (`src/shell/mod.rs`). A spring module sits in `src/backend/render/animations/`.
- **Window state.** `PendingWindow` holds `fullscreen`, `minimized`, `maximized` and `sticky` (`src/shell/mod.rs`).
- **Activation.** `ACTIVATION_TOKEN_EXPIRE_TIME` is 5 s (`src/shell/mod.rs`). `xdg_activation.rs` handles the token.
- **Stacks.** `CosmicStack` is a tabbed group in `src/shell/element/`. The stack toggle code is near lines 1076-1156 of a file under `src/shell/` [INFERENCE: file not re-checked].

### Protocols

- COSMIC uses `cosmic_protocols` workspace v2 and `toplevel_info` (`src/shell/mod.rs` imports).
- Its handler directory also has `overlap_notify`, `output_power`, `image_copy_capture` and `toplevel_management`, per the brief.
- Gnoblin has `xdg_activation_v1.rs`, `ext_image_copy_capture_v1.rs`, `ext_session_lock_v1.rs` and `zwlr_foreign_toplevel_management_v1.rs` in `crates/gnoblin-wayland/src/protocols/`. Those gaps are smaller than the list suggests.

## Feature table: floating management

| Feature                         | Sanctum     | COSMIC                                | Gnoblin today (file:line)                                                                       |
| ------------------------------- | ----------- | ------------------------------------- | ----------------------------------------------------------------------------------------------- |
| New-window placement            | Not public  | Anchored cascade, two-thirds sizing   | `FreeSpace`, `Center`, `Cascade` (`crates/gnoblin-wm/src/placement.rs:22-28`)                   |
| Move grab and snapping          | Not public  | Snap zones (`moving.rs:292-307`)      | No pointer grab. `move_window` is a test seam (`crates/gnoblin-wayland/src/state.rs:2184-2194`) |
| Resize grab and edges           | Not public  | Edge snap in `floating/grabs`         | `MoveResize` event only (`crates/gnoblin-wm/src/event.rs`)                                      |
| Resize border and cursor        | Not public  | 10 px border                          | Not implemented; frame reports twelve actions (`gnoblin_window_frame_v1.rs:124-149`)            |
| Client min and max size         | Not public  | Read at placement (`mod.rs:467-481`)  | Not read. No `min_size` or `max_size` in `gnoblin-wm` or `gnoblin-wayland` (grep)               |
| Click to raise                  | Not public  | Not checked                           | `raise_on_click` default true (`crates/gnoblin-wm/src/wm.rs:127`)                               |
| Focus follows pointer           | Not public  | Not checked                           | `focus_mode` and `pointer_moved` (`crates/gnoblin-wm/src/wm.rs:1273`)                           |
| Focus stealing prevention       | Not public  | Activation token, 5 s expiry          | `Prevent` denies (`crates/gnoblin-wm/src/wm.rs:1247-1259`)                                      |
| Always-on-top and sticky        | Not public  | `sticky` in `PendingWindow`           | None. Layers exist (`crates/gnoblin-wm/src/stack.rs`) but no sticky flag                        |
| Minimise and restore            | Not public  | `minimized` state                     | `Minimise` and `Unminimise` events (`crates/gnoblin-wm/src/event.rs:142-144`)                   |
| Maximise and fullscreen restore | Not public  | `maximized`, `fullscreen` state       | Restore rect stored (`crates/gnoblin-wm/src/wm.rs:1614`)                                        |
| Float and tile toggle           | Not public  | Per window                            | Tiling only (`crates/gnoblin-wm/src/layout/kind.rs:46-48`)                                      |
| Stacks and tabs                 | Not public  | `CosmicStack` tabbed groups           | None. `stack.rs` holds layers only                                                              |
| Decorations                     | Freya shell | Config preference, stacks server-side | Always client-side (`crates/gnoblin-wayland/src/protocols/zxdg_decoration_v1.rs:43`)            |
| Animation                       | Not public  | 200 ms fades                          | No window animation found by grep in `gnoblin-lua` or `gnoblin-wm`                              |

## Gap table: what Gnoblin lacks

Effort: S is a few days, M a week or two, L longer. Home says whether the work belongs in core Rust or in a Lua layout or extension. These are my judgements [INFERENCE].

| Gap                        | Sanctum     | COSMIC                          | Gnoblin today (file:line)                                                                                | Home                    | Effort |
| -------------------------- | ----------- | ------------------------------- | -------------------------------------------------------------------------------------------------------- | ----------------------- | ------ |
| Interactive move grab      | Not public  | Move grab, `moving.rs`          | Missing. `drag::constrain` is called from `move_resize` (`wm.rs:1601-1606`) but no pointer drag feeds it | Core, input layer       | M      |
| Interactive resize grab    | Not public  | Resize grab in `floating/grabs` | Missing. `MoveResize` only                                                                               | Core, input layer       | M      |
| Edge snapping during drag  | Not public  | Snap zones                      | Zones exist (`snapping.rs:14`). Snap needs `edge_tiling` (`wm.rs:1476`)                                  | Core policy, Lua option | S-M    |
| Client min and max size    | Not public  | Read at placement               | Not read                                                                                                 | Core, geometry clamp    | S      |
| Resize edge detection      | Not public  | 10 px border                    | Frame renderer reports actions (`gnoblin_window_frame_v1.rs:124-149`)                                    | External frame renderer | M      |
| Anchored cascade           | Not public  | `map_anchored`                  | `Cascade` exists (`placement.rs:28`), different rule                                                     | Core, small change      | S      |
| Urgency for focus requests | Not public  | Activation token                | Deny only (`wm.rs:1247-1259`)                                                                            | Core plus shell signal  | S-M    |
| Server-side frames         | Freya shell | Config preference               | Frame protocol exists; not composed (`gnoblin_window_frame_v1.rs:30-40`)                                 | Shell renderer card     | L      |
| Stacks and tabs            | Not public  | `CosmicStack`                   | None                                                                                                     | Lua layout plus frame   | L      |
| Window animation           | Not public  | 200 ms fades                    | None for windows                                                                                         | Lua extension           | S-M    |
| Sticky and always-on-top   | Not public  | `sticky` state                  | None (`stack.rs` has layers only)                                                                        | Core, stack             | S      |
| Float toggle in tiling     | Not public  | Per window                      | Refused outside tiling (`kind.rs:46-48`)                                                                 | Core, D25 decision      | S      |

Two facts shape the table. `floating.lua` leaves `on_request` nil, so it keeps windows where they are (`data/lua/gnoblin/layouts/floating.lua`). X11 move and resize requests are logged and not handled (`crates/gnoblin-wayland/src/xwayland.rs:614-625`).

## Ideas worth taking

Ranked by core effort, least first. Each one is independent unless noted.

1. **Resize edges and cursor.** Detection and cursor shape stay in the frame renderer. Core already receives the actions. Core, none.
2. **Stacks and tabs.** A tabbed group is a Lua layout with a shell tab bar. Core needs only placement `visible`, which exists. Drawing needs idea 10. Core, S.
3. **Sticky flag.** Hold sticky in the workspace model, not as a stack layer. Core, S.
4. **Read client min and max size.** COSMIC reads both at placement (`mod.rs:467-481`). Store them on `Window` and clamp in placement and `move_resize`. Core, S.
5. **Anchored placement as a mode.** COSMIC centres a child on its anchor (`mod.rs:520`). Gnoblin needs an anchor field on the new-window request. Core, S.
6. **Urgency instead of silent deny.** `Prevent` marks the window urgent and still refuses focus. The shell draws the signal. Core, S-M.
7. **Window animation through transforms.** Lua animates transform and opacity, with 200 ms as COSMIC uses. Core needs a per-frame driver. Core, M [INFERENCE].
8. **Move and resize grab.** Feed `drag::constrain` (`crates/gnoblin-wm/src/drag.rs:34`) from pointer deltas in `gnoblin-input`. Idea 9 depends on this. Core, M.
9. **Edge snapping during drag.** Reuse `SnapZone` (`snapping.rs:14`) inside the grab from idea 8. Zone rules stay in Lua or shell config. Core, S-M after 8.
10. **Frame composition.** Compose server-side frames, which the protocol does not do yet (`gnoblin_window_frame_v1.rs:33`). Core, L.

## Ideas to avoid

- **Copying COSMIC code.** COSMIC is `GPL-3.0-only` (`Cargo.toml`). Gnoblin is `GPL-2.0-or-later` (`Cargo.toml:84`). [INFERENCE] These do not combine cleanly. Take ideas, not code.
- **A toolkit inside the compositor.** COSMIC draws chrome with a UI toolkit inside the compositor. Gnoblin keeps its shell outside it (`design/native-ui-removal.md`). Keep it that way.
- **A per-window floating flag.** D25 removed it (`design/rust-compositor.md:1626`). COSMIC's per-window toggle would undo that decision.
- **Copying Sanctum.** Its source is closed, so there is nothing to copy. Its one-process model also conflicts with Gnoblin's shell split.
- **COSMIC's config stack.** `cosmic_config` ties config to libcosmic. Gnoblin uses Lua. Do not add a second config system.

## Needs Kieran

- **Float toggle in tiling.** Keep D25 (no float flag, toggle refused outside tiling), or allow a per-window float inside tiling as COSMIC does?
- **Owner of the move grab.** Core input (`gnoblin-input`) or the frame renderer? The table assumes core.
- **Anchored placement.** Should layouts receive the parent anchor, or should core place transients only?
- **Urgency.** Add an urgent state and signal, or keep deny-only?
- **Animation API.** `crates/gnoblin-config/src/document.rs:142` names `gnoblin.animation`, but no animation object appears in `window.toml` or the Lua host (grep). Add per-window transform and opacity setters with tweens in Lua, or keep motion in the shell?
- **Default placement.** Keep `FreeSpace` (`wm.rs:121`), or switch to an anchored cascade?
