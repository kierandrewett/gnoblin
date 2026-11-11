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
| `gnoblin-wasm` `src/` + `build.rs` (deleted step 10A) | 3633         | -          | `bindings.rs` 1582, `wit.rs` 570, `host.rs` 615; `tests/failure_modes.rs` 835                                                                          |
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
| `gnoblin-wasm`   | root `Cargo.toml:85` only (crate deleted step 10A)               | none                                                                              |

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

## 5. Failure-mode mapping

Moved to section 12, which holds the group-level mapping and the keep, port or drop decision for each group. The earlier name-only gap list is superseded.

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

**(e) Capability and grant survey (CutL5, 2026-11-10).** Counts are lines in that file matching `Capabilit|capabilit|[Gg]rant`
(measured by `grep -c`); a bare "(n)" with no count was not measured. "Keep" means not part of the capability model (gesture
grants for step 11, Wayland roles, portal consent UI, Lua grant names). No row is deleted until its gate is green.

| File                                                                                                                                                  | Symbol(s)                                                                                                                                                                                                                                                                                                                      | Action                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ----------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/gnoblin-proto/src/capability.rs` (295 lines)                                                                                                  | `Capability` enum, `Capabilities` bitset, `of/from_bits/NONE/ALL`                                                                                                                                                                                                                                                              | Delete. Entangled: `Capabilities` is the handshake wire field (`handshake.rs`), re-exported by `lib.rs`, used by `error.rs`.                                                                                                                                                                                                                                                                                                |
| `crates/gnoblin-proto/src/handshake.rs` (43)                                                                                                          | `Handshake.capabilities`, accept/hello encode                                                                                                                                                                                                                                                                                  | Remove field and its bytes (B3, last). Entangled: changes the 20-byte fixed handshake; every reader and writer moves together (`gnoblin-client` transport, compositor `server.rs`).                                                                                                                                                                                                                                         |
| `crates/gnoblin-proto/src/grants.rs` (480 lines, 130 hits)                                                                                            | `Grant`, `GrantsError`, `encode/decode/read/write_grants`, `MAX_GRANTS`                                                                                                                                                                                                                                                        | Extension-namespace grants frame, not the capability bitset. Entangled: `error.rs` (`CodecError::Grants`), `lib.rs` re-exports, compositor `src/extensions.rs` (24), `gnoblin-client` `client.rs` (23) and `loopback.rs` (17), `gnoblin-config/src/extensions.rs` (13), `tests/extensions.rs` (16), `tests/extension_scripts.rs` (9), `tests/config_extensions.rs` (20). Separate step (B2b), decided with the D22 removal. |
| `crates/gnoblin-compositor/src/ipc/server.rs` (103)                                                                                                   | `Capabilities` on connection, `required_capability` (l.2805), gates at l.1141-1143 (`OfferRequest`/`RetrieveSecret`), 1209 (`OfferRequest`), 1405 (`CommitConfig`, other-connection check), 1497 (`ReadConfig`, the owner test), 2029 (`RegisterLayer`), 2194 and 2251 (`WindowControl`), 2805-2812 (member to capability map) | Replace each `contains(Capability::X)` with the owner-connection predicate (the `is_owner` test at l.1497). Each gate gets a test: non-owner refused `not-owner`, owner accepted. One commit per IPC area.                                                                                                                                                                                                                  |
| `crates/gnoblin-compositor/src/ipc/privileged.rs` (302 lines, 96 hits)                                                                                | privileged roles, slot claims, `Capability` grants per role                                                                                                                                                                                                                                                                    | Delete the capability-slot parts. Keep the Wayland `ClientRole` variants and pid-promise mechanism (`gnoblin-wayland/src/access.rs`, 3 hits) and `Global::ViewSource`. Entangled with `server.rs` role lookup and `ipc/portal.rs`.                                                                                                                                                                                          |
| `crates/gnoblin-compositor/src/ipc/portal.rs` (34)                                                                                                    | `Capability::PortalUi` / `PortalInput` checks                                                                                                                                                                                                                                                                                  | Replace with owner predicate. Entangled: `portal/validate.rs`, `portal/outcome.rs` (counts not measured).                                                                                                                                                                                                                                                                                                                   |
| `crates/gnoblin-compositor/src/ipc/requests.rs` (25)                                                                                                  | capability checks on request kinds                                                                                                                                                                                                                                                                                             | Replace with owner predicate.                                                                                                                                                                                                                                                                                                                                                                                               |
| `crates/gnoblin-compositor/tests/privilege.rs` (33), `tests/portal.rs` (12), `tests/grants.rs` (27), `tests/eis.rs` (21), `tests/support/mod.rs` (18) | `Capability`, `Capabilities`, grant setup                                                                                                                                                                                                                                                                                      | Rewrite to owner connection. `tests/grants.rs` deletes with the grant frame.                                                                                                                                                                                                                                                                                                                                                |
| `crates/gnoblin-model/src/slot.rs` (54) and `crates/gnoblin-service-core/src/slot.rs`                                                                 | `CapabilitySlot`                                                                                                                                                                                                                                                                                                               | Delete with the slot claim path. `CapabilitySlot` is also used by `gnoblin-service-core` `agent.rs` (12), `tests/failure_modes.rs`, `gnoblin-service-location/tests/geoclue.rs`, `gnoblin-service-polkit` `src/lib.rs` and `tests/dbus.rs`, `gnoblin-services` `src/keyring.rs` and `src/role.rs`. Each consumer moves before the type is deleted.                                                                          |
| `crates/gnoblin-authority/src/` (`authority.rs` 61, `tests.rs` 40, `model.rs` 15, `effect.rs` 8, `event.rs` 10)                                       | `Capability` (its own enum for authority decisions)                                                                                                                                                                                                                                                                            | Not surveyed in depth. The authority crate may be a second capability model. Do not touch in B2/B3 until surveyed.                                                                                                                                                                                                                                                                                                          |
| `crates/gnoblin-input-eis/src/`, `tests/eis.rs`, `tests/eis_real.rs`                                                                                  | `Capability::InputInjection`, `Keyboard`, `Pointer`, `Touch`                                                                                                                                                                                                                                                                   | EIS input-injection grants. Not the IPC gate; separate decision. Keep until the EIS grant design is settled.                                                                                                                                                                                                                                                                                                                |
| `crates/xdg-desktop-portal-gnoblin/src/consent.rs` (84)                                                                                               | portal consent `Capture`, `CaptureScreen`                                                                                                                                                                                                                                                                                      | Portal consent UI, not an IPC capability. Keep (outside step 10).                                                                                                                                                                                                                                                                                                                                                           |
| `crates/gnoblin-lua/src/host.rs`, `crates/gnoblin-config`                                                                                             | Lua grant names                                                                                                                                                                                                                                                                                                                | Keep unless the Lua grant surface is removed in step 11.                                                                                                                                                                                                                                                                                                                                                                    |
| `docs/shell-api/grants.md`                                                                                                                            | gesture grants (user-action proofs)                                                                                                                                                                                                                                                                                            | Keep: step 11 (gesture chords).                                                                                                                                                                                                                                                                                                                                                                                             |
| `design/*.md`, `design/docs-parity/*`                                                                                                                 | design notes and parity allow-list                                                                                                                                                                                                                                                                                             | Update when the code changes (docs-parity gate).                                                                                                                                                                                                                                                                                                                                                                            |
| `CHANGELOG.md`                                                                                                                                        | history                                                                                                                                                                                                                                                                                                                        | Leave.                                                                                                                                                                                                                                                                                                                                                                                                                      |

**Status at this table.** Deleted so far in this step: the `gnoblinctl capabilities` verb (`f0410f5d6`) and the `gnoblin-wasm` crate (`878a55586`). Remaining: B2 (grant frame, entangled with extensions and the D22 tier), B3 (IPC gates, one commit per area), then the handshake field removal (after B3, so no reader still expects the bits).

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
- Failure-mode gaps: section 12 replaces the name-only gap list. The Lua side is not checked test by test, so some Drop or Port verdicts may change.

## 11. Not verified (blockers for a complete survey)

- Test-by-test Lua mapping for the 76 placed names, the 4 unplaced names, and the failure-mode names beyond the first 100 in the list read.
- Lua case titles for the `*_test.lua` files other than `tiling_test.lua` are counted in section 12, not listed.
- Config keys in `config.toml` and `schema/config.toml` beyond the six focus keys already in 7.4.
- `gnoblinctl` capabilities verb (`cli.rs:386,456`) and the schema for `effect.toml`/`layers.toml`.
- Goldens (`catalog.txt`), `xtask/src/deps.rs` allow lists (lines 41, 49, 120-129), `layout-allow.toml:74`, root `Cargo.toml` lines 56, 85, 87, 267 beyond the crate names above.
- Packaging lists (`*.spec`, Meson, CMake) for `window-frame` and `frame-renderer`.

## 12. Failure-mode mapping

Group-level mapping of the tests in `crates/gnoblin-wm/tests/failure_modes.rs` to the stock shell. Names come from the file; the Lua side is not mapped test by test. `Keep` means the behaviour survives in the object model. `Port` means the assertion moves to a stock Lua test. `Drop` means the behaviour goes. Design citations are to `design/node-core.md` and `design/node-api-refactor.md`.

| Group                            | Tests (count)                                                                                                                                                                                                                | Stock equivalent                             | Decision                     | Evidence                                                                                 |
| -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- | ---------------------------- | ---------------------------------------------------------------------------------------- |
| Focus and activation             | never lands on closed window; minimise moves focus; focus without raise; denied activation; hidden activation refused; no-op events (6)                                                                                      | `focus_test.lua` (12 cases, not mapped)      | Port                         | request signals incl. activate, `node-api-refactor.md:666`                               |
| Output unplug and placement      | unplug keeps valid output; last output hides all; no workspace or output; new windows follow focused output; output move reclamps floating (5)                                                                               | none found                                   | Keep                         | unplug keeps rect, `node-core.md:633`; the window stays as a node                        |
| Identity                         | duplicate and unknown identities rejected (1)                                                                                                                                                                                | none                                         | Keep                         | owner not verified                                                                       |
| Rules                            | no reapply; title match; app_id match; merge in list order; last match wins; missing workspace rejection; refused list keeps rules (7)                                                                                       | `rules_test.lua` (9 cases, not mapped)       | Port; `layer` match deleted  | `node-api-refactor.md:183`                                                               |
| Secure                           | secure match; non-match not secure; reload clears; title toggles; closed secure leaves no record (5)                                                                                                                         | none                                         | Keep                         | `secure` stays (`node-core.md:124`); mask treatment deleted (`node-api-refactor.md:208`) |
| Appearance                       | opacity refused; corner radius refused; appearance rule announces (3)                                                                                                                                                        | none                                         | Port; opacity has one writer | four writers fold, `node-api-refactor.md:189`                                            |
| Fullscreen and workspace switch  | raising keeps fullscreen on top; switching keeps fullscreen intact (2)                                                                                                                                                       | none                                         | Rewrite as tree order        | `StackLayer` deleted, `node-api-refactor.md:176`, `:2165`                                |
| Workspace removal                | removing workspace moves windows and keeps focus; active neighbour refocused (2)                                                                                                                                             | none                                         | Keep                         | windows go to root, not destroyed, `node-core.md:627`                                    |
| Reserved space                   | reapply to maximised; reapply to snapped (2)                                                                                                                                                                                 | `snap_test.lua` (snap only)                  | Drop from WM                 | `Desk:arrange` docks `Bar`, `node-api-refactor.md:1553`                                  |
| Snap, drag, maximise, fullscreen | snap and unsnap; snap maximised floats; snap refused without edge tiling; drag constrained; drag may leave strip; drag maximised floats; fullscreen restore; maximise refused (8)                                            | `snap_test.lua` (6 cases), `tiling_test.lua` | Port                         | drag 7 and snap 6 Lua cases, not mapped                                                  |
| Dialog and transient             | dialog above parent; rules do not move dialog; transient cannot leave parent; moving parent takes transient; transient follows to output (5)                                                                                 | `floating_test.lua` (14 cases)               | Port                         | not mapped                                                                               |
| Workspace direction              | switch reports direction (1)                                                                                                                                                                                                 | `workspaces` script                          | Port                         | not mapped                                                                               |
| Moves                            | move to workspace; unknown rejected; current is no-op; focused moved away hands focus; unknown output rejected; move reshapes to output; maximised fills new area; fullscreen not moved; transient follows (7)               | `strip_test.lua` (8 cases)                   | Port                         | not mapped                                                                               |
| Sets                             | unknown set refused; empty set refused; undeclared workspace refused; removed workspace refused; duplicate names refused; empty name refused; two entries for one output refused; unknown output falls back with warning (8) | none                                         | Keep as Lua table validation | sets are a Lua table, `node-api-refactor.md:185`, `:1596`                                |
| No-shell cascade                 | cascade from 32,32; three distinct cascades (2)                                                                                                                                                                              | none                                         | Drop                         | F1, `node-core.md:589`; `fallback.rs:15-34`                                              |
| Pointer and hover                | click mode ignores motion; hover keeps focus; hover strict clears; motion cancels rest focus; rest focus after settle; closed window cancels rest focus; held button ignores focus change; rest focus not while held (8)     | `focus_test.lua`                             | Port                         | not mapped                                                                               |
| Raise and auto-raise             | click on buried window raises; raise off keeps stacking; leaving cancels auto-raise; closing armed window cancels (4)                                                                                                        | `focus_test.lua`                             | Port                         | not mapped                                                                               |

Placed: 76 names. Not placed: the failure-mode names beyond the first 100 in the list read, including the surface-cap test at `failure_modes.rs:1918`. Its per-provider cap of 32 has no node-tree equivalent; the node cap is 65536 (`node-core.md:304`), so that test is replaced by a node-cap test, not ported. The `model.rs` (2), `wm_layout` (5) and inline (93, grouped) tests are counted only.

## 13. Port lanes

Groups from section 12 mapped to the lanes in section 9:

| Lane | Groups                                                                                                                          | Notes                                                                                                 |
| ---- | ------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| L0   | No-shell cascade (2)                                                                                                            | Conflict: L0 moves the cascade out of `layout/`, but section 12 drops it (F1). Decide before L0 runs. |
| L1   | Focus and activation (6), pointer and hover (8), raise and auto-raise (4)                                                       | Stock focus parity first; port `focus_test.lua` cases before deleting `pointer_focus.rs`.             |
| L2   | Output, identity, rules, secure, appearance, fullscreen, removal, reserved space, snap and drag, dialog, direction, moves, sets | Largest lane. Port each group to stock Lua tests, then delete `gnoblin-wm`.                           |
| L3   | None directly                                                                                                                   | Layers and modal are not in the failure-mode groups.                                                  |
| L4   | None directly                                                                                                                   | Views and capture are not in the failure-mode groups.                                                 |
| L5   | None directly                                                                                                                   | Grant and config removal are not in the failure-mode groups.                                          |

### Ported: snap and maximise (lane PortSnap, steps 1-2)

Old tests in `crates/gnoblin-wm/tests/failure_modes.rs`, mapped to the stock shell (`crates/gnoblin-lua/tests/lua/snap_test.lua`). Later steps append their rows here.

| Old test                                                                | Stock test (file:case)                                                                                                     |
| ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `snapping_and_unsnapping_restore_the_floating_rectangle`                | `snap_test.lua`: snapping tiles a window to an edge and unsnapping restores its floating rectangle                         |
| `snapping_a_maximised_window_floats_it_and_unsnapping_restores_it`      | `snap_test.lua`: snapping a maximised window floats it first, and unsnapping restores the floating rectangle               |
| `snapping_is_refused_when_edge_tiling_is_off`                           | `snap_test.lua`: snapping is refused when edge tiling is off and leaves the window where it was                            |
| `maximising_a_fullscreen_window_is_refused`                             | `snap_test.lua`: a client maximise request fills the work area, and is refused while the window is fullscreen              |
| `a_drag_is_constrained_to_the_work_area`                                | `drag_test.lua`: under work_area a dragged window is pulled back inside the work area                                      |
| `a_drag_may_leave_a_visible_strip_when_configured`                      | `drag_test.lua`: under minimum_strip a window may leave the area but 75 pixels of it stay inside                           |
| `dragging_a_maximised_window_floats_it_and_fullscreen_refuses_the_drag` | `drag_test.lua`: a fullscreen window refuses a drag, and a maximised window floats first                                   |
| `fullscreen_returns_to_the_maximised_or_floating_rectangle`             | `snap_test.lua`: leaving fullscreen returns the floating rectangle, and a maximised window stays maximised                 |
| `reserved_space_reapplies_to_maximised_windows`                         | `shell_test.lua`: a maximised window refits when a bar reserves space at the top (failure mode 9, maximised)               |
| `reserved_space_reapplies_to_snapped_windows`                           | `shell_test.lua`: a snapped window refits when a bar reserves space at the top (failure mode 9, snapped)                   |
| `a_dialog_opens_above_its_parent_on_the_parents_workspace`              | `floating_test.lua`: a dialog is centred over its parent and is a child of it, so the compositor stacks it above           |
| `workspace_rules_do_not_move_a_dialog_away_from_its_parent`             | `floating_test.lua`: a window rule that names another workspace does not take a dialog away from its parent                |
| `switching_workspace_keeps_the_fullscreen_window_intact`                | `shell_test.lua`: a workspace switch keeps a fullscreen window fullscreen and on its workspace when that workspace returns |
| `raising_keeps_the_fullscreen_layer_on_top`                             | `shell_test.lua`: raising a window keeps a fullscreen sibling above it, and a fullscreen window stays on top when raised   |

Drops: none in this group. The old `OutputReserved` event (gnoblin-wm `event.rs`) has no stock schema counterpart: a bar is the reserved space, and `Desk:arrange` refits maximised and snapped windows when a bar changes the work area.

### Ported: focus, pointer and raise (lane PortFocus)

Old tests in `crates/gnoblin-wm/tests/failure_modes.rs`, mapped to the stock shell (`crates/gnoblin-lua/tests/lua/shell_test.lua`, focus policy in `shell/focus.lua`). Client activation requests reach the shell as `activate` (`tree_frame.rs`, `an_activation_request_is_named_for_the_shell`).

| Old test                                                          | Stock test (`shell_test.lua`) or status                                                                                 |
| ----------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `focus_never_lands_on_a_closed_window`                            | `a window closed while focused drops focus`; `a closed window cancels its pending rest focus`                           |
| `minimising_the_focused_window_moves_focus_away`                  | OPEN: the stock shell has no minimise request or minimised property. Needs a compositor event (schema gap). Not ported. |
| `focus_without_raise_leaves_the_stack_alone`                      | `raise_on_click off: a press focuses but does not raise`                                                                |
| `a_denied_activation_reports_the_decision_without_changing_focus` | OPEN: the denial is decided in Rust (`xdg_activation_v1.rs`, `refuse_activation`) and has no stock test.                |
| `activating_a_hidden_window_is_refused`                           | `activating a window on a hidden workspace is refused and leaves focus alone`                                           |
| `no_op_events_produce_no_effects`                                 | Partial: `activating the focused window again changes nothing`                                                          |
| `click_mode_ignores_pointer_motion`                               | `click mode: pointer motion does not move focus`                                                                        |
| `hover_keeps_focus_when_the_pointer_crosses_a_gap`                | `hover: entering a window focuses it and leaving keeps focus`                                                           |
| `hover_strict_clears_focus_when_the_pointer_crosses_a_gap`        | `hover-strict: leaving the focused window clears focus`                                                                 |
| `motion_cancels_a_pending_rest_focus`                             | `a motion restarts the rest timer`                                                                                      |
| `a_pointer_resting_over_a_window_focuses_it_once_settled`         | `a pointer rest focuses the window; a leave before the rest cancels it`                                                 |
| `a_closed_window_cancels_its_pending_rest_focus`                  | `a closed window cancels its pending rest focus`                                                                        |
| `a_focus_change_while_a_button_is_held_is_ignored`                | `a focus change while a button is held is ignored`                                                                      |
| `a_rest_focus_does_not_fire_while_a_button_is_held`               | `a rest does not focus while a button is held`                                                                          |
| `a_click_on_a_buried_window_raises_it`                            | `a press on a buried window raises it`                                                                                  |
| `a_click_with_raise_on_click_off_keeps_the_stacking`              | `raise_on_click off: a press focuses but does not raise`                                                                |
| `auto_raise_is_cancelled_by_leaving_the_window`                   | `a leave cancels an armed auto-raise`                                                                                   |
| `closing_the_armed_window_cancels_its_auto_raise`                 | `closing an armed window cancels its auto-raise`                                                                        |

### Ported: window rules and workspace sets (lane PortRules)

Rules: stock cases are in `rules_test.lua`. Sets: stock cases are in `sets_test.lua`. Status is one of ported, partial (stated), drop (design line given), or open (no stock case yet).

| Old test                                                      | Stock test (file:case) or status                                                                                                    |
| ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `rules_do_not_reapply_while_the_same_rules_match`             | `rules_test.lua`: re-applying unchanged rules writes nothing to the core                                                            |
| `a_title_that_newly_matches_a_rule_applies_it`                | `rules_test.lua`: a title change re-evaluates the rules: secure follows the new title                                               |
| `an_app_id_that_newly_matches_a_rule_applies_it`              | Partial: `rules_test.lua`: a rule that stops matching clears secure (the stop direction only; the newly-matching direction is open) |
| `a_re_match_merges_all_matching_rules_in_list_order`          | `rules_test.lua`: rules merge in list order: the last matching rule wins for each field, the others are kept                        |
| `contradicting_rules_settle_on_the_last_match`                | `rules_test.lua`: the last matching rule wins for a field                                                                           |
| `a_rule_naming_a_missing_workspace_reports_a_rejection`       | Open: no stock check that a rule's workspace exists                                                                                 |
| `a_refused_rules_list_leaves_the_rules_in_force`              | Partial: `rules_test.lua`: a malformed pattern is a compile error; the old rules staying in force on refusal is open                |
| `reserved_space_reapplies_to_maximised_windows`               | `rules_test.lua`: a maximised rule fills the work area, not only sets the flag                                                      |
| `a_secure_rule_marks_the_matching_window_secure`              | `rules_test.lua`: a rule that stops matching clears secure (asserts the set first)                                                  |
| `a_non_matching_window_is_not_secure`                         | `rules_test.lua`: a title change re-evaluates the rules (asserts secure false before the match)                                     |
| `an_out_of_range_opacity_is_refused_and_the_old_rules_stay`   | Partial: `rules_test.lua`: a rule's opacity must be a number in [0, 1]; the old rules staying is open                               |
| `a_negative_corner_radius_is_refused`                         | DROP: corners is refused at load (`rules_test.lua`: corners is refused at load). Design: the stock shell draws blur and shadow only |
| `a_matching_appearance_rule_announces_opacity_and_radius`     | Partial: `rules_test.lua`: an opacity rule sets the window's opacity. The radius half is DROP (see above)                           |
| `reloading_without_the_secure_rule_clears_secure`             | `rules_test.lua`: a rule that stops matching clears secure                                                                          |
| `a_title_change_toggles_secure_with_the_match`                | `rules_test.lua`: a title change re-evaluates the rules: secure follows the new title                                               |
| `a_closed_secure_window_leaves_no_secure_record`              | DROP: design: `secure` is a node property in Lua, so a closed node holds no record; there is no compositor-side registry to leak    |
| `activating_an_unknown_set_is_refused_and_changes_nothing`    | `sets_test.lua`: activating an unknown set is refused and changes nothing                                                           |
| `a_set_with_no_entries_is_refused_and_the_previous_sets_stay` | `sets_test.lua`: a set with no entries is refused and the previous sets stay                                                        |
| `a_set_naming_an_undeclared_workspace_is_refused_as_a_whole`  | `sets_test.lua`: a set naming an undeclared workspace is refused as a whole                                                         |
| `a_set_whose_workspace_is_removed_refuses_to_activate`        | `sets_test.lua`: a set whose workspace is removed refuses to activate                                                               |
| `duplicate_set_names_are_refused`                             | `sets_test.lua`: duplicate set names are refused                                                                                    |
| `an_empty_set_name_is_refused`                                | `sets_test.lua`: an empty set name is refused                                                                                       |
| `two_entries_for_one_output_in_a_set_are_refused`             | `sets_test.lua`: two entries for one output in a set are refused                                                                    |
| `activating_a_set_twice_is_idempotent`                        | `sets_test.lua`: activating the live set again changes nothing                                                                      |
| `a_set_shows_each_named_workspace_on_its_own_output`          | `sets_test.lua`: a set shows each named workspace on its own output                                                                 |

Stock-only (no old test): `sets_test.lua`: an unknown output falls back to the primary with a warning.

Design line for rules: `corners` is refused at load. The stock shell draws blur and shadow only, so the carousel depth fade is also DROP.

### Ported: outputs, dialogs, moves and workspace removal (lane PortWorkspaces2)

Old tests in `crates/gnoblin-wm/tests/failure_modes.rs`, mapped to the stock shell (`crates/gnoblin-lua/tests/lua/shell_test.lua`, moves and removal in `shell/init.lua`: `Window:move_to`, `Window:move_to_output`, `Desk:remove_workspace`, `Screen:on_ready` and `Screen:on_destroyed`).

| Old test                                                            | Stock test (`shell_test.lua`) or status                                                                                                                                                                                         |
| ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `unplugging_an_output_keeps_windows_on_a_valid_output`              | `unplugging one of two outputs moves its windows to the other output`                                                                                                                                                           |
| `unplugging_the_last_output_hides_every_window`                     | `unplugging the last output keeps its windows hidden and drops focus`; `reconnecting an output adopts the parked windows and focuses one`                                                                                       |
| `an_output_move_reclamps_floating_windows`                          | `moving a window to another output reshapes it into that output's work area`                                                                                                                                                    |
| `a_transient_cannot_leave_its_parents_workspace_on_its_own`         | `a dialog does not move to another workspace on its own`                                                                                                                                                                        |
| `moving_a_parent_to_another_workspace_takes_its_transient_with_it`  | `moving a parent window takes its dialog with it`                                                                                                                                                                               |
| `a_transient_follows_its_parent_to_another_output`                  | `a dialog follows its parent to another output`                                                                                                                                                                                 |
| `moving_to_the_current_workspace_changes_nothing`                   | `moving a window to its current workspace is a no-op`                                                                                                                                                                           |
| `moving_an_unknown_window_or_to_an_unknown_workspace_is_rejected`   | Partial: `moving a window to a workspace that is not in any desk is rejected`. An unknown window: OPEN (the Lua shell cannot name one)                                                                                          |
| `moving_a_window_to_an_unknown_output_is_rejected`                  | `moving a window to an output that is not a screen is rejected`                                                                                                                                                                 |
| `moving_a_window_to_another_output_reshapes_it_into_that_work_area` | `moving a window to another output reshapes it into that output's work area`                                                                                                                                                    |
| `a_maximised_window_fills_the_work_area_of_its_new_output`          | `a maximised window fills the new output when it moves`                                                                                                                                                                         |
| `a_fullscreen_window_is_not_moved_between_outputs`                  | Partial: `a fullscreen window does not move to another workspace`. `move_to` refuses fullscreen for an output move too, but no output-only case                                                                                 |
| `removing_a_workspace_moves_its_windows_and_keeps_focus_valid`      | Partial: `removing a workspace moves its windows to the neighbour before it`. Focus on the moved window: `removing the active workspace shows its neighbour and refocuses the moved window`; focus on an inactive removal: OPEN |
| `removing_the_active_workspace_shows_its_neighbour_and_refocuses`   | `removing the active workspace shows its neighbour and refocuses the moved window`                                                                                                                                              |
| (no old test)                                                       | Stock-only: `the last workspace of a desk cannot be removed`                                                                                                                                                                    |

Drops: none in this group. Workspace removal has no compositor event. The stock shell removes a workspace with `Desk:remove_workspace`, which moves its top-level windows to the neighbour before it (or after it, for the first). An output unplug is the `output.removed` lifecycle, handled by `Screen:on_destroyed`: the Desk's windows move to another output's Desk, or, with no output left, the Desk is parked hidden and focus is dropped until `Screen:on_ready` adopts it on the next output.

Open from this group: `a_window_needs_a_workspace_and_an_output` and `new_windows_open_on_the_focused_windows_output` have no stock case yet.

## 14. gnoblin-wm consumer map

Every use of `gnoblin_wm` or `gnoblin-wm` outside `crates/gnoblin-wm`, by consumer crate. Lines are from grep at lane HEAD `15db87384`. Replacement is one of: P (node property write), R (request or signal the shell answers), C (compositor-owned seat or policy value), or D (delete, with the design line). Rows marked UNVERIFIED were not reopened in this pass; section 14.7 lists them.

### 14.1 gnoblin-backend

| File:line                              | Symbol               | Purpose                                | Replacement                                                                   |
| -------------------------------------- | -------------------- | -------------------------------------- | ----------------------------------------------------------------------------- |
| `crates/gnoblin-backend/Cargo.toml:70` | dependency edge      | shared effect vocabulary               | D with the crate edge (W6)                                                    |
| `src/render/compose.rs:81`             | `WorkspaceDirection` | workspace-switch direction             | D: the switch is a Lua tween on the Desk node (`node-api-refactor.md` step 7) |
| `src/render/compose.rs:1861`           | doc comment only     | paint order "as gnoblin-wm reports it" | D: comment; paint order is tree order                                         |
| `src/x11.rs:52`                        | `Effect`, `Wm`       | X11 window table applies WM effects    | R: X11 windows become Window nodes; the effect vocabulary D                   |
| `src/x11.rs:5-19,126`                  | doc comments         | describe the WM boundary               | D with the code                                                               |

### 14.2 gnoblin-compositor

| File:line                                  | Symbol                                                                                                                                                               | Purpose                                     | Replacement                                                                  |
| ------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------- | ---------------------------------------------------------------------------- |
| `Cargo.toml:19`                            | dependency edge                                                                                                                                                      |                                             | D (W6)                                                                       |
| `src/layout.rs:33-35`                      | `Camera`, `Generation`, `Handled`, `Job`, `LayoutEffect`, `LayoutEngine`, `LayoutFailure`, `LayoutOutcome`, `Reply`, `Ticket`                                        | adapter for Lua and floating layout engines | D with step 7 (`Desk:arrange`)                                               |
| `src/layout.rs:409-416`                    | `FloatingEngine::kind`, `extent` (`Floating`, `Output`)                                                                                                              | built-in floating engine identity           | D: floating placement is `shell/floating.lua`                                |
| `src/layout.rs:461-499`                    | `Job`, `LayoutEngine`, `Params`, `Slot`, `Collected`, `arrange` helper                                                                                               | test harness for arrange                    | D with step 7; replaced by a Lua layout test                                 |
| `src/window_management.rs:15`              | `DragConstraint`, `FocusMode`, `NewWindowFocus`, `WmOptions`                                                                                                         | reads `config.wayland.wm.*`                 | C for surviving keys (15.2); D rest                                          |
| `src/window_management.rs:157-267` (tests) | config fields `edge_tiling`, `drag_constraint`, `new_window_focus`, `focus_mode`, `focus_change_on_pointer_rest`, `raise_on_click`, `auto_raise`, `auto_raise_delay` | option tests                                | P/C: the same values become seat properties (`node-api-refactor.md` step 11) |
| `src/window_rules.rs:11`, `:151`           | `Match`, `Pattern`, `Rule`, `RuleFields`, `WindowProtocol`, `WorkspaceSelector`, `validate_list`                                                                     | converts config rules into WM rules         | D: rules are Lua `rules` (`node-api-refactor.md:183`)                        |
| `src/workspace_sets.rs:11`                 | `WorkspaceSet as WmWorkspaceSet`, `WorkspaceSetEntry`                                                                                                                | converts config sets                        | D: sets are a Lua table (`node-api-refactor.md:185`, `:1596`)                |
| `src/ipc/server.rs:2458`                   | `gnoblin_wm::layout::LayoutCommand`                                                                                                                                  | IPC layout command                          | D with step 7                                                                |
| `tests/workspace_sets.rs:13`               | `WorkspaceSet`, `WorkspaceSetEntry`                                                                                                                                  | set tests                                   | D: ported to `sets_test.lua` (section 13)                                    |

### 14.3 gnoblin-lua

| File:line                                           | Symbol                                                                                                                           | Purpose                                   | Replacement                                           |
| --------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------- | ----------------------------------------------------- |
| `Cargo.toml:40`                                     | dependency edge                                                                                                                  |                                           | D (W6)                                                |
| `src/layout/engine.rs:1`, `:15-17`                  | `LayoutEngine`, `Generation`, `Job`, `LayoutExtent`, `LayoutFailure`, `LayoutKind`, `LayoutOutcome`, `Params`, `Reply`, `Ticket` | Lua layout registered as a WM engine      | D with step 7 (`shell/tiling.lua`, `shell/strip.lua`) |
| `src/layout/lua.rs:10`                              | `Arg`, `LayoutExtent`, `LayoutKind`, `Reply`                                                                                     | Lua return conversion                     | D with step 7                                         |
| `src/layout/mod.rs:5-8`, `:45`                      | `LayoutEngine`, `LayoutFailure`, `Arg`                                                                                           | doc links; failure falls back to floating | D with step 7                                         |
| `src/layout/reply.rs:1-10`                          | `Camera`, `Handled`, `LayoutEffect`, `Placement`, `Reply`, `Transform3`                                                          | Lua return validation                     | D with step 7                                         |
| `tests/layout.rs:15`, `tests/shipped_layouts.rs:36` | `layout::{…}`                                                                                                                    | layout tests                              | D: `strip_test.lua`, `tiling_test.lua`                |

`src/layout/snapshot.rs` also imports from the crate (`LayoutOutput`, per the earlier survey); the line was not re-read here (UNVERIFIED).

### 14.4 gnoblin-wayland

The main consumer. `state.rs` owns the WM and applies `Event`s to it; the `Effect` arms are its reply channel.

| File:line                                                                              | Symbol                                                                                                                                                                                                                    | Purpose                          | Replacement                                                         |
| -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------- | ------------------------------------------------------------------- |
| `src/state.rs:1432-1495`                                                               | `Effect` arms: Configure, SetVisible, SecureChanged, Appearance, Stack, Focus, ActivationDenied, WorkspaceShown, WorkspaceSetOutputFallback, WorkspaceSetActivated, RuleRejected, Rejected, PlacementTarget, LayoutFailed | applies WM decisions to surfaces | D one arm at a time: P (node property write) or R (shell request)   |
| `src/state.rs:883`                                                                     | `Event::WorkspaceAdded`                                                                                                                                                                                                   | workspace lifecycle              | R: `Desk` creation in the shell                                     |
| `src/state.rs:959-970`                                                                 | `Event::MoveResize`, `MoveToWorkspace`, `MoveToOutput`                                                                                                                                                                    | client and shell move requests   | P: `Window:move_to`, `Window:move_to_output`                        |
| `src/state.rs:1004`, `:1035`                                                           | `Event::WorkspaceSetActivated`                                                                                                                                                                                            | set activation                   | R: `sets_test.lua` contract                                         |
| `src/state.rs:1320`, `:1360`, `:1365`, `:1418`                                         | `OutputDisconnected`, `OutputConnected`, `OutputIdentified`, `OutputMoved`                                                                                                                                                | output lifecycle                 | R: `output.removed`, `Screen:on_destroyed`, `Screen:on_ready`       |
| `src/state.rs:1812`, `:1857`                                                           | `Event::WindowOpened`, `WindowClosed`                                                                                                                                                                                     | window lifecycle                 | R: node creation and destruction (`node-core.md:633`)               |
| `src/state.rs:1900`, `:1904`                                                           | `Event::WindowAppIdChanged`, `WindowTitleChanged`                                                                                                                                                                         | rule re-evaluation               | P: `app_id`, `title` properties; rules re-run (`rules_test.lua`)    |
| `src/state.rs:1942`                                                                    | `Event::ActivationRequested`                                                                                                                                                                                              | xdg-activation                   | R: `activate` request (`node-api-refactor.md:666`)                  |
| `src/state.rs:1981`, `:2013`, `:2108`                                                  | `rebuild_layout`, `rebuild_all_layouts`                                                                                                                                                                                   | layout re-arrange                | D with step 7 (`Desk:arrange`)                                      |
| `src/state.rs:2032`                                                                    | `layout_command`                                                                                                                                                                                                          | IPC layout command               | D with step 7                                                       |
| `src/state.rs:2207`                                                                    | `Event::Frame`                                                                                                                                                                                                            | per-frame tick                   | R: `tick` in the shell VM                                           |
| `src/state.rs:2334`, `:2341`, `:2351`                                                  | `Event::RulesChanged`, `OptionsChanged`, `WorkspaceSetsChanged`                                                                                                                                                           | reload events                    | D: rules and sets are Lua tables                                    |
| `src/state.rs:2399`                                                                    | `Event::WindowRoleChanged`                                                                                                                                                                                                | role                             | P: node property                                                    |
| `src/state.rs:2748`                                                                    | `Event::MoveResize`                                                                                                                                                                                                       | interactive move                 | R: `Window:on_drag`                                                 |
| `src/state.rs:3012`, `:3139`, `:3149`, `:3166`                                         | `Event::PointerPressed`, `PointerMoved`, `PointerGrab`; `apply`                                                                                                                                                           | pointer policy                   | D: `Window:on_press` and rest timers in `shell/focus.lua`           |
| `src/server.rs:247`, `:250`                                                            | `window_rules`, `workspace_sets` fields                                                                                                                                                                                   | server config                    | D with the config path                                              |
| `src/server.rs:1031`, `:1409`                                                          | `LayoutCommand`                                                                                                                                                                                                           | layout command API               | D with step 7                                                       |
| `src/server.rs:1127`, `:1459`                                                          | `set_window_rules`, `Rule`                                                                                                                                                                                                | rules API                        | D                                                                   |
| `src/server.rs:1136`, `:1464`                                                          | `set_window_management(WmOptions, TitlebarActions)`                                                                                                                                                                       | policy API                       | C for surviving keys (15.2); D rest                                 |
| `src/protocols/extension.rs:264`, `:278-280`, `:310`                                   | `AppIdChanged`, `ToggleMaximize`, `Minimize`, `Lower`, `Raise`                                                                                                                                                            | extension titlebar actions       | R: `Window:on_press`, `maximize`, `minimize`                        |
| `src/protocols/xdg_shell.rs:130-165`                                                   | `ToggleMaximise`, `ToggleFullscreen`, `Minimise`                                                                                                                                                                          | xdg-shell requests               | R: `maximize`, `fullscreen`, `minimize` on the Window node          |
| `src/protocols/zwlr_foreign_toplevel_management_v1.rs:296`, `:322`, `:336-340`, `:354` | `ActivationRequested`, `ToggleMaximise`, `Minimise`, `Unminimise`, `ToggleFullscreen`                                                                                                                                     | foreign-toplevel requests        | R: same requests as xdg-shell                                       |
| `src/protocols/xdg_activation_v1.rs:295`, `:331`                                       | `ActivationRequested`                                                                                                                                                                                                     | xdg-activation                   | R: `activate`; denial decided in Rust (section 13, PortFocus OPEN)  |
| `src/protocols/gnoblin_window_frame_v1.rs:615`, `:619`, `:641-707`                     | `ToggleMaximise`, `Minimise`, stack, window, focused                                                                                                                                                                      | decoration frame                 | D with step 8 (`node-api-refactor.md` step 8)                       |
| `src/protocols/zwlr_layer_shell_v1.rs:334`, `:402`, `:446`, `:450`                     | `end_surface`, `OutputReserved`, `layers()`                                                                                                                                                                               | layer shell reserved zones       | D: `Bar` docks; `Desk:arrange` refits (`node-api-refactor.md:1553`) |
| `src/xwayland.rs:470`                                                                  | `Event::WindowClosed`                                                                                                                                                                                                     | X11 close                        | R: node destruction                                                 |

UNVERIFIED in this pass (named in an earlier survey, not reopened): `src/state.rs:77`, `:1020-1042`, `:2289-2299`, `:2452`, `:2545`, `:1513`; `src/frame.rs:260`; `src/lib.rs:24-25`; `src/xwayland.rs:14`, `:81`; `src/protocols/extension.rs:53`, `:424`; `src/protocols/xdg_shell.rs:38`; `src/protocols/zwlr_foreign_toplevel_management_v1.rs:64`; `src/protocols/xdg_activation_v1.rs:83`; `src/protocols/zwlr_layer_shell_v1.rs:9`, `:14`, `:60`; `src/protocols/xdg_dialog_v1.rs:7` (doc only); `tests/capture.rs:1558-1566`; `tests/new_globals.rs:32`; `tests/protocols.rs:38`; `tests/owned/mod.rs:29`; `tests/owned/master.rs:10-65`; `tests/session/mod.rs:29`; `tests/real_client.rs:38-1295`; `tests/render.rs:50-440`; `src/protocols/extension.rs:424`.

### 14.5 gnoblin-xwayland

| File:line                | Symbol                                                                                                                 | Purpose                       | Replacement                                      |
| ------------------------ | ---------------------------------------------------------------------------------------------------------------------- | ----------------------------- | ------------------------------------------------ |
| `src/windows.rs:36`      | `ActivationDecision`, `Event`, `WindowKind`, `WindowProtocol`, `WindowSpec`, `Wm`                                      | X11 window table feeds the WM | R: X11 windows become Window nodes; vocabulary D |
| `src/windows.rs:205-358` | `Event::ActivationRequested`, `WindowOpened`, `WindowClosed`, `MoveResize`, `WindowTitleChanged`, `WindowAppIdChanged` | X11 lifecycle                 | R/P: as 14.4                                     |
| `src/windows.rs:394`     | `Effect`, `WmOptions`                                                                                                  | test                          | D with the test                                  |
| `src/windows.rs:398-401` | `Event::WorkspaceAdded`, `OutputConnected`                                                                             | test fixtures                 | D with the test                                  |

### 14.6 gnoblin-session, gnoblin-config, gnoblinctl, xtask

| Location                                                     | Symbol                                      | Purpose                          | Replacement                                            |
| ------------------------------------------------------------ | ------------------------------------------- | -------------------------------- | ------------------------------------------------------ |
| `crates/gnoblin-session/src/raw.rs:303`                      | string `gnoblin_wm:`                        | log-line fixture; not an API use | C: change the fixture string when the crate is deleted |
| `crates/gnoblin-config/schema/config.toml:1781-1937`         | `window-management` section                 | config keys                      | C or D per key (15.2)                                  |
| `crates/gnoblin-config/schema/config.toml:2343`              | `layout` section                            | layout selector                  | D with step 7                                          |
| `crates/gnoblin-config/schema/config.toml:2370`              | `workspaces` section                        | workspace list                   | C: stays as the declared list                          |
| `crates/gnoblin-config/schema/config.toml:2383`              | `workspace-sets` section                    | sets                             | D: Lua table (`node-api-refactor.md:185`)              |
| `crates/gnoblin-config/schema/config.toml:2508`              | `window-rules` section                      | rules                            | D: Lua `rules` (`node-api-refactor.md:183`)            |
| `crates/gnoblin-config/src/hooks.rs:86-116`                  | `window_management` hook                    | refuses removed keys             | keep until W3; then delete with the keys               |
| `crates/gnoblinctl/src/cli.rs:386`, `:456`                   | `config`, `workspace`, `activate-set` verbs | CLI                              | D: `activate-set` goes with sets; `config` stays       |
| `xtask/dupes-allow.toml` (entries for `gnoblin-wm::*`)       | duplicate allow-list                        |                                  | D in W6                                                |
| `xtask/src/deps.rs:15`, `:49`, `:1284`, `:1656`              | `gnoblin-wm` in crate lists                 | layering rules                   | D: remove entries                                      |
| `xtask/src/dupes.rs:703`; `xtask/src/logging.rs:359`, `:368` | `gnoblin-wm` fixtures                       | tests                            | D with the tests                                       |
| `xtask/fixtures/metadata-testkit-normal.json:4`              | package entry                               | fixture                          | D                                                      |
| `Cargo.toml:87` (root)                                       | workspace member or dependency              |                                  | D (W6)                                                 |

### 14.7 Not verified in this pass

- `gnoblinctl` windows or layout verbs: none found under `crates/gnoblinctl/src` with those names in this pass. Only `workspace` and `config` were confirmed.
- Docs that name window-management keys, not read: `docs/config/configure/window_management.md`, `docs/config/window_rule.md`, `docs/recipes/put-apps-on-named-workspaces.md`, `docs/shell-api/overrides.md`, `docs/guides/theming.md`, `docs/config.md`. Each needs a line pass in the lane that deletes its key.
- `xtask/dupes-allow.toml`: per-line numbers not listed; the gate fails on a stale entry, so W6 must remove every `gnoblin-wm::` entry.

## 15. Deletion lanes

Six lanes, in order. Each ends green on Agent37 with its own tests passing. W1 to W4 have disjoint file sets; W5 starts only after they merge; W6 removes the crate.

| Lane | Area                                 | Files                                                                                                                                                                                                                                                                                                                             | Tests that must exist before deleting                                                                                                                                |
| ---- | ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| W1   | Backend and X11 windows              | `gnoblin-backend/src/render/compose.rs:81`; `gnoblin-backend/src/x11.rs:52`; `gnoblin-xwayland/src/windows.rs` (36, 205-358, 394-401); `gnoblin-wayland/src/xwayland.rs:470`                                                                                                                                                      | X11 map, close, title and app_id tests in `gnoblin-xwayland`; port the open and close checks to node creation first                                                  |
| W2   | Layouts                              | `gnoblin-lua/src/layout/*`; `gnoblin-lua/tests/layout.rs`; `gnoblin-lua/tests/shipped_layouts.rs`; `gnoblin-compositor/src/layout.rs`; `gnoblin-compositor/src/ipc/server.rs:2458`; `gnoblin-wayland/src/state.rs:1981-2108`; `gnoblin-wayland/src/server.rs:1031,1409`; `config.toml:2343`                                       | `strip_test.lua` and `tiling_test.lua` in place; floating placement goldens from `shell/floating.lua`                                                                |
| W3   | Options, rules, sets and config keys | `gnoblin-compositor/src/window_management.rs`; `window_rules.rs`; `workspace_sets.rs`; `tests/workspace_sets.rs`; `gnoblin-wayland/src/server.rs:247,250,1127,1136,1459,1464`; `gnoblin-wayland/src/state.rs:2334,2341,2351`; `config.toml:1781-1937,2383,2508`; `hooks.rs:86-116`; `gnoblinctl/src/cli.rs` (`activate-set` only) | `rules_test.lua` and `sets_test.lua` in place. Surviving keys must be readable as seat properties before the Rust readers go                                         |
| W4   | Toplevel protocol requests           | `gnoblin-wayland/src/protocols/extension.rs` (264, 278-280, 310); `xdg_shell.rs` (130-165); `zwlr_foreign_toplevel_management_v1.rs` (296, 322, 336-340, 354); `xdg_activation_v1.rs` (295, 331); `zwlr_layer_shell_v1.rs` (402, 446)                                                                                             | Requests map to node methods (`maximize`, `fullscreen`, `minimize`, `activate`). The denied-activation test must be ported first: OPEN in section 13                 |
| W5   | State core and the frame protocol    | `gnoblin-wayland/src/state.rs` (all sites in 14.4); `gnoblin_window_frame_v1.rs` (615, 619, 641-707, with step 8)                                                                                                                                                                                                                 | Every row of sections 12 and 13 reads PORTED or DROP with a line, except the OPEN rows in 15.1. Starts after W1 to W4 merge                                          |
| W6   | Crate and edges removal              | `crates/gnoblin-wm/` (whole crate); root `Cargo.toml:87`; the five crate `Cargo.toml` edges (14.1 to 14.4); `gnoblin-session/src/raw.rs:303`; `xtask` entries (14.6); `tests/` files in 14.4 that reference the crate                                                                                                             | Every file in `crates/gnoblin-wm/tests/` ported or dropped (the `failure_modes.rs` 80 tests, `layout_validation.rs`, `layout_engine.rs`, `wm_layout.rs`, `model.rs`) |

### 15.1 Old tests still open

From sections 12 and 13. Owner is the lane that must act.

| Old test or group                                                                  | Status                                                          | Lane |
| ---------------------------------------------------------------------------------- | --------------------------------------------------------------- | ---- |
| `minimising_the_focused_window_moves_focus_away`                                   | OPEN: no minimise request or property in the stock shell        | W4   |
| `a_denied_activation_reports_the_decision_without_changing_focus`                  | OPEN: decided in Rust (`xdg_activation_v1.rs`)                  | W4   |
| `no_op_events_produce_no_effects`                                                  | Partial                                                         | W5   |
| `a_window_needs_a_workspace_and_an_output`                                         | OPEN                                                            | W5   |
| `new_windows_open_on_the_focused_windows_output`                                   | OPEN                                                            | W5   |
| Identity group (1)                                                                 | No stock test; section 12 says Keep                             | W5   |
| Secure group (5): four Keep, `a_closed_secure_window_leaves_no_secure_record` DROP | Port four to `rules_test.lua` or node tests                     | W5   |
| `an_app_id_that_newly_matches_a_rule_applies_it`                                   | Partial: stop direction only                                    | W3   |
| `a_rule_naming_a_missing_workspace_reports_a_rejection`                            | OPEN                                                            | W3   |
| `a_refused_rules_list_leaves_the_rules_in_force`                                   | Partial                                                         | W3   |
| `an_out_of_range_opacity_is_refused_and_the_old_rules_stay`                        | Partial                                                         | W3   |
| `a_negative_corner_radius_is_refused`                                              | DROP: design draws blur and shadow only                         | W3   |
| `moving_an_unknown_window_or_to_an_unknown_workspace_is_rejected`                  | Partial: unknown-window case OPEN                               | W5   |
| `a_fullscreen_window_is_not_moved_between_outputs`                                 | Partial: output-only case missing                               | W5   |
| Removal: focus on an inactive removal                                              | OPEN                                                            | W5   |
| Workspace direction (1)                                                            | Port from the `workspaces` script; not line-mapped              | W5   |
| No-shell cascade (2)                                                               | Section 12 says DROP; section 13 L0 conflicts. Decide before W2 | W2   |

### 15.2 Config keys: seat property or deleted

Source: `config.toml:1781-1937`. A seat property is a setting read from the main seat (`node-api-refactor.md` step 11). The SeatPolicy lane publishes 11 window-management keys; that lane's file is not in this worktree, so the 11 are not verified here. The decisions below are a candidate list from the schema.

| Key (line)                                                                 | Decision      | Reason                                               |
| -------------------------------------------------------------------------- | ------------- | ---------------------------------------------------- |
| `focus-mode` (1788)                                                        | seat property | pointer focus policy                                 |
| `focus-new-windows` (1797)                                                 | seat property | `NewWindowFocus`                                     |
| `raise-on-click` (1814)                                                    | seat property | pointer press policy                                 |
| `auto-raise` (1822), `auto-raise-delay` (1830)                             | seat property | pointer policy                                       |
| `focus-change-on-pointer-rest` (1840)                                      | seat property | pointer policy                                       |
| `check-alive-timeout` (1848)                                               | seat property | compositor liveness setting                          |
| `edge-tiling` (1866)                                                       | seat property | drag-to-edge; the shell's snap module reads it       |
| `attach-modal-dialogs` (1874)                                              | seat property | dialog placement                                     |
| `disable-workarounds` (1882)                                               | seat property | X11 compatibility; survives only while Xwayland does |
| `constrain-drag-to-work-area` (1890)                                       | seat property | `DragConstraint` input                               |
| `auto-maximize` (1806)                                                     | DELETED (W3)  | auto-maximise is a shell rule                        |
| `workspaces-only-on-primary` (1858)                                        | DELETED (W3)  | a Desk policy in `shell/`, not a seat key            |
| `dynamic-workspaces` (1898)                                                | DELETED (W3)  | workspace count is the declared list (2370)          |
| `action-double-click-titlebar` (1905), `-middle-` (1914), `-right-` (1923) | KEPT (W3)     | published as seat props; the shell reads them        |
| `button-layout` (1932)                                                     | DELETED (W3)  | no reader in the node world                          |
| refused-keys hook (`hooks.rs:86-116`)                                      | KEPT (W3)     | tombstones for the removed window-management keys    |

Seat-property keys are written to `Seat` nodes at step 11. Until then W3 keeps `config.wayland.wm` as the reader and swaps only the type.
