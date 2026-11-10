# Cutover survey: steps 6, 7, 8 and 10

Survey for `design/node-api-refactor.md` section 7.2, rows 6, 7, 8 and 10. Read-only: no builds, no tests run.
Numbers are `wc -l` on the worktree at `489a0cace`. Where a count was not measured, it says so.

## 1. Measured sizes

| Crate or tree                                         | Source lines | Test lines | Notes                                                                                                                                                  |
| ----------------------------------------------------- | ------------ | ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `gnoblin-wm` `src/*.rs` (17 files)                    | 7048         | -          | `wm.rs` 3356, `rules.rs` 650, `stack.rs` 487, `arrange.rs` 497                                                                                         |
| `gnoblin-wm` `src/layout/` (8 files)                  | 1310         | -          | engine, placement, settle, transform, kind, error, fallback                                                                                            |
| `gnoblin-wm` `tests/` (5 files)                       | -            | 3404       | `failure_modes.rs` 1948 (80 `#[test]`), `layout_validation.rs` 431 (34 tests), `layout_engine.rs` 289 (17), `wm_layout.rs` 390 (5), `model.rs` 346 (2) |
| `gnoblin-layers` `src/`                               | 1987         | -          | `registry.rs` 885, `name.rs` 347, `order.rs` 226                                                                                                       |
| `gnoblin-wasm` `src/` + `build.rs`                    | 3633         | -          | `bindings.rs` 1582, `wit.rs` 570, `host.rs` 615; `tests/failure_modes.rs` 835                                                                          |
| `gnoblin-lua` `src/layout/`                           | 2418         | -          | `host.rs` 461, `lua.rs` 388, `encoding.rs` 326                                                                                                         |
| `gnoblin-lua` `tests/layout.rs`, `shipped_layouts.rs` | -            | 1114       | 424 + 690                                                                                                                                              |
| `data/lua/gnoblin/layouts/*.lua` (8 files)            | 1297         | -          | `strip` 398, `dwindle` 288, `carousel3d` 162                                                                                                           |
| `data/lua/gnoblin/shell/*.lua` (14 files)             | 1446         | -          | `init` 287, `tiling` 294, `strip` 206, `ui` 123                                                                                                        |
| `data/lua/gnoblin/node.lua`                           | 744          | -          | the stock class library                                                                                                                                |

Correction: the working notes had `gnoblin-wasm` at 2974 source lines. The measured total is 3633, plus `Cargo.toml` 44 and
`tests/` 835.

## 2. Dependents

Cargo dependencies, grepped across the workspace root and `crates/` (`extensions/` is excluded from the workspace and does
not depend on any of the three crates).

| Crate            | Depended on by                                                   | Rust use sites (files)                                                            |
| ---------------- | ---------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `gnoblin-wm`     | backend, compositor, lua, wayland, xwayland; root `Cargo.toml`   | compositor 6, backend 2, lua 8, wayland 23, xwayland 2, session 1                 |
| `gnoblin-layers` | compositor, config, wayland, wm, xwayland; root `Cargo.toml:267` | compositor 2, config 1, wayland 9, wm 2 (plus 20 uses inside `wm.rs`), xwayland 1 |
| `gnoblin-wasm`   | root `Cargo.toml:85` only                                        | none                                                                              |

Consumers that are not Rust dependencies, to check at each lane (not yet measured, see section 7):
config keys in `crates/gnoblin-config/schema/config.toml` and `crates/gnoblin-schema/schema/config.toml`; schema files
`crates/gnoblin-schema/schema/layers.toml`, `schema/effect.toml`, `schema/tree/effect.toml`; docs `docs/shell-api/grants.md`;
`gnoblinctl` (`cli.rs:386,456`, the capabilities verb).

## 3. Step 6: stacking and focus

**(a) Deletes.** `crates/gnoblin-wm/src/stack.rs` (487), `pointer_focus.rs` (244). The six focus and raise keys in
`config.toml:1797-1855` (already listed in 7.4). Step 6 is the smallest cut.

**(b) Consumers.** `gnoblin-wm/src/stack.rs` uses `gnoblin_layers` twice. `wm.rs` has 20 `gnoblin_layers` uses, so `wm.rs` is
edited here and deleted in step 7. The focus keys are read by config; the replacement is the stock `Window:on_press` and
`raise`.

**(c) Behaviour parity.** The stock shell has focus tests in `crates/gnoblin-lua/tests/lua/focus_test.lua` and
`shell_test.lua`. Mapping each `failure_modes` focus test to a named stock test is in section 5. Gap: the stock shell
has no pointer-focus test that matches `pointer_focus.rs`.

**(d) Lane.** L1, after the no-shell fallback move (section 6). Depends on: none. Parallel with L4 only if `wm.rs` is untouched.

## 4. Step 7: states, workspaces, rules, layouts

**(a) Deletes.** `gnoblin-wm` whole crate: `src/` 8358 lines including `layout/`, plus `tests/` 3404.
`crates/gnoblin-lua/src/layout/` (2418) and its two test files (1114). `data/lua/gnoblin/layouts/` (1297).
Total removed: about 16,600 lines.

