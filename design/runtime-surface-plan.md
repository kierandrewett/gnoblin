# Runtime surface plan

Status: implementation plan for the Rust tree. S1 and S2 are being ported to `crates/`; the C tree in
`src/` remains a reference. The audit rows it relies on are in `design/docs-parity/{api,config,guides}.md`.

Related: `design/rust-compositor.md` (Phase 3 at lines 815-826, Phase 4 at 828-861, object model at 1052-1063,
Configuration and Lua at 1064-1083), `design/lua-api-goal.md`, `design/shell-api-redesign.md`.

## 1. The main finding: the design lives in the legacy C tree

The parity audit confirms that most documented Lua and CLI APIs remain missing from `crates/`. The earlier redesign's
"Done" primitives are in the C tree (`src/config/lua/`); its API-table branch is an ancestor of HEAD, so those
primitives are a reference but have not been ported.

The Rust port rebuilds them using the redesign's names and principles, not its C shapes. This plan records that
distinction and S14 corrects the redesign status line.

## 2. Verified evidence

Each row was checked by grep or by reading the file. "Missing" means grep found nothing in the named tree.

| Surface                                                                                                                    | Result                                                                         | Evidence                                                                                     |
| -------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------- |
| `gnoblin.on` (global)                                                                                                      | Missing. `on` exists per instance only                                         | `gnoblin-lua/src/bindings.rs:351`, `:536`                                                    |
| `gnoblin.once`                                                                                                             | Missing                                                                        | grep `once` in `gnoblin-lua/src`: nothing                                                    |
| `gnoblin.find`                                                                                                             | Missing                                                                        | grep `"find"` in `gnoblin-lua/src`: nothing                                                  |
| `gnoblin.windows()` collection                                                                                             | Missing. `windows` is a layout context value                                   | `gnoblin-lua/src/layout/lua.rs:351`                                                          |
| `gnoblin.process.spawn`, `Process`                                                                                         | Missing                                                                        | grep `"spawn"`: nothing                                                                      |
| `gnoblin.tween`, `ease`, `lerp`                                                                                            | Missing                                                                        | grep `tween`, `"ease"`, `"lerp"`: nothing                                                    |
| `gnoblin.snapshot`, `array`                                                                                                | Missing                                                                        | grep `"snapshot"`: nothing                                                                   |
| `gnoblin.animation`                                                                                                        | Missing. Config section only                                                   | `gnoblin-config/schema/config.toml:2122`                                                     |
| Shortcuts, keybindings in Lua                                                                                              | Missing. Config sections only                                                  | `config.toml:2196`, `:2206`                                                                  |
| `gnoblin.window_rule`                                                                                                      | Missing. Config section and wm consumer exist                                  | `config.toml:2211`; `gnoblin-wm/src/effect.rs:21`                                            |
| Window rule producer                                                                                                       | Missing in production. Only tests send `RulesChanged`                          | `gnoblin-wm/src/wm.rs:422` (handler); `gnoblin-wm/tests/failure_modes.rs:52`, `model.rs:202` |
| Autostart in Lua                                                                                                           | Missing. Config and session plan exist                                         | `config.toml:2186`; `gnoblin-session/src/plan.rs:31,38,144`                                  |
| Window close, minimise, maximise, fullscreen, move, resize, workspace, monitor                                             | Missing. Methods are `focus`, `kill`, `add_effect`, `request_menu`             | `gnoblin-schema/schema/window.toml`                                                          |
| Window `pid`, `process`                                                                                                    | Missing. No such properties                                                    | `window.toml` property list                                                                  |
| `Window:present`, `set_opacity`, `set_visual`                                                                              | Missing                                                                        | grep: nothing in `gnoblin-lua/src` or `window.toml`                                          |
| `Operation:map`, `Operation:timeout`                                                                                       | Missing. `Timer:cancel` exists                                                 | `bindings.rs:924` (Timer only)                                                               |
| `gnoblinctl` status, logout, ping, version, config, session, focus, shortcut, input, privacy, launch, capabilities, reload | Missing. Only `ipc`, `completions`, `log` and generated object verbs           | `gnoblinctl/src/cli.rs:39-60`, `:454-478`                                                    |
| `org.gnoblin.SessionIdle`                                                                                                  | Present (S11). Serves `GetActivity` and `ActivityChanged` from the idle object | `gnoblin-service-sessionidle/src/lib.rs:193`; `tests/session_idle.rs`                        |
| `org.gnoblin.Compositor`, `CheckPermission`                                                                                | Missing                                                                        | grep: nothing in the tree                                                                    |
| `org.freedesktop.ScreenSaver`                                                                                              | Present. Drift still to check                                                  | `gnoblin-service-screensaver/src/lib.rs:62,64`                                               |
| `timers.after` unit                                                                                                        | Mismatch. Docs say seconds; Rust binds ticks                                   | `docs/public/gnoblin.d.lua:274`; `gnoblin-lua/host_members.toml`                             |

