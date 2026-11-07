# Docs parity: api group

Scope: `docs/shell-api/` (28 pages), `docs/bridge/` (8), `docs/cli/` (8), `docs/dbus/` (3), `docs/gnoblinctl.md`. Read-only audit of the Rust tree on `rust-compositor`.

Status key: BUILT (file:line evidence), PARTIAL, MISSING, CHANGED (name or shape differs; described inline). MISSING and CHANGED are judged against the Rust candidate in `crates/`. The live Mutter C runtime in `src/` is a separate tree (`lua-api.md:1809`); rows that name a live C reference (`src/...:line`) show surfaces that already exist there. `[INFERENCE]` marks claims not checked in code.

Effort key: S (under a day), M (a few days), L (a week or more).

Method: grep of `crates/gnoblin-schema/schema/*.toml`, `crates/gnoblin-lua/host_members.toml`, `crates/gnoblin-lua/src/bindings.rs`, `crates/gnoblinctl/src/cli.rs`, `crates/gnoblin-service-screensaver/src/lib.rs`, and `crates/gnoblin-compositor/src/ipc/path.rs`. Doc line numbers come from heading and member greps.

## Summary

Counts are the item rows in the tables below (78 rows), counted by status column.

| Status  | Rows |
| ------- | ---- |
| BUILT   | 3    |
| PARTIAL | 19   |
| MISSING | 48   |
| CHANGED | 8    |

The Rust tree has a schema-generated object model (window, output, workspace, session, settings, idle, lock, request, capture, eis, effect, portal_ui). Lua binds each as a class (`gnoblin.window.all()`, `gnoblin.window.by_id(id)`, instance `:on`) at `bindings.rs:366`, `:380`, `:536`. Beside those classes, `gnoblin` exposes only `configure`, `load` and `timers` (`host_members.toml`). Most other documented API is absent.

## Top ten gaps

1. **Lua runtime surface** (`gnoblin.on`, `once`, `find`, `windows`, `outputs`, `workspaces`, `input`, `shortcuts`, `process`, `tweens`, `settings`, `config`). Owner: Phase 3 runtime role (`design/rust-compositor.md:815-826`). Effort L.
2. **`gnoblin.window_rule`** (`docs/shell-api/windows.md:156`). Owner: WindowRules lane [INFERENCE, from peer roster]. Effort M.
3. **Window methods** `close`, `set_minimized`, `set_maximized`, `set_fullscreen`, `move`, `resize`, `move_to_workspace`, `move_to_monitor`, `thumbnail`, `present` (`windows.md:83-104`). Crates: only `focus`, `kill`, `add_effect`, `request_menu` exist in `window.toml`. Live C runtime (`src/config/lua/gnoblin-lua.c:87-131`) implements them, so the Rust candidate is behind. Owner: unowned; Phase 4 names close and minimise (`rust-compositor.md:836`). Effort M.
4. **Top-level gnoblinctl verbs** `status`, `logout`, `ping`, `version`, `config *`, `session *`, `focus`, `shortcut`, `input`, `privacy`, `launch`, `capabilities`, `reload`. Crates `cli.rs` has none. Live C CLI (`src/cli/gnoblinctl-cli.c:10-35`) implements all of them, including `lua`. Owner: unowned for the Rust port. Effort M.
5. **JSON bridge** `compositor-v1.sock` and `GNOBLIN_COMPOSITOR_SOCKET` (all 8 bridge pages). Rust uses binary protobuf on `ipc.sock` (`ipc/path.rs:4`). Owner: design decision `rust-compositor.md:1573` and `:190-221`; the docs rewrite is unowned. Effort M.
6. **`org.gnoblin.SessionIdle`** (`dbus/session-idle.md`). No crate. Live C runtime serves it (`src/idle.c:18-39`, `ActiveChanged` at `:30`, emitted at `:286`). Design keeps it (`rust-compositor.md:1171-1172`). Owner: services role, unowned for the crate. Effort M.
7. **`org.gnoblin.Compositor`** (`dbus/index.md:6`). No implementation in `src/` or `crates/` (grep found nothing). `portal-policy-contract.md:30` cites `CheckPermission`, which has zero hits in both trees. Owner: unowned. Effort M.
8. **ScreenSaver contract drift**: `SetActive` always false (`lib.rs:311`), `UnInhibit` no-op for foreign cookies (`lib.rs:300`), `ActiveChanged` declared but not emitted (`lib.rs:339`). Owner: services role (`rust-compositor.md:854`). Effort S.
9. **Keyboard grabs, input handlers, shortcuts, text insertion**. No Lua or IPC entry points. Owner: gnoblin-input lane (`rust-compositor.md:1137-1151`) [INFERENCE]. Effort L.
10. **Tweens and animations** (`tweens.md`, `appearance-animations.md`). Owner: Render lane (`rust-compositor.md:442`, `:1070`, `:836`). Effort L.

