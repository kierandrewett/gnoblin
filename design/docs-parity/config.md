# Docs parity: config

Scope: `docs/config.md` and `docs/config/` (configure, configure/input, animation, array, config, load, snapshot, window_rule). Audit is read-only. Rust tree at `origin/rust-compositor` (crates/). MISSING means "not in the Rust candidate". The live Mutter C runtime in `src/` is a separate tree (`lua-api.md:1809`); where it already has the surface, the Evidence column cites it as `src/...:line`. Tags: BUILT, PARTIAL, MISSING, CHANGED. `[INFERENCE]` marks unverified claims.

## Summary

Counts cover the rows below, not every setting leaf. Leaf-level coverage of all 21 configure pages was not finished.

| Status  | Rows |
| ------- | ---- |
| BUILT   | 7    |
| PARTIAL | 11   |
| MISSING | 14   |
| CHANGED | 1    |

## Lua functions

| Item                               | Doc path:line                | Status  | Evidence / gap                                                                                                                                                      | Owner                                           | Effort |
| ---------------------------------- | ---------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------- | ------ |
| `gnoblin.configure`                | docs/config/configure.md:1   | CHANGED | Bound at crates/gnoblin-lua/src/configure.rs:43. Keys pass verbatim; no snake_case to kebab step found. Docs require snake_case.                                    | Unowned [INFERENCE]                             | S      |
| `gnoblin.config.path()`            | docs/config/config.md:17     | MISSING | Rust: no binding in gnoblin-lua. Live C: `src/config/lua/gnoblin-lua.c:50` (`config.path`).                                                                         | design/lua-api.md:187 (compat view, not a plan) | M      |
| `gnoblin.config.default()`         | docs/config/config.md:17     | MISSING | Rust: no binding. Live C: `src/config/lua/gnoblin-lua.c:51` (`config.default`).                                                                                     | Unowned                                         | M      |
| `gnoblin.config.restore_default()` | docs/config/config.md:17     | MISSING | Rust: no binding. Live C: `src/config/lua/gnoblin-lua.c:52`, an Operation (`CONFIG_RESTORE_DEFAULT`).                                                               | Unowned                                         | M      |
| `gnoblin.snapshot()`               | docs/config/snapshot.md:1    | MISSING | Rust: no binding. Live C: `src/config/lua/gnoblin-lua.c:55`. Doc says it errors at run time.                                                                        | design/lua-api.md:171 (snapshot shape only)     | M      |
| `gnoblin.array{}`                  | docs/config/array.md:1       | MISSING | Rust: no binding. Live C: `src/config/lua/gnoblin-lua.c:54`. Marker must survive Lua conversion.                                                                    | Unowned                                         | S      |
| `gnoblin.animation{}`              | docs/config/animation.md:1   | MISSING | Rust: no Lua binding; `gnoblin-animation` crate exists (easing, property, transform). Live C: named view `animation` at `src/config/lua/gnoblin-lua-install.c:312`. | Unowned [INFERENCE]                             | M      |
| `gnoblin.window_rule{}`            | docs/config/window_rule.md:1 | PARTIAL | Rule types and matching at crates/gnoblin-wm rules.rs:166-249; apply_rules wm.rs:1157. No Lua binding. RulesChanged has no producer outside tests.                  | In-flight lane [INFERENCE]                      | L      |
| `gnoblin.load()`                   | docs/config/load.md:1        | PARTIAL | MAX_LOADS=32 at crates/gnoblin-lua/src/host.rs:48; loader confinement in loader.rs. Recovery pages not verified.                                                    | design/lua-api.md [INFERENCE]                   | S      |
| Config nesting limit               | docs/config/configure.md     | PARTIAL | MAX_DEPTH=64 exists but is undocumented.                                                                                                                            | Unowned                                         | S      |

## Configure: top-level keys

| Item                                                                                                     | Doc path:line               | Status  | Evidence / gap                                                                                                                                                                                                  | Owner   | Effort |
| -------------------------------------------------------------------------------------------------------- | --------------------------- | ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------- | ------ |
| 21 documented sections                                                                                   | docs/config/configure.md:20 | BUILT   | Accepted at crates/gnoblin-config/src/document.rs:149 (TOP_LEVEL_KEYS).                                                                                                                                         | n/a     | n/a    |
| Undocumented sections: layout, workspaces, logging, compat, animations, window-rules, extensions         | schema only                 | PARTIAL | Schema accepts them; no config page. Animations and window-rules have their own pages.                                                                                                                          | Unowned | S      |
| Carried, not validated: animations, autostart, extensions, keybindings, portals, shortcuts, window-rules | document.rs:21-24           | PARTIAL | Passed through without validation in the config crate.                                                                                                                                                          | Unowned | M      |
| `gnoblinctl config` subcommands                                                                          | docs/config/config-files    | MISSING | Rust: no config, init, reload, shortcut or input subcommands in crates/gnoblinctl. Live C: `src/cli/gnoblinctl-cli.c:27` (`config`: path default show reload restore-default), `:28` (`init`), `:32` (`input`). | Unowned | M      |
| IPC `config.commit`, `config.current`                                                                    | IPC docs                    | BUILT   | crates/gnoblin-proto wire.rs:92-94.                                                                                                                                                                             | n/a     | n/a    |

## Configure: sections