The audit files count MISSING rows as 49 in `api.md`, 14 in `config.md` and 12 in `guides.md`.

## 3. Architecture that exists

- **Runtime role is real.** `gnoblin/src/main.rs:216` runs `gnoblin_lua::RoleRunner`. Roles are in
  `gnoblin/src/role.rs:29-37`: guardian, compositor, runtime, services.
- **Core objects are installed.** `gnoblin-lua/src/bindings.rs:250-252` installs one class table per schema object in
  the `gnoblin` namespace: capture, effect, eis, idle, lock, output, portal_ui, request, session, settings, window,
  workspace. So `gnoblin.window` exists; its methods are the schema's.
- **Host-only members are declared once.** `gnoblin-lua/host_members.toml` declares `configure`, `load` and `timers`.
  The installer is `install` in `bindings.rs:188`.
- **Extension namespaces** become Lua modules from the schema at load time (`extension_bindings.rs`). `on` is reserved
  there (`:34`).
- **Window rules and autostart are config data.** Both are `carried` sections of `gnoblin-config/schema/config.toml`.
  The wm consumes rules (`gnoblin-wm/src/effect.rs`, `rules.rs`).
- **Input and animation exist without Lua.** `gnoblin-input` has `binding.rs`, `chord.rs`, `router.rs` and `idle.rs`.
  `gnoblin-animation` has `easing.rs`, `property.rs` and `transform.rs`. Neither is reachable from a script.
- **Operation state is tracked.** `gnoblin-lua/src/host.rs` holds per-operation status and outcome (`:69`). No Lua
  combinators sit on top of it.

## 4. Assumptions

- The Rust port is the target. The C tree is a reference, and it is deleted at cutover (`rust-compositor.md`, Lua card).
- Lua names follow the redesign's principles: positional arguments, explicit Operation for every write, one Request
  lifecycle, one descriptor table, no shims (`design/lua-api-goal.md`).
- A new public name is added only where `docs/` already defines it. Anything else is not added.
- Each slice writes its failure-mode tests first, then the code, then its docs. Verification is the main agent's job.
- Shipped sections keep their schema names; the schema is the source for the Lua stub, the IPC catalogue and `gnoblinctl`.

## 5. Slices

Foundations first, then the window and process surface, then animation, input, CLI and the D-Bus services.

| #   | Slice                                     | Effort | Depends on      | Start now |
| --- | ----------------------------------------- | ------ | --------------- | --------- |
| S1  | Timers and Operation combinators          | M      | decision 1      | yes       |
| S2  | Global event bus                          | M      | S1              | yes       |
| S3  | Object collections and `find`             | M      | S2              | no        |
| S4  | Window method set and `present`           | L      | S3, decision 3  | no        |
| S5  | Process object and attribution            | L      | S1, S2          | no        |
| S6  | Request lifecycle in Lua                  | M      | S2              | no        |
| S7  | Visuals, tweens and named animations      | L      | S1, S4          | no        |
| S8  | Shortcuts and keybindings                 | L      | decision 4      | phase A   |
| S9  | Autostart and window-rule producer        | M      | S2, WindowRules | no        |
| S10 | gnoblinctl verbs                          | L      | S4, S5, S8      | phase A   |
| S11 | `org.gnoblin.SessionIdle`                 | M      | none            | yes       |
| S12 | `org.gnoblin.Compositor`, CheckPermission | M      | decision 6      | no        |
| S13 | ScreenSaver drift                         | S      | S11             | no        |
| S14 | Parity gate and doc correction            | M      | S1-S13          | no        |