## shell-api pages

Headings and member lines are from `docs/shell-api/*.md`. Status is for the documented surface on that page.

| Page                  | Doc line                                    | Status  | Rust evidence or gap                                                                                                                                                                        | Design ref                                      | Effort |
| --------------------- | ------------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------- | ------ |
| index                 | `index.md:7` API domains                    | PARTIAL | Only `configure`, `load`, `timers` in `host_members.toml`. Object classes are bound at `bindings.rs:366`, `:380`, `:536`. No global event bus or windows, find, settings or config globals. | `shell-api-redesign.md:28`                      | L      |
| index                 | `index.md:42` runtime identity and settings | MISSING | No `gnoblin.version` or `settings` entry                                                                                                                                                    | `rust-compositor.md:1052-1080`                  | M      |
| appearance-animations | `:22` animation controls                    | MISSING | No `gnoblin.animations`                                                                                                                                                                     | `rust-compositor.md:836`                        | L      |
| authentication        | `:19` answer a prompt from Lua              | MISSING | `request.toml` has claim/answer/decline, no Lua                                                                                                                                             | `rust-compositor.md:177`                        | M      |
| authentication        | `:48` the request                           | PARTIAL | Request object schema only                                                                                                                                                                  | `rust-compositor.md:177`                        | M      |
| errors                | `:5` operation results                      | MISSING | No Operation type in bindings                                                                                                                                                               | `shell-api-redesign.md:76`                      | M      |
| errors                | `:54` timers                                | PARTIAL | `timers.after`, `timers.now` BUILT in `host_members.toml`                                                                                                                                   | n/a                                             | S      |
| find                  | `:16` kinds                                 | MISSING | No `gnoblin.find`                                                                                                                                                                           | `rust-compositor.md:1050` [INFERENCE]           | M      |
| focus-activation      | `:66` focus a program the shell started     | MISSING | `window.toml` focus method only                                                                                                                                                             | `rust-compositor.md:1039`                       | M      |
| grants                | `:25` where a grant comes from              | MISSING | No grants object                                                                                                                                                                            | `rust-compositor.md:1049`                       | M      |
| idle                  | `:24` IdleWatch object                      | PARTIAL | `idle.toml`: props idle_ms, inhibitors; inhibit, uninhibit; no IdleWatch Lua                                                                                                                | `rust-compositor.md:1151`                       | M      |
| input-grab            | `:32` the lease                             | MISSING | No grab object                                                                                                                                                                              | `rust-compositor.md:1143`                       | L      |
| input-handlers        | `:7` input handlers                         | MISSING | No handler registration                                                                                                                                                                     | `rust-compositor.md:1070`                       | L      |
| input                 | `:120` input handlers                       | MISSING | No input API                                                                                                                                                                                | `rust-compositor.md:1137`                       | L      |
| keyboard-shortcuts    | `:195` capture a shortcut                   | MISSING | Only `gnoblinctl shortcut` docs; no CLI                                                                                                                                                     | `rust-compositor.md:1137` [INFERENCE]           | M      |
| launch-feedback       | `:1` launch feedback                        | MISSING | No launch object                                                                                                                                                                            | `rust-compositor.md:1039`                       | M      |
| layer-surfaces        | `:1` layer surfaces                         | MISSING | No layer object in schema                                                                                                                                                                   | `rust-compositor.md:177` [INFERENCE]            | M      |
| outputs               | `:10` read outputs                          | PARTIAL | `output.toml` props and apply, set_layout; no `outputs.list`                                                                                                                                | `rust-compositor.md:1137`                       | M      |
| outputs               | `:104` privacy screens                      | MISSING | No privacy-screen member                                                                                                                                                                    | unowned                                         | M      |
| overrides             | `:24` settings                              | MISSING | No `settings.override`                                                                                                                                                                      | unowned                                         | M      |
| overrides             | `:61` window effects                        | PARTIAL | `window.toml` add_effect; `effect.toml` kinds                                                                                                                                               | `rust-compositor.md:836`                        | M      |
| passphrase-prompts    | `:19` answer a prompt from Lua              | MISSING | Services role prompt exists; no Lua                                                                                                                                                         | `rust-compositor.md:178`                        | M      |
| privacy               | `:9` privacy state                          | MISSING | No privacy object                                                                                                                                                                           | `rust-compositor.md:847`                        | M      |
| processes             | `:22` options                               | MISSING | No `gnoblin.process` in bindings                                                                                                                                                            | `shell-api-redesign.md:80` (legacy branch only) | M      |
| requests              | `:21` fields                                | PARTIAL | `request.toml`: claim, answer, decline; signals offered, finished                                                                                                                           | `rust-compositor.md:177`                        | M      |
| session-runtime       | `:69` runtime health and reload             | MISSING | `session.toml` has version and signals only                                                                                                                                                 | `rust-compositor.md:1281`                       | M      |
| session-runtime       | `:172` locking                              | PARTIAL | `lock.toml`: prop locked, lock_session                                                                                                                                                      | `rust-compositor.md:847`                        | S      |
| tablet-pad-help       | `:7` tablet-pad help                        | MISSING | No member                                                                                                                                                                                   | unowned                                         | M      |
| text-insertion        | `:7` request a text target                  | MISSING | No `text_target`                                                                                                                                                                            | `rust-compositor.md:1137` [INFERENCE]           | M      |
| tweens                | `:20` timeline                              | MISSING | No `gnoblin.tween`                                                                                                                                                                          | `rust-compositor.md:442`, `:1070`               | L      |
| types                 | `:7` shared records                         | PARTIAL | Schema objects exist; no Lua record types                                                                                                                                                   | `rust-compositor.md:1052-1080`                  | M      |
| window-snapping       | `:7` pointer and keyboard snapping          | MISSING | No snapping member                                                                                                                                                                          | unowned                                         | M      |
| windows               | `:3` read windows                           | MISSING | No `gnoblin.windows`; `window` object generated                                                                                                                                             | `rust-compositor.md:1052-1080`                  | M      |
| windows               | `:83` window methods                        | PARTIAL | Only focus, kill, add_effect, request_menu in `window.toml`                                                                                                                                 | `rust-compositor.md:836`                        | M      |
| windows               | `:126` change how a window is drawn         | PARTIAL | `add_effect` BUILT; `too-many-passes` error                                                                                                                                                 | `rust-compositor.md:836`                        | S      |
| windows               | `:156` window rules                         | MISSING | No `gnoblin.window_rule`                                                                                                                                                                    | `rust-compositor.md:3004`                       | M      |
| windows               | `:177` compositor requests                  | PARTIAL | `window.toml` signal `menu_requested`                                                                                                                                                       | `rust-compositor.md:833`                        | S      |
| workspaces            | `:23` read workspaces                       | MISSING | No `gnoblin.workspaces`                                                                                                                                                                     | `rust-compositor.md:1052-1080`                  | M      |
| workspaces            | `:56` create and change                     | PARTIAL | `workspace.toml` set_layout, layout_command                                                                                                                                                 | `rust-compositor.md:1605`                       | M      |