**(b) Consumers outside the deleted code.**

- `gnoblin-compositor/src/layout.rs` (9 use sites, including `FloatingEngine` at line 411 and its test helpers at 463-497).
  `FloatingEngine` has no Lua VM, so it is deleted with step 5 (already done) only if its trait is replaced; this file must be
  rewritten, not just trimmed.
- `gnoblin-compositor/src/ipc/server.rs:2478` (`LayoutCommand`).
- `gnoblin-compositor/src/window_management.rs:15` (`FocusMode`, `NewWindowFocus`, `WmOptions`).
- `gnoblin-compositor/src/window_rules.rs:11,151` and `workspace_sets.rs:11`, with `tests/workspace_sets.rs`.
- `gnoblin-lua/src/shell_role.rs:19` uses `layout::{default_prefix, installed_module_dir}`. These two functions must move
  before `layout/` is deleted, or the shell VM cannot find its modules.
- `gnoblin-lua/src/host.rs` and `lib.rs` (6 and 2 uses of `layout`).
- Backend `render/compose.rs:81` (`WorkspaceDirection`) and `x11.rs:52` (`Wm`, `Effect`).
- Wayland: `state.rs` (15 uses), `server.rs` (7), `xdg_activation_v1.rs` (3), `xdg_shell.rs`, `xdg_dialog_v1.rs`,
  `zwlr_foreign_toplevel_management_v1.rs` (2), `xwayland.rs` (2), `frame.rs`.
- Wayland tests that drive the WM layout engine: `tests/owned/master.rs` (10), `tests/real_client.rs` (11),
  `tests/render.rs` (5), `tests/capture.rs` (7).
- Config: `config.toml` `layout` section and `workspace:set_layout`, replaced per 7.4.

**(c) Behaviour parity.** Each of the 80 `failure_modes` tests needs a named stock test or a deliberate drop. Measured
mapping in section 5. The `layout_engine` (17), `layout_validation` (34) and `wm_layout` (5) tests cover the layout
protocol, which the stock shell replaces with Lua arrange functions, so they map to tiling and strip tests, not one to one.

**(d) Lane.** L2, the largest lane. Port tests first, then delete. Depends on L1 and on the fallback move (section 6).
Conflicts with L3 and L4 (both edit `wm.rs`-linked files), so it cannot run in parallel with them.

## 5. Failure-mode mapping (gnoblin-wm `tests/failure_modes.rs`, 80 tests)

Only the names are checked here. Each row needs a stock test name or a drop decision; that is the next step.

| Old test group (from names)                                                    | Count (approx.) | Stock equivalent                           | Status  |
| ------------------------------------------------------------------------------ | --------------- | ------------------------------------------ | ------- |
| Focus: never lands on closed window; minimise moves focus; focus without raise | ~3              | `strip_test.lua` closing-column focus only | partial |
| Output unplug keeps windows valid; last output hides all                       | ~2              | none found                                 | gap     |
| Rules: reapply, new title/app_id match, merge order, last match wins           | ~5              | `rules_test.lua`                           | partial |
| Rule naming a missing workspace reports rejection; refused list keeps rules    | ~2              | none found                                 | gap     |
| Reserved space reapplies to maximised and snapped windows                      | ~2              | `snap_test.lua` (snap only)                | gap     |
| Fullscreen layer kept on top; workspace switch keeps fullscreen                | ~2              | none found                                 | gap     |

Counts are from names only; the full 80-name list was written to a scratch file and not yet mapped one by one.
Stock shell Lua tests: 23 `case()` titles in `tiling_test.lua`; the other eight `*_test.lua` files use a different
naming form, not yet listed.

Gaps listed above are real only as far as the names go: no stock test with a matching name was found. Whether the
behaviour is missing from the stock shell has not been checked against the Lua code.

## 6. No-shell fallback (risk to step 5 and step 7)

`crates/gnoblin-wm/src/wm.rs` lines 93-96 define `FALLBACK_CASCADE_OFFSET = 32` and `FALLBACK_CASCADE_STEPS = 32`. Lines
1307-1325 place windows in a cascade when no shell is running, from (32, 32), stepping 32 px per visible window on each
output and workspace (commit `489a0cace`).

Risk: deleting `gnoblin-wm` removes this. It must move to a small module in `gnoblin-compositor` first (lane L0), with a
test that reproduces the cascade positions, so the no-shell start still places windows.

## 7. Step 8: layers and decorations

**(a) Deletes.** `crates/gnoblin-layers` (1987 source, 14 Cargo). `gnoblin-wayland/src/modal.rs`, `gnoblin_window_frame_v1.rs`
(sole `GnoblinWindowFrameV1Frame` global in `access.rs:189,263,597`). `crates/gnoblin-schema/schema/layers.toml`.

**(b) Consumers.**