### S1. Timers and Operation combinators

- **Goal:** Lua `timers.after` and `timers.now` with the documented unit; `Operation:map` and `Operation:timeout`
  added; `Timer:cancel` (present at `bindings.rs:924`) kept.
- **Files:** `gnoblin-lua/src/host.rs`, `bindings.rs`, `host_members.toml`, `gnoblin-model` (operation ids).
- **Schema:** none new; the Operation handle is the existing model type.
- **Tests first:** cancel stops a timer but not an action already sent; a throwing callback is isolated; a reload
  cancels pending timers; timeout completes with a named code.
- **Docs:** `docs/shell-api/errors.md`, generated `gnoblin.d.lua`.

### S2. Global event bus

- **Goal:** `gnoblin.on(name, fn)` and `gnoblin.once(name, fn)` as the one subscription path. Per-object `on` stays
  only where the redesign keeps it.
- **Files:** `gnoblin-lua/src/bindings.rs` (`:351`, `:536`), `host.rs`, `extension_bindings.rs` (`on` reserved, `:34`).
- **Schema:** reuse each object's signals from `gnoblin-schema/schema/*.toml`.
- **Tests first:** a throwing handler does not disable later handlers; `once` fires once; unsubscribe and reload clear
  handlers; a handler's return value reaches the caller's Operation.
- **Docs:** `docs/shell-api/` event pages.

### S3. Object collections and `find`

- **Goal:** `gnoblin.windows()`, `outputs()`, `workspaces()` and `gnoblin.find(kind, filter?)`, returning interned live
  objects with `alive`.
- **Files:** `gnoblin-lua/src/bindings.rs`, `gnoblin-model` (interning and revisions).
- **Schema:** reuses `window.toml`, `output.toml`, `workspace.toml`. No new fields.
- **Tests first:** `find` returns only live objects; an unknown filter field is refused; an object whose window closed
  reports `alive == false`.
- **Docs:** `docs/shell-api/find.md`, window and output pages.

### S4. Window method set and `present`

- **Goal:** `close`, `minimise`, `maximise`, `fullscreen`, `move`, `resize`, `set_workspace`, `set_output`, and
  `Window:present()` with an input-quiet gate.
- **Files:** `gnoblin-schema/schema/window.toml` (new methods), `gnoblin-wm` (effects), `gnoblin-compositor` (dispatch),
  `gnoblin-lua`.
- **Schema:** new methods on `window`, using only names `docs/` already defines.
- **Tests first:** a stale window refuses with a named code; `move` and `resize` clamp or refuse outside the output
  (decision 3); `present` reports "focused", not "accepted".
- **Docs:** `docs/shell-api/focus-activation.md`.

### S5. Process object and attribution

- **Goal:** `gnoblin.process.spawn(argv, options?)` returning `Process` (`pid`, `wait`, `kill`, exit event); `Window.pid`
  and `Window.process` from the display server.
- **Files:** `gnoblin-lua` (new process module), `gnoblin-compositor` (pid lookup), `gnoblin-session` (spawn semantics).
- **Schema:** new `process` object in the `gnoblin` namespace; `window.toml` gains `pid` and `process`.
- **Tests first:** a long-lived child does not hold a reload back; `kill` is idempotent; a window's `process` is
  attributed from the compositor, never from an app ID.
- **Docs:** `docs/shell-api/launch-feedback.md`.

### S6. Request lifecycle in Lua

- **Goal:** one `request` object for prompt, auth, location and lock, with `claim`, `answer`, `decline`, `finished`,
  one uniform outcome.
- **Files:** `gnoblin-lua/src/bindings.rs` (class already installed), `gnoblin-compositor/src/ipc/requests.rs`.
- **Schema:** `gnoblin-schema/schema/request.toml` already declares the request surface; check the Lua class exposes it.
- **Tests first:** a socket client that did not receive the request cannot answer it; `finished` fires exactly once;
  a shell that dies mid-request ends it with a named code.
- **Docs:** `docs/shell-api/authentication.md`, `docs/shell-api/grants.md`.

### S7. Visuals, tweens and named animations