Notes on shell-api:

- `shell-api-redesign.md:80` says `process.spawn` is Done. That is the legacy branch, not the Rust tree. Treat as design-only.
- The `object model` is in the Rust tree. The Lua surface that exposes it is not.

## bridge pages

All eight bridge pages document the JSON socket. The Rust tree does not implement it.

| Page                   | Doc line                                  | Status  | Rust evidence or gap                                        | Design ref                                                  | Effort |
| ---------------------- | ----------------------------------------- | ------- | ----------------------------------------------------------- | ----------------------------------------------------------- | ------ |
| connection             | `:1-30` compositor-v1.sock, NDJSON, hello | CHANGED | `ipc/path.rs:4` uses `ipc.sock`, binary protobuf            | `rust-compositor.md:190-221`, `:1573`                       | M      |
| index                  | bridge overview                           | CHANGED | JSON replaced by protobuf; `gnoblinctl --json` only at edge | `rust-compositor.md:1948`                                   | M      |
| examples               | JSON request examples                     | MISSING | No JSON bridge                                              | `rust-compositor.md:1686` (legacy translator, time-bounded) | M      |
| input-and-focus        | input and focus methods                   | MISSING | No JSON methods                                             | `rust-compositor.md:1686`                                   | M      |
| privacy                | privacy methods                           | MISSING | No JSON methods                                             | `rust-compositor.md:847`                                    | M      |
| runtime-and-appearance | runtime and appearance                    | MISSING | No JSON methods                                             | `rust-compositor.md:1281`                                   | M      |
| session-and-windows    | session and windows                       | MISSING | No JSON methods                                             | `rust-compositor.md:1686`                                   | M      |
| shortcuts-and-events   | shortcuts and events                      | MISSING | No JSON methods or events stream                            | `rust-compositor.md:1686`                                   | M      |

