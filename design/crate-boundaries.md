# Crate boundaries

Status: analysis, with the renderer split in progress. Done: the renderer-neutral frame seam is `gnoblin-render-frame` (row `gnoblin-backend` `render/` below, phase 1). The Vulkan, GLES and software renderers are still in `gnoblin-backend` until their phases land. Every number comes from reading the files at `origin/rust-compositor` (8793f0573). Line counts are non-test Rust lines under `src/`, with test files counted separately where named. Transitive counts are from `Cargo.lock` only where stated; other counts are inference.

## The rule

A module becomes its own crate when it is a distinct feature, or when it drags a large external stack that the rest of its crate does not need. A binary crate only dispatches. Each flag below names the rule it fires.

- **Rule i (heavy leaf):** one module, or a small group, is the only user of a heavy external dependency.
- **Rule ii (distinct feature):** the module has its own failure modes and tests.
- **Rule iii (costly hub):** a crate many others depend on carries rarely needed heavy deps, so every dependant rebuilds when it changes.

Blast radius is the count of transitive dependants, computed from the `path` dependencies in each `Cargo.toml`.

## Known and out of scope

Cursor (`gnoblin-render/src/cursor/`, 2014 lines) is owned by the CursorCrate lane. Animation (`effect.rs`, `tween.rs`, `easing.rs`, `property.rs`, 2,000-odd lines together) is owned by the AnimationCrate lane. The compat wiring is owned by BinSlim, and `gnoblin-log` folding is handled. This audit does not re-flag them.

New crate `gnoblin-keyring-store` (phase 1 of `design/keyring.md`) is a leaf: it names only `gnoblin-log` among workspace crates. `xtask/src/deps.rs` rule `keyring-store-isolation` enforces this. It is not a split of an existing crate, so it is not in the table below.

## Flagged splits

| Crate               | Module                                       | Lines                   | Heavy deps it alone pulls       | Rule   | Proposed crate           |
| ------------------- | -------------------------------------------- | ----------------------- | ------------------------------- | ------ | ------------------------ |
| gnoblin-wayland     | `pipewire_export.rs` + tests                 | 509 + 621               | pipewire (61 crates, inference) | i, ii  | gnoblin-pipewire-export  |
| gnoblin-input       | `eis/` (done)                                | 2297                    | reis (26 crates, inference)     | i, ii  | gnoblin-input-eis        |
| gnoblin-lua         | `layout/`                                    | 2054                    | mlua, in 5 of 10 files          | i, ii  | gnoblin-lua-layout       |
| gnoblin-backend     | `render/vulkan/`                             | about 4,600 incl. tests | ash, naga (optional)            | i      | gnoblin-backend-vulkan   |
| gnoblin-backend     | `drm/`, `libinput/`, `udev.rs`, `session.rs` | about 5,300             | smithay hardware features       | i, iii | gnoblin-backend-drm      |
| gnoblin-portal-core | `ui.rs`, `paint.rs`                          | 377 + 349               | egui (82 crates, inference)     | i      | gnoblin-portal-ui (done) |
| gnoblinctl          | `logs/`, `logs.rs`                           | 1117                    | none beyond clap                | ii     | gnoblinctl-logs          |
| gnoblin-session     | `guardian.rs`                                | 1007                    | none                            | ii     | gnoblin-session-guardian |

Rows are read from the files named below. `gnoblin-lua/src/layout/` has ten files, and five of them contain `use mlua` (`data.rs`, `lua.rs`, `reference.rs`, `reply.rs`, `require.rs`). `gnoblin-backend` carries `ash` only behind the default `vulkan` feature (`Cargo.toml`), and `naga/spv-out` is shared with the GLES path.

## Per-split detail

**pipewire export.** Only `pipewire_export.rs` and its test file name `pipewire`. Dependants of `gnoblin-wayland` are 6 transitively. The split leaves `gnoblin-wayland` free of the PipeWire stack. Risk: the module reads capture types from the wayland crate, so the new crate sits above `gnoblin-wayland` and cannot be the reverse (inference, no cycle check run).

**input EIS.** `reis` is named only in `eis/server.rs` (855 lines). `eis/service.rs` is 1327 lines and uses no `reis`. Dependants of `gnoblin-input` are 7 transitively. Risk: `service.rs` and `server.rs` share types; they move together.

**lua layout.** The layout engine is a distinct feature with its own tests. Five of its files call `mlua`, and `engine.rs`, `host.rs` and `snapshot.rs` do not. Dependants of `gnoblin-lua` are 4 transitively. Overlap: `gnoblin-lua-sandbox` already exists (1630 lines, `mlua`), so the naming needs a decision.