- **Goal:** `gnoblin.ease`, `gnoblin.lerp`, `gnoblin.tween()` compiled to a native description; `Window:set_opacity`,
  `set_scale`, `set_rotation`, `set_offset`, `reset_visuals`; named `gnoblin.animation` declarations.
- **Files:** `gnoblin-animation` (`easing.rs`, `property.rs`, `transform.rs`), `gnoblin-lua` (tween module),
  `gnoblin-schema` (`window.toml`, `effect.toml`), config section `animations`.
- **Tests first:** a tween on a closed window ends with a named code; the lock refuses visual writes; a reload resets
  visuals; reduced motion (`settings.enable_animations`) finishes at the end state.
- **Docs:** `docs/shell-api/appearance-animations.md`, `docs/config/configure/appearance.md`.

### S8. Shortcuts and keybindings

- **Goal:** one chord grammar; `shortcuts` and `keybindings` config reach the input router. Phase A is config wiring
  with no Lua. Phase B adds a Lua surface only if decision 4 asks for it.
- **Files:** `gnoblin-input` (`binding.rs`, `chord.rs`, `router.rs`), `gnoblin-config/src/document.rs`.
- **Schema:** `config.toml` sections `shortcuts` (`:2206`) and `keybindings` (`:2196`).
- **Tests first:** a duplicate chord is refused with a named code; a shortcut does nothing under the lock; a reload
  removes bindings it no longer declares.
- **Docs:** `docs/config/configure/shortcuts.md`, `docs/config/configure/keybindings.md`,
  `docs/shell-api/keyboard-shortcuts.md`.

### S9. Autostart and window-rule producer

- **Goal:** config `autostart` feeds the session plan; a production producer sends `RulesChanged` so `wm.rs:422` acts.
- **Files:** `gnoblin-config/src/document.rs`, `gnoblin-session/src/plan.rs`, `gnoblin-wm/src/rules.rs`, `gnoblin-lua`.
- **Coordination:** the WindowRules lane owns window rules. This slice takes autostart only, and waits for that lane
  before touching the rule producer.
- **Tests first:** an autostart entry starts once and is not restarted by a reload; a rule naming a missing workspace
  logs a warning and leaves the window in place (`effect.rs:21`).
- **Docs:** `docs/config/configure/autostart.md`, `docs/config/configure/window_management.md`.

### S10. gnoblinctl verbs

- **Goal:** `status`, `version`, `ping`, `capabilities` first (phase A, no dependency); then `reload`, `config`,
  `session`, `focus`, `shortcut`, `input`, `privacy`, `launch`, `logout` as their slices land.
- **Files:** `gnoblinctl/src/cli.rs`, `gnoblinctl/src/` command modules. The generated object verbs stay.
- **Schema:** verbs are generated from the schema where they are object actions; the rest are hand-written
  (decision 5).
- **Tests first:** each verb's output is stable JSON; an unknown verb lists the ones that exist; `status` reports
  a dead compositor as an error, not an empty success.
- **Docs:** `docs/cli/index.md`, `docs/cli/`.

### S11. `org.gnoblin.SessionIdle`

- **Goal:** a D-Bus service exposing the idle state from the existing idle object, implemented against the existing
  spec: `GetActivity` and the `ActivityChanged` signal, nothing else.
- **Files:** new crate beside `gnoblin-service-screensaver`; reads `gnoblin-input/src/idle.rs`; registers in
  `gnoblin-services`.
- **Schema:** reuses `idle.toml` (`idle_ms` and the idle state). `docs/dbus/session-idle.md` defines no inhibit method,
  so the `inhibit`, `uninhibit` and `inhibitors` fields are not exposed here.
- **Tests first:** `ActivityChanged` fires once per state change and not on repeats; an unknown caller gets the folded
  state; a restarted service reseeds from the compositor and does not replay the old state.
- **Not in S11:** inhibit cookies and their release. Inhibit belongs to `org.freedesktop.ScreenSaver` (the screensaver
  crate, S13) and to the runtime idle object. The plan had listed inhibit tests here by mistake; the spec does not
  define them on `org.gnoblin.SessionIdle`, and Main decided against adding them.
- **Docs:** `docs/dbus/session-idle.md` (spec exists; update status only).

### S12. `org.gnoblin.Compositor` and CheckPermission