`grep compositor-v1` over `crates/` returns nothing. The design makes the bridge a time-bounded legacy translator (`:1686`). The docs rewrite to `ipc.sock` and `GNOBLIN_IPC_SOCKET` is unowned.

## cli pages

| Page                          | Doc line                                                                                             | Status         | Rust evidence or gap                                                              | Design ref                            | Effort |
| ----------------------------- | ---------------------------------------------------------------------------------------------------- | -------------- | --------------------------------------------------------------------------------- | ------------------------------------- | ------ |
| index                         | `:8` version and build identity                                                                      | PARTIAL        | clap `--version` only (`cli.rs:129`)                                              | `rust-compositor.md:211-212`          | S      |
| index                         | `:29` version via API                                                                                | MISSING        | No version verb                                                                   | unowned                               | S      |
| completion                    | `completion` verb                                                                                    | CHANGED        | Rust is `completions` (`cli.rs:271-272`)                                          | `rust-compositor.md:211-212`          | S      |
| graphical-console             | console                                                                                              | MISSING        | No console verb                                                                   | `rust-compositor.md:2829` [INFERENCE] | M      |
| input-and-animations          | input and animation verbs                                                                            | MISSING        | No verbs in `cli.rs`                                                              | unowned                               | M      |
| lua                           | `gnoblinctl lua`                                                                                     | MISSING (Rust) | No `lua` subcommand in Rust `cli.rs`; live C `src/cli/gnoblinctl-cli.c:35` has it | unowned                               | M      |
| session-and-policy            | status, logout, session, ping, config, reload, capabilities, focus, input, privacy, launch, shortcut | MISSING        | None in `cli.rs`                                                                  | `rust-compositor.md:1052-1080`        | M      |
| windows                       | window list and get                                                                                  | BUILT          | Generic object verbs, `cli.rs:161-226`                                            | n/a                                   | S      |
| windows                       | window actions table `:123-138`                                                                      | MISSING        | Only generic `call` for focus and kill                                            | `rust-compositor.md:836`              | M      |
| windows                       | window thumbnail                                                                                     | MISSING        | No schema member                                                                  | `rust-compositor.md:836`              | M      |
| workspaces-monitors-shortcuts | workspace list                                                                                       | BUILT          | Generic object verbs, `cli.rs:161-226`                                            | n/a                                   | S      |
| workspaces-monitors-shortcuts | output list                                                                                          | BUILT          | Generic object verbs                                                              | n/a                                   | S      |
| workspaces-monitors-shortcuts | output renderer state                                                                                | PARTIAL        | `output.toml` has no renderer state                                               | `rust-compositor.md:1137`             | S      |