**backend vulkan.** The Vulkan renderer is opt-in in practice: `vulkan` is a default feature, and `ash` is a raw binding. Splitting it removes `ash` and its `unsafe` shims from the GLES and DRM builds. Blast radius is the backend's 10 transitive dependants.

**backend drm.** Hardware code (`drm/gpu.rs` 1292 lines, `drm/plan.rs` 913, `libinput`, `udev`, `session`) needs smithay's hardware features. Nested and headless paths need none of them. Blast radius is 10 transitive dependants, so the split helps only if hardware-only changes stop rebuilding the nested and headless paths (inference).

**portal ui.** `ui.rs` and `paint.rs` were the only `egui` users in `gnoblin-portal-core`; `model/` is not (grep inference from the file list). Dependants are 4 transitively. Done: they moved to `gnoblin-portal-ui`, which holds the `DialogUi` trait, the shell and the painter. `gnoblin-portal-core` now holds the models only and drops the egui dependency.

**gnoblinctl logs.** `gnoblinctl` is a binary, yet its library holds 1117 lines of log logic (`logs/follow.rs` 460, `logs/bounds.rs` 338, `logs.rs` 319). That breaks the dispatch-only rule. Dependants: 1.

**session guardian.** `guardian.rs` (1007 lines) has its own tests in `tests.rs`. It adds no heavy dependency, so this split is for rule ii only and is the lowest priority.

## Considered and not flagged

- `gnoblin-wasm`: `wasmtime` (119 crates, `Cargo.lock` inference) is confined to `bindings.rs` and `host.rs`, but no crate depends on `gnoblin-wasm`. It is a leaf, so a split gains nothing.
- `gnoblin-proto`: `prost-build` and `protox` are build-dependencies. They compile once per profile and do not rebuild when `proto` sources change, so they are not a rebuild cost.
- `gnoblin-compat-core/src/caller.rs` (55 lines) is the only `zbus` user in that crate, but `gnoblin-compat-host` uses `zbus` too. Moving 55 lines removes nothing from the build.
- `gnoblin-config` (27 transitive dependants) and `gnoblin-model` (24) are hubs with no heavy external deps. Splitting them would not help.

## Tiny crates

These add a graph node and no feature. They are flagged in the opposite direction.

- `gnoblin-compat-dbus-gnome-session-manager`, `gnoblin-compat-dbus-mutter-servicechannel`, `gnoblin-compat-dbus-mutter-inputmapping`: `src/lib.rs` is one comment line each.
- `gnoblin-service-testkit`: 48 lines.
- `gnoblin-compat-qt-portal-settings`: 91 lines, one dependant (`gnoblin-compat-host`).

## Store duplication

`gnoblin-store` is the only crate that uses `rusqlite`. The old `gnoblin-log/src/store/` is gone, and `gnoblin-log` is a facade with no store. `gnoblin-store` depends on `gnoblin-log` and `gnoblin-model`, and the compositor and the portal depend on it.

## Ranked top ten

1. `gnoblin-wayland` pipewire export to `gnoblin-pipewire-export`. Done: the module moved with its tests, and `xtask deps` rule `pipewire-export-isolation` holds it.
2. `gnoblin-input` EIS to `gnoblin-input-eis`. Done.
3. `gnoblin-backend` Vulkan to `gnoblin-backend-vulkan`.
4. `gnoblin-backend` hardware to `gnoblin-backend-drm`.
5. `gnoblin-lua` layout to `gnoblin-lua-layout`.
6. `gnoblin-portal-core` UI to `gnoblin-portal-ui` (done).
7. `gnoblinctl` logs to `gnoblinctl-logs`.
8. `gnoblin-session` guardian to `gnoblin-session-guardian`.
9. Store duplication: done. `gnoblin-log` no longer has a store, and `gnoblin-store` owns persistence.
10. Tiny stub crates: remove or fold the three one-line compat stubs.

## Needs Kieran

- Settled: `gnoblin-store` is wired in. The compositor and the portal open the log store through it.
- Lua naming: `gnoblin-lua-layout` next to the existing `gnoblin-lua-sandbox`, or one crate?
- Are the three one-line compat stubs placeholders to fill, or dead crates to delete?
- Backend split: do you want `drm` and `vulkan` out now, or only `vulkan`, since `drm` has the weaker blast-radius gain?
- Compat `Extension` seam: `gnoblin-compat-host` keeps the compositor-side `Extension` registry (`compat`, Redirect and Interop adapters) for now. Should that registration move behind a seam trait implemented by `gnoblin-wayland`, or should the adapters link `gnoblin-wayland` as the two protocol adapters already do? Services-role adapters now own their glue in an `agent` module and depend on `gnoblin-service-core`.