- **Goal:** `CheckPermission` answers from the authority model in `gnoblin-authority`.
- **Files:** `gnoblin-authority`, a new service crate, `gnoblin-services` registration.
- **Schema:** the permission set (decision 6).
- **Tests first:** a request from an unattributed process is denied; a permission never comes from an app ID or title.
- **Docs:** `docs/dbus/` (new page for the Compositor interface).

### S13. ScreenSaver drift

- **Goal:** compare `gnoblin-service-screensaver` with `docs/dbus/screensaver.md` and the idle object; close method and
  signal gaps.
- **Files:** `gnoblin-service-screensaver/src/lib.rs`.
- **Tests first:** `Inhibit` and `UnInhibit` pair correctly; `ActiveChanged` follows the idle object.
- **Docs:** `docs/dbus/screensaver.md`.

### S14. Parity gate and doc correction

- **Goal:** generate `gnoblin.d.lua` and the reference from the schema; make the audit rows a CI check, so a MISSING
  row without a ledger entry fails; mark the redesign's "Done" items as C-tree work; resolve the timer unit.
- **Files:** `gnoblin-lua/build.rs`, `gnoblin-config-codegen`, `docs/public/gnoblin.d.lua`, `design/docs-parity/`,
  `design/shell-api-redesign.md`.
- **Tests first:** the generated stub matches the schema; a removed member fails the gate.
- **Docs:** this plan's status line; the parity ledger.

## 6. Needs Kieran

Real choices only. Each one changes a slice.

1. **Timer unit.** Seconds, as the docs and the C tree say, or ticks, as the Rust host binds now. Recommendation:
   seconds, because two sources agree on it.
2. **Port shape.** Port the redesign's names (`process.spawn`, `Operation`, global `gnoblin.on`) or the C names. This
   plan assumes the redesign's names, with no shims.
3. **Window control authority.** Are `close`, `minimise`, `move` and `resize` for the shell only, gated by attribution,
   or for any extension?
4. **Shortcut surface.** Config only, or also a Lua `shortcut.bind`? Is the leased `input.grab` in scope for this port?
5. **gnoblinctl verbs.** Keep the generated object verbs only, or ship the hand-written verbs in `docs/cli/`? Which of
   them are real commands, and which should be deleted from the docs?
6. **CheckPermission model.** Which permissions does it answer, and does it live in the services role or the
   compositor?
7. **SessionIdle host.** A new service crate in the services role, or inside the compositor's idle object?

### Decided 2026-11-07

Kieran delegated these to the lanes. Each one is recorded here so a later slice does not re-open it.

1. **Timer unit.** Seconds, matching the docs and the C tree.
2. **Port shape.** The redesign's names (`process.spawn`, `Operation`, global `gnoblin.on`), with no shims.
3. **Window control authority.** `close`, `minimise`, `move` and `resize` are for the shell only, gated by attribution. Extensions need a granted power later.
4. **Shortcut surface.** Config only for now. Lua `shortcut.bind` and `input.grab` are out of scope for this port.
5. **gnoblinctl verbs.** Keep the generated object verbs. Implement the small real set (`status`, `version`, `ping`, `reload`, `config get`, `config validate`). Delete the rest from `docs/cli` until built.
6. **CheckPermission.** Deferred. S12 has not started.
7. **SessionIdle host.** A new service crate in the services role, `gnoblin-service-sessionidle`, served on the ScreenSaver connection.

## 7. Parallel dispatch

Four slices can start now. They touch disjoint files, so they can run in parallel.

| Slice       | Files it touches                                              | Why it is disjoint               |
| ----------- | ------------------------------------------------------------- | -------------------------------- |
| S1          | `gnoblin-lua/src/host.rs`, `bindings.rs`, `host_members.toml` | only Lua host files              |
| S8 phase A  | `gnoblin-input/src/`, `gnoblin-config/src/document.rs`        | input crate; config parsing      |
| S10 phase A | `gnoblinctl/src/`                                             | CLI only                         |
| S11         | new service crate, `gnoblin-services` registration            | new crate; one registration line |

Two conflicts to clear before dispatch. S8 phase A and the WindowRules lane both touch
`gnoblin-config/src/document.rs`; ask that lane first. S11 and S12 both register in `gnoblin-services`; S12 waits for
decision 6, so it does not start now.