Extra `cli.rs` surface present and undocumented: `ipc dump` (`cli.rs:253-258`) and `log` (`cli.rs:291`) [INFERENCE that these are undocumented; not checked].

## gnoblinctl.md

| Item                                | Doc line    | Status  | Rust evidence or gap                | Design ref                           | Effort |
| ----------------------------------- | ----------- | ------- | ----------------------------------- | ------------------------------------ | ------ |
| `--version`, `--json`               | `:3-25`     | PARTIAL | `cli.rs:127-138`; no build identity | `rust-compositor.md:211-212`         | S      |
| `ping`                              | `:81`       | MISSING | None                                | unowned                              | S      |
| `logout`                            | `:53`       | MISSING | None                                | unowned                              | S      |
| `help window`                       | help verb   | MISSING | clap help only                      | unowned                              | S      |
| `window match`                      | window verb | MISSING | None                                | unowned                              | S      |
| `orientation-lock`, `input devices` | input verbs | MISSING | None                                | `rust-compositor.md:1137`            | M      |
| `layer list`                        | layer verb  | MISSING | None                                | `rust-compositor.md:177` [INFERENCE] | M      |

## dbus pages

| Item                                                                    | Doc line                             | Status         | Rust evidence or gap                                                                                               | Design ref                                                  | Effort |
| ----------------------------------------------------------------------- | ------------------------------------ | -------------- | ------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------- | ------ |
| `org.freedesktop.ScreenSaver` names and paths                           | `screensaver.md`; `lib.rs:62`, `:64` | PARTIAL        | Names and paths present                                                                                            | `rust-compositor.md:178`                                    | S      |
| `Inhibit(app, reason)`                                                  | `screensaver.md`                     | CHANGED        | `lib.rs:287`: empty app not defaulted to caller                                                                    | `rust-compositor.md:178` [INFERENCE]                        | S      |
| `UnInhibit` foreign cookie                                              | `screensaver.md`                     | CHANGED        | `lib.rs:300`: no-op, docs say AccessDenied                                                                         | `rust-compositor.md:854`                                    | S      |
| `GetActive`                                                             | `screensaver.md`                     | PARTIAL        | `lib.rs:306`: lock object only, no forward to `org.gnome.ScreenSaver`                                              | `rust-compositor.md:1171-1172`                              | S      |
| `SetActive`                                                             | `screensaver.md`                     | CHANGED        | `lib.rs:311`: always false; docs say true requests lock                                                            | `rust-compositor.md:854`                                    | S      |
| `Lock`                                                                  | `screensaver.md`                     | PARTIAL        | `lib.rs:326`: succeeds only once confirmed                                                                         | `rust-compositor.md:854`                                    | S      |
| `ActiveChanged` signal                                                  | `screensaver.md`                     | MISSING        | Declared `lib.rs:339`, not emitted                                                                                 | `rust-compositor.md:854`                                    | S      |
| `SimulateUserActivity`                                                  | `screensaver.md`                     | CHANGED        | `lib.rs:331`: NotSupported                                                                                         | `rust-compositor.md:854`                                    | S      |
| `GetActiveTime`, `GetSessionIdleTime`                                   | not documented                       | CHANGED        | `lib.rs:316`, `:321`: extras, undocumented                                                                         | n/a                                                         | S      |
| `org.gnoblin.SessionIdle` (`GetActivity`, `ActivityChanged`, 120000 ms) | `session-idle.md`                    | MISSING (Rust) | No crate; live C `src/idle.c:18-39`, `ActiveChanged` `:30`                                                         | `rust-compositor.md:1171-1172`; `shell-api-redesign.md:186` | M      |
| `org.gnoblin.Compositor` at `/org/gnoblin/Compositor`                   | `dbus/index.md:6`                    | MISSING        | No implementation in `src/` or `crates/`; `portal-policy-contract.md:30` cites `CheckPermission`, absent from both | unowned                                                     | M      |

## Caveats

- Rows cite design and code evidence from grep. Behaviour was not run.
- `[INFERENCE]` rows were not checked line by line.
- Per-page item lists are summarised. Each page has more members than shown; the status reflects the page's main surface.