| Item                                 | Doc path:line                                | Status  | Evidence / gap                                                                                                              | Owner                        | Effort |
| ------------------------------------ | -------------------------------------------- | ------- | --------------------------------------------------------------------------------------------------------------------------- | ---------------------------- | ------ |
| `frame-renderers`                    | docs/config/configure/frame_renderers.md     | PARTIAL | Parsed in gnoblin-config only. No runtime consumer found.                                                                   | Unowned                      | M      |
| `layer-shell.preserve-active-window` | docs/config/configure/layer_shell.md         | MISSING | No consumer outside gnoblin-config.                                                                                         | design/layers.md [INFERENCE] | S      |
| `touchpad-gestures`                  | docs/config/configure/touchpad_gestures.md   | BUILT   | Consumed at crates/gnoblin-input gesture.rs [INFERENCE].                                                                    | n/a                          | n/a    |
| `location`                           | docs/config/configure/location.md            | BUILT   | crates/gnoblin-service-location lib.rs:163.                                                                                 | n/a                          | n/a    |
| `logging`                            | docs/config/configure (logging)              | BUILT   | crates/gnoblin-compositor log_store.rs uses gnoblin_config::Logging.                                                        | n/a                          | n/a    |
| `portals`                            | docs/config/configure/portals.md             | MISSING | No config hit in xdg-desktop-portal-gnoblin lib.rs [INFERENCE].                                                             | Unowned                      | M      |
| `autostart`                          | docs/config/configure/autostart.md           | MISSING | No config to session wiring found [INFERENCE].                                                                              | Unowned                      | M      |
| `auth.polkit_agent`                  | docs/config/configure/auth.md                | PARTIAL | Key is accepted in the legacy tree. Rust consumer not confirmed [INFERENCE].                                                | Unowned                      | S      |
| `prompts`, `keyring`                 | docs/config/configure/prompts.md, keyring.md | PARTIAL | Read via environment in gnoblin-services [INFERENCE]. Not traced end to end.                                                | Unowned                      | S      |
| `window-management`                  | docs/config/configure/window_management.md   | PARTIAL | crates/gnoblin-compositor/src/window_management.rs:62-101 maps 11 leaves plus TitlebarActions::of. The rest are not mapped. | Unowned                      | M      |
| `keyboard` repeat                    | docs/config/configure/keyboard.md            | BUILT   | session_tables.rs:180-184.                                                                                                  | n/a                          | n/a    |

## Configure: input

| Item                      | Doc path:line                  | Status  | Evidence / gap                                                                            | Owner   | Effort |
| ------------------------- | ------------------------------ | ------- | ----------------------------------------------------------------------------------------- | ------- | ------ |
| `input.tablets` routing   | docs/config/configure/input.md | PARTIAL | Tablet tool events are dropped at crates/gnoblin-backend-drm/src/libinput/mod.rs:405-412. | Unowned | M      |
| Tablet `orientation_lock` | docs/config/configure/input    | PARTIAL | Always None in the Rust tree.                                                             | Unowned | S      |
| Stylus settings           | docs/config/configure/input    | MISSING | No hits [INFERENCE].                                                                      | Unowned | M      |

## Animation and shortcuts

| Item                                                                                 | Doc path:line                      | Status  | Evidence / gap                                                                               | Owner   | Effort |
| ------------------------------------------------------------------------------------ | ---------------------------------- | ------- | -------------------------------------------------------------------------------------------- | ------- | ------ |
| Animation events: minimize, dialog-open, layer-open, workspace-switch, shadow-change | docs/config/animation.md           | MISSING | Names not found in any crate.                                                                | Unowned | M      |
| Easing `ease-out-expo`                                                               | docs/config/animation.md           | PARTIAL | Present at crates/gnoblin-animation/src/easing.rs:39 (EaseOutExpo). Other names not checked. | Unowned | S      |
| Shortcut `trigger`, `binding`, `command`                                             | docs/config/configure/shortcuts.md | BUILT   | Router has binding and inhibit at gnoblin-input router.rs [INFERENCE].                       | n/a     | n/a    |
| Shortcut `capture_input`                                                             | docs/config/configure/shortcuts.md | MISSING | Only a portal-side remote.rs hit.                                                            | Unowned | M      |

## Gaps: top ten

1. `gnoblin.window_rule` not bound (MISSING). Owner: in-flight WindowRules lane [INFERENCE].
2. `gnoblin.animation` not bound, and five documented events absent. Unowned.
3. `gnoblin.config`, `gnoblin.snapshot`, `gnoblin.array` not bound. Unowned.
4. `gnoblin.configure` snake_case keys not converted (CHANGED). Unowned; needs confirmation.
5. `gnoblinctl config` subcommands absent. Unowned.
6. `frame-renderers` has no runtime consumer. Unowned.
7. `layer-shell.preserve-active-window` has no consumer. Owner: layers lane [INFERENCE].
8. `portals` and `autostart` not wired. Unowned.
9. `capture_input` on shortcuts absent. Unowned.
10. Seven schema sections undocumented. Unowned.

## Open checks

- Confirm snake_case to kebab conversion in `gnoblin.configure` with a live test (not run; read-only audit).
- Trace `prompts`, `keyring` and `auth.polkit_agent` consumers.
- Leaf-level review of all 21 configure pages was not completed. Rows above cover section level only.