- `gnoblin-config/src/engine.rs:17` (`LayerName`).
- `gnoblin-compositor/src/ipc/portal.rs:34` (`InputPolicy`), `ipc/server.rs` (2 uses).
- `gnoblin-wayland`: `modal.rs:12` (`InputPolicy`), `zwlr_layer_shell_v1.rs` (3 uses), `server.rs`, `state.rs`,
  `frame.rs`, `gnoblin_window_frame_v1.rs:615-619` (`ToggleMaximise`, `Minimise`).
- Tests: `real_client.rs`, `session_lock.rs`, `virtual_keyboard.rs` (in `gnoblin-wayland`), `wm/tests/failure_modes.rs` (2).

**(c) Conflict.** `ClientRole::FrameRenderer` must survive step 10 per the assignment, but step 8 deletes
`gnoblin_window_frame_v1`, its only `Global` in `access.rs`. Decision needed: either `FrameRenderer` is deleted in step 8,
or the `view` role (`access.rs:262`, `Global::ViewSource` at `106,156,258,262,481`) takes over. This is a design question, not
an edit.

**(d) Lane.** L3, after L2 (`wm.rs` uses layers). Depends on L2 and on the 7.2 step 8 design decision above.

## 8. Step 10: trust edge

**(a) Deletes.** `crates/gnoblin-proto/src/capability.rs`, `grants.rs`; `crates/gnoblin-compositor/src/ipc/privileged.rs`;
`crates/gnoblin-compositor/tests/grants.rs`; `docs/shell-api/grants.md`; `crates/gnoblin-wasm` (3677 with Cargo and
tests); the `slot.rs` files in `gnoblin-model` and `gnoblin-service-core`; the D22 code (locate by grep before the lane).
Schema: `crates/gnoblin-schema/schema/tree/effect.toml` if it is only the grant surface (not checked).

**(b) Consumers.** `gnoblin-wasm` has no Rust dependents, only the root `Cargo.toml:85`. `grants.md` is linked from docs
(not measured). `input policy.rs` and `wayland/frame.rs`, `modal.rs` (modal overlaps step 8 and must wait for it).

**(c) Behaviour.** IPC tests on one owner connection (7.2 "green at the end"). No stock-shell equivalent is needed: the
grants are deleted, not moved.

**(d) Lane.** L5. Can run in parallel with L2 if it avoids `wayland/modal.rs` and the compositor layout files. It conflicts
with L3 on `modal.rs`.

## 9. Lane sequence

| Lane | Scope                                                                                                              | Depends on                       | Parallel with                | Risk                                                |
| ---- | ------------------------------------------------------------------------------------------------------------------ | -------------------------------- | ---------------------------- | --------------------------------------------------- |
| L0   | Move no-shell cascade (wm.rs 93-96, 1307-1325) and `shell_role` helpers out of `layout/` into `gnoblin-compositor` | none                             | L5                           | low, but blocks L2                                  |
| L1   | Step 6: delete `stack.rs`, `pointer_focus.rs`, the focus keys; stock focus parity                                  | L0                               | L5                           | medium: focus is in every test                      |
| L2   | Step 7: port the 80 failure modes to the stock shell, delete `gnoblin-wm`, `layout/`, `layouts/`                   | L0, L1                           | L5 only                      | high: largest, touches compositor, backend, wayland |
| L3   | Step 8: delete `gnoblin-layers`, `modal.rs`, `layers.toml`, `gnoblin_window_frame_v1.rs`; decide `FrameRenderer`   | L2                               | none                         | high: `wayland` state and a design decision         |
| L4   | Step 9: views, capture, effects                                                                                    | L2                               | L5 (after L3 for modal)      | medium                                              |
| L5   | Step 10: delete grants, privileged roles, D22, `gnoblin-wasm`, slots                                               | none (after L0 for `shell_role`) | L0, L1, L2 if files disjoint | low to medium                                       |

XR (`extensions/gnoblin-xr`): none of its Cargo.toml files depend on `gnoblin-wm`, `gnoblin-layers` or `gnoblin-wasm`. The
extension is outside the workspace (`exclude = ["extensions"]`), so these lanes cannot break it at build time. Untested: whether
XR uses the stock shell's layout or focus at runtime.

## 10. Risks

- No-shell fallback: lost if L2 runs before L0 (section 6).
- `FrameRenderer` vs step 8 (section 7c): cannot be resolved by the survey.
- `wm.rs` is the shared file of L1, L2 and L3; the three cannot run in parallel.
- Failure-mode gaps (section 5): not checked against the Lua code, so some gaps may be false.

## 11. Not verified (blockers for a complete survey)

- The full 80-name mapping to stock tests, one by one.
- The stock shell's Lua test names for eight of nine `*_test.lua` files.
- Config keys in `config.toml` and `schema/config.toml` beyond the six focus keys already in 7.4.
- `gnoblinctl` capabilities verb (`cli.rs:386,456`) and the schema for `effect.toml`/`layers.toml`.
- Goldens (`catalog.txt`), `xtask/src/deps.rs` allow lists (lines 41, 49, 120-129), `layout-allow.toml:74`, root `Cargo.toml` lines 56, 85, 87, 267 beyond the crate names above.
- Packaging lists (`*.spec`, Meson, CMake) for `window-frame` and `frame-renderer`.
