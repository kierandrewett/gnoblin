# Changelog

## Unreleased

### Changed

- The default shell moved from `data/js/starter/shell.ts` to `shells/default/shell.ts` (lane NpStarterMove; design/native-std-port.md, O7). It is a hand-written user shell, not part of the library, so it no longer sits under `data/js`. `cargo xtask dist` still installs it as `share/gnoblin/default-shell/shell.ts`, the packaged-default tier of discovery looks for it at the new path in a source tree, and `scripts/js-typecheck.sh` no longer excludes it.

#### Compositor and rendering

- The three standard capture protocols, `ext_image_capture_source_v1`, `ext_image_copy_capture_v1` and `zwlr_screencopy_v1`, are crates of their own under `crates/protocols/` (lane CaptureMove; design/protocol-crates.md). They advertise the same globals at the same versions and in the same order, and the capture engine and the View source stay in `gnoblin-wayland`.
- Window fade and placement animations now come from `libgnoblin-std` rather than the core, so core-only sessions no longer fade windows, while std sessions look unchanged.
- Frame callbacks now go only to windows shown by a visible `View` or on a monitor; mapped windows on neither no longer receive them.
- Window opacity from `wp_alpha_modifier_v1` and window rules now combines into the window's scene node, and a closing window keeps the opacity it had when its close fade began.

#### Windows, workspaces and shell

- The `DwindleLayout` split direction now follows its cell's shape on every layout, so taller cells split top and bottom, unless `preserveSplit: true` or an explicit `split` is set.
- Window rules are now the only source of a window's `opacity` and `maximised`, so arranging the carousel no longer overwrites a rule's opacity, and a `maximised` rule keeps the floating rectangle.
- Lua `gnoblin.find(kind, filter?)` lists live objects by kind with table or function filters, `gnoblin.config` is reduced to `path()`, `default()` and `restore_default()` with errors pointing to `gnoblin.configure`, and several Lua animation and window APIs changed accordingly.
- Focus, move, resize and snap actions now share one scoped `Grant` that `event.grant` provides, so each operation of a gesture can run once, and `event.focus_context` and `event.menu_context` are removed from Lua.

#### Input, seats and accessibility

- The `gnoblin-xr` pointer's seat is now named `xr` instead of `gnoblin-xr-pointer`, reusing an existing `xr` seat when one is present.
- The stock shell now receives pointer and touch input on windows, so `Window:on_enter`, `on_motion`, `on_leave`, `on_press` and `on_release` run on the real compositor and focus follows them, while clients still get the input.
- Replaces `gnoblin.input.select_source()` in Lua with `InputSource:select()`, where `input.sources()` and `input.current_source()` return `InputSource` objects with `changed`, `selected` and `closed` signals.

#### Session, services and recovery

- The strike policy, file watcher, spawn handling and reload logic moved from `gnoblin-js` into `gnoblin-shell-host`, and the JavaScript shell behaves as before.
- Attaching the tree shell from a connection lacking the owner grant now fails with `not-owner` (`failed_op` -1) instead of `not-privileged`, while transactions from non-holders still return `not-privileged`.
- IPC owner-only commands like `config.commit`, `portal.ui` and `capture.screen` are now refused to any connection not owned by the session user, which is checked via socket peer credentials; the `runtime`, `services`, `portal` and `input` IPC roles have been removed.
- A Lua handler that throws no longer stops later events: Gnoblin logs the error with the event name and Lua stack, and the other handlers of the event still run.
- Calls to `gnoblin.session.suspend()`, `hibernate()`, `reboot()` and `poweroff()` now report logind's actual answer, returning `{accepted = true}` or an error such as `denied`, `unsupported`, `busy`, `unavailable`, `timed_out` or `internal`.

#### Configuration

- Custom shaders are now enabled by default; set `compositor.user-shaders = false` to switch them off, which takes effect at the next login.
- `Output.apply` and `Output.mirror` now return an `Operation<Output>` that succeeds with the output or fails with the compositor's error message and code, replacing the polled `OutputApplication` with `done` and `error`.
- The shell config file is now `shell.ts`, found by discovery, and `GNOBLIN_SHELL` replaces `GNOBLIN_CONFIG`; `init.ts`/`init.js` are renamed to `shell.ts`/`shell.js` with no alias.
- The stock JavaScript library is now split into `gnoblin-core` (host bridge and core object types) and `gnoblin-std` (desktop policy); the bare `gnoblin` specifier and `gnoblin/<module>` imports are removed.
- The stock desktop is now defined only by `data/js/starter/init.ts`, which runs when no `init.ts` exists or fails to load and is written by `gnoblinctl config init`; `installDefaults` and `loadDefault` are removed, so custom configs should copy the starter.
- Sets `client.frameInset` to a four-sided `FrameInset` (`top`, `right`, `bottom`, `left`) instead of a titlebar height, so layout, placement, drags and snaps keep room on all sides.
- An `init.ts` now fully replaces the default setup, so without it the starter defaults apply, while a present file must call `installDefaults(gnoblin)` or copy the starter statements to keep them.
- The TypeScript shell is now the default, and it owns the whole configuration: `init.ts` or `init.js` in `~/.config/gnoblin` is used, and a session with only `init.lua` keeps the Lua shell with a warning.
- Layouts with `extent = "unbounded"` must now declare `kind = "tiling"`, and placements outside the output or past the coordinate bound refuse the whole reply.
- Key and pointer chords now share one grammar across configuration, Lua, `configure.shortcuts` and stylus bindings: modifiers go in angle brackets, such as `<Shift><Control><Alt><Meta><Super><Hyper>`, and malformed chords are rejected with the correct form to write.
- A `Process` now emits `closed` like Window, Workspace and Output, with `alive`, `status`, `exit_code` and `signal` describing how it ended, while `wait()` and `kill()` remain.
- Orientation lock and privacy screen now use `gnoblin.settings.override`, replacing the removed Lua setters; socket clients keep the wire methods, which end on config reload and fail when no output has a privacy screen.

#### Portals, capture and privacy

- The `org.gnoblin.Shell.GlobalShortcutsProvider` BindShortcuts and ConfigureShortcuts methods and the `org.gnoblin.Shell.GlobalShortcutsRebind` RebindShortcuts method now take a trailing request handle, and calls with the old argument lists are no longer supported; deploy the portal backend and shell provider together.

#### Client protocols and compatibility

- The JavaScript API now uses one set of snake_case error codes via `ErrorCode` from `gnoblin/errors`, replacing the mixed kebab-case, snake_case and host-specific codes.
- The JavaScript API now takes all times in milliseconds and uses CSS easing names (`linear`, `ease`, `ease-in`, `ease-out`, `ease-in-out`), replacing seconds and the old `Client.animate` easing names; `gnoblin.after` is replaced by `gnoblin.timers.after`.
- The IPC handshake no longer sends a grants frame, so the accept frame is the whole preamble and clients must be rebuilt because the version stays at 0.2.0.
- X11 windows now appear to the shell as `window` nodes like Wayland toplevels, with title, app ID, size and focus published, and closing them ends their captures and frames; servers older than Xwayland 23.1 still get no node.
- Xwayland windows now get Gnoblin's own frame, rules and effects like Wayland windows, with `frame.mode = "auto"` as the default for X11 windows that do not disable decorations.
- Gnoblin's control wire operations now come from one shared table, with no wire change and the API still at version `1.92`.
- Live `Tween`, `Process` and `AnimationPreview` objects now share one subscription model: `on("closed")` reports the outcome, `watch` accepts only changeable fields, `gnoblin.input.on` returns a `Subscription` you can `unsubscribe()`, and a config reload ends all runtime-owned objects.

#### Command line and tooling

- The isolated shell now logs under the role `shell` instead of `runtime`, shown as `shell/component` on the console, `gnoblin-shell` in the journal, `GNOBLIN_ROLE=shell`, and selectable with `gnoblinctl log --role shell`; older records keep `runtime`.
- Log records are now written to `gnoblin.jsonl` in the state directory, read by `gnoblinctl log` and `gnoblinctl log --follow`; the `gnoblinctl store` command and `--store` flag are removed.
- Log messages no longer start with a `[tag]` prefix; a tag matching the component is dropped, and other tags become an `area` field on the event.
- Patches are now grouped into topic directories, each patch project's `series` file sets apply order, and `scripts/manage-patches.py` gains `renumber` and `path` commands plus stricter `check`.
- `gnoblinctl lua` now matches the names and call shapes of a Lua configuration, with runtime-only names rejected by a clear error.

#### Packaging and install

- Packaging files for RPM, PKGBUILD and openSUSE were removed, while package names, Fedora epoch, Provides and Obsoletes stay unchanged so existing installs upgrade; the portal backend is now version `51.0.0`.
- COPR no longer receives releases; releases drop source RPMs, PKGBUILDs and `gnoblin.db`, and `scripts/install-system.sh` is removed, so use the package repositories or release packages instead.
- Running `scripts/prepare-release.sh` now sets the version only in `src/gnoblin-version.json`.
- The Nix package and module now live in `nix/` instead of `packaging/nix/`, and the flake's outputs are unchanged.
- Packages now accept pre-release versions: the source bundle keeps the SemVer spelling (`0.1.13-nightly.…`), while RPM packages use `0.1.13~nightly.…`, and release versions are unchanged.

#### Documentation and tests

- Duplicated internal code is consolidated, and `gnoblinctl config migrate` now reads its stock desktop from the starter `shell.ts` file rather than a separate copy.
- The JavaScript shell's frame stages are now listed in a table with their ordering requirements recorded and tested, with no change in behaviour.
- The XR library is now imported as `gnoblin/extensions/xr`, which runs the XR availability check; the old `gnoblin/xr`, `gnoblin/xr_bindings` and `gnoblin/xr/*` paths are removed, and `gnoblin/extensions/xr/exports` provides the same names without the check.
- Archived the JavaScript runtime logs and Lua parity audit, and `design/js-runtime.md` now describes the runtime as it currently exists.

### Added

- The Rust SDK of `libgnoblin-std` calls the generated classes for workspaces, clients, collections, layouts and dividers, and example 34 with `modular::native_workspaces` (lane NpStdObjects, slice 4; design/native-std-port.md, 2.10; `crates/gnoblin-std/src/{workspace,collection,client,client_state,desktop,layout,builtin,split,subscription}.rs`, `examples/34-native-workspaces`). `Workspace`, `Collection`, `Client` and `Layout` keep their names and go through the object protocol, so Rust, Python and TypeScript see one set of workspaces. A selection of clients takes the keys of `client_query`. The example makes two workspaces, gives the first each layout of the library and one of its own, drags a divider with a pointer binding of the core, and a stock `ext_workspace_manager_v1` client reads the active workspace.
- The window policy is native in `libgnoblin-std` (lane NpStdPolicy; design/native-std-port.md, JS6 and JS15; `crates/gnoblin-std-impl/src/policy/`, `crates/gnoblin-std-abi/schema/policy.toml`, `examples/32-native-policy`). `focus_policy` runs the focus modes, raise on click, auto raise and the pointer rest from the seat's settings; `window_state` routes the requests of windows to listeners the shell adds (the first that handles one answers, the stock answer runs last) and answers maximise, fullscreen, minimise, activate, raise, lower, close, move and resize; `repositioning` moves and resizes a window with the arrow keys; `snap` names the edge zones; `move_binding`, `resize_binding`, `swap_binding`, `split_resize_binding`, `swipe_action_binding`, `pinch_action_binding` and `workspace_slide_binding` install on an input context of the core. Each service does nothing before its `start`, so a shell with a policy of its own is not answered twice. The window menu's Move and Resize use the keyboard modes when they are started. The TypeScript policy under `data/js` is unchanged until the switch.
- Output profiles, displays and viewports are native in `libgnoblin-std` (lane NpStdDisplay; design/native-std-port.md, JS10; `crates/gnoblin-std-impl/src/display_objects/`, `crates/gnoblin-std-abi/schema/display.toml`, `examples/33-native-displays`). `output_profile` (`match`, `id`, `settings`, `format_mode`), `viewport` (`content`, `camera`, `clear_color`), `display` (`selectors`, `mode`, `outputs`, `viewport`) and the `display_manager` service (the lists, add, remove, replace, `display_at`, `snapshot` and `restore`, and the signals `changed`, `applied` and `noted`) are classes of the object protocol over the state the std table's `profile_*` and `display_*` functions use. The checks and their words are the TypeScript classes'. A transition that does not commit puts back the workspace each desk shows, the displays' waiting workspaces and the viewports' cameras and colours. `tests/displays.rs` replays the 24 cases of the TypeScript display tests, and `modular::native_displays` runs example 33 in a real session. The generated Rust API writes a property named with a keyword as a raw identifier (`r#match`) and builds its helpers from the plain name.
- The not-responding alert, the launcher, the overview and the stock lock screen are native in `libgnoblin-std` (lane NpStdShellUi; design/native-std-port.md, JS14; `crates/gnoblin-std-impl/src/{shell,lock}/`, `crates/gnoblin-std-abi/schema/{shell,lock}.toml`, `examples/31-native-shell-ui`). `not_responding_alerts` looks at the compositor's windows, opens a `not_responding_dialog` for one that stops answering (Escape waits, Tab, Left and Right move between the buttons, Force quit ignores a press until `arm_ms` has passed and then asks the window's own `kill`), and `alert_tracker` holds the rule of which windows get one. `launcher` (`open`), `overview` (`toggle`), `lock_screen`, `password_entry` and `lock_style` are classes of the object protocol; the lock screen sends the password through the core's `session_lock`, overwrites the characters it held, and a shell that makes none leaves the lock to the compositor's card. `tests/shell_ui.rs` replays nine of the ten `parts_*` fixtures, and `modular::native_shell_ui` runs example 31 from Rust, TypeScript and Python.
- The pointer and gesture side of the input matcher is native in `libgnoblin-core`, with the device classes and their settings (lane NpCoreInput, part 2; design/native-std-port.md, 5.3 and 5.4; `crates/gnoblin-core-impl/src/model/input/{pointer,gesture,hooks,settings}.rs`, `crates/gnoblin-core-abi/schema/input.toml`, `examples/30-native-pointer`). `bind` takes a function for a drag, a click or a swipe, `bind_drag` and `bind_gesture` take an object that implements `pointer_drag_binding` or `gesture_binding`, and `binding_event` carries `phase`, `dx`, `dy`, `button`, `progress` and the other fields of a drag or a gesture. A press past the threshold is offered to the candidates in order, a window that closes, a binding that goes or a reload cancels the drag, and a drag in flight ends from the shutdown hook of the object library while the shell's code is still mapped. A device is a `keyboard`, `mouse`, `touchpad`, `touchscreen` or `tablet`; the settings of a kind are one ledger, written as `input_manager.document`. `tests/input.rs` replays `input_drags` and the three tiling fixtures that drag or gesture.
- The key side of the input matcher is native in `libgnoblin-core` (lane NpCoreInput, part 1; design/native-std-port.md, 5.3 and 5.4; `crates/gnoblin-core-impl/src/model/input/`, `crates/gnoblin-core-abi/schema/input.toml`, `examples/28-native-input`). `gnoblin.input` has a context for each seat, `bind` takes a function (`false` declines) or an object the shell implements, bindings of one trigger share one claim, a press runs device, layout then context bindings by priority, and `triggers.parse` is the grammar for every language. A chord a staged generation binds is held until it takes over and goes out behind the old generation's release. `listen::claim_with` makes a claim for a native handler. Replays of the `input_grammar` and `input_chords` fixtures, `modular::native_input` from Rust and Python.
- The window frame is native in `libgnoblin-std` (lane NpStdFrame; design/native-std-port.md, JS12 and 4.4; `crates/gnoblin-std-impl/src/frame/`, `crates/gnoblin-std-abi/schema/frame.toml`). `window_frame` with its four kinds of outlet (edges with start, centre and end zones, corners, the underlay and the overlay), `decorations` (a frame for each window, the shell's `frame` callback or the stock titlebar, rules, hooks for the shell's window policy, and the room each frame reserves told to the library), `titlebar` (`install`), `window_actions` (close with its 180 ms fade and the restore after 1500 ms, minimise, maximise and the seat's click actions), `window_drag` and the parts (`app_icon`, `drag_area`, `frame_backdrop`, `frame_border`, `frame_glow`, `resize_handle`, `resize_handles`, `title_text`, `window_button`, `window_buttons`). `tests/frame.rs` replays the 19 recorded `frame_*` fixtures through the classes, paint bytes included, and example 27 (Rust, TypeScript and Python) with `modular::native_frame` runs it in a real session.
- The window menu is served by `libgnoblin-std` (lane NpStdMenus; design/native-std-port.md, JS13; `crates/gnoblin-std-impl/src/menus/`, `crates/gnoblin-std-abi/schema/menus.toml`). `window_menus` (the service that answers a client's `menu` request, opens one card at a time, gives the keyboard back and keeps "Always on top" windows above the others), `menu_entry`, `menu_provider` (a shell's own entries, or its own card), `menu_navigator`, `menu_item`, `menu_panel` and `window_menu` (canvases of the toolkit) and `menu_metrics` are classes of the object protocol. The stock entries read the library's workspaces and client records; Move and Resize take the policy's keyboard modes as `move_with_keys` and `resize_with_keys`. `tests/menus.rs` replays the 11 menu oracle fixtures and the paint files against the library (allowances in the design); example 29 and `modular::native_menus` run it from Rust, TypeScript and Python over a real session, with a client that asks for its menu through `show_window_menu`. The modular harness gains a virtual pointer (`Pointer`) and `Mapper::show_window_menu`.
- Every Python example type-checks clean with `pyright` against the installed stubs (lane NpPyRuntime; docs/python-shell.md; `crates/gnoblin-sdkgen/src/{object_py,python_std,layouts}.rs`, `crates/gnoblin-runtime-python/tests/shells.rs`). The object stubs read an object or a record as `| None` only where the catalog marks it `optional`, a derived class names its base, a std call that answers a handle no longer answers `Optional` unless it is a lookup, and a built-in binding has `start`. `every_python_example_type_checks_clean_against_the_installed_stubs` runs `pyright` over every `examples/*/shell.py` and the golden shell when `pyright` is on the path or `GNOBLIN_PYRIGHT` names it, and fails on any error.

- The `gnoblin` Python package is Rust and `data/python` is deleted (lane NpPyRuntime; design/native-std-port.md, 2.3, PY2 and PY3; docs/python-shell.md; `crates/gnoblin-runtime-python/src/{loader,logging,descriptors,singleton,subscription,values,finder,std_base,std_sugar,methods}.rs`). The loader that isolates each generation (a fresh compile of the script and of every module beside it, `sys.modules` purged and restored, `__main__` as under `python shell.py`), the routing of Python's reporting into the core's log, the `Node`, `Property`, `Method` and `Signal` descriptors, the `gnoblin` object with its named subscriptions, `Subscription`, the event values, the exceptions, and the base and spellings of `gnoblin.std` are classes and functions of the runtime library, served to its interpreter by a finder on `sys.meta_path` beside the generated modules. No file of the package is read, and `GNOBLIN_PYTHON_DIR` is gone. `cargo xtask dist` installs a stub package to `share/gnoblin/types/python/` (the runtime crate's stubs in `crates/gnoblin-runtime-python/stubs/`, the generated ones, `py.typed`) instead of the source tree that was `share/gnoblin/python/`. `check_stubs.py` is replaced by `the_stubs_of_the_package_agree_with_what_the_runtime_reflects`, which holds the stubs to what the runtime reflects of every public name, and `the_package_is_served_by_the_runtime_and_no_file_of_it_is_read` proves nothing is read with a poisoned `GNOBLIN_PYTHON_DIR` in the environment. A reload still compiles from source, purges and restores the modules (`modular::script_reload`, `reload_scripts`, `tests/shells.rs`).

- The workspaces, desks, clients' library state and layouts of the TypeScript std are classes of the object protocol in `libgnoblin-std` (lane NpStdObjects, slice 1; design/native-std-port.md, JS3 and JS5; `crates/gnoblin-std-impl/src/workspace_objects/`, `crates/gnoblin-std-abi/schema/workspace.toml`). `workspace`, `workspaces`, `desks`, `client_collection` and `client_state` work over the state the std table's functions use, and a shell implements the interface `layout` to place windows. The built-in layouts are classes generated from `LAYOUT_KINDS` (`crates/gnoblin-sdkgen/src/layout_schema.rs`), each extending `layout`. The TypeScript modules stay until NpFlip. `tests/objects.rs` replays nine tiling oracle fixtures through the classes (`shell-objects`); example 26 and `modular::native_objects` run a workspace, a layout of the shell's own and a divider from Rust, TypeScript and Python.
- A transition that does not commit puts back the workspaces' layouts, gaps and names with the styles; `view.show_workspace` and `view.show_client` take a workspace object and a client's id; a shell that uses `gnoblin.workspaces` has the library write the seat's `active_workspace`, so `ext-workspace-v1` clients see the workspace that is shown (lane NpStdObjects, slice 3).
- The styles and transitions of the TypeScript std are native in `libgnoblin-std` (lane NpStdStyle; design/native-std-port.md, JS4, JS7 and JS8; `crates/gnoblin-std-impl/src/{style/objects.rs,style/edit.rs,animation/objects.rs,animation/staging.rs}`, `crates/gnoblin-std-abi/schema/{style,transition}.toml`). `style` (one object for each scope: every window, a workspace's windows, a `where` selection or one window; copy-on-write lists through `shadow_list`, `border_list` and `filter_list`; `computed` and `warnings` of a window) and `transition` (`begin`, `set`, `peek`, `commit`, `abort`, `request`, `animate`, the controls and the `finished` signal) are classes of the object protocol over the state the std table's functions use. A transition begun through them holds an `abort` callback: when its callback fails, its commit is refused or it is aborted, the library puts back every style and calls the shell's `rollback` before the failing call answers. `tests/style.rs` replays the 20 style and transition oracle fixtures; example 25 and `modular::native_style` run the classes from Rust, TypeScript and Python. The TypeScript modules stay until NpFlip; their importers are listed in the design.

- The scene, its math and property bindings of the TypeScript std are native in `libgnoblin-std` (lane NpStdScene; design/native-std-port.md, JS9 and JC7; `crates/gnoblin-std-impl/src/{scene,math}/`, `crates/gnoblin-std-abi/schema/{scene,math}.toml`). `scene_node`, `view` (of a client, or of a workspace through a native mirror), `camera3d`, `camera2d`, the `scene` service, `node_bindings` (dependency-tracked, across this library's objects and the object model's) and `geometry` (vectors, rays, planes, poses, quaternion and transform checks, rounded as the fixtures round them) are classes of the object protocol. `tests/scene.rs` replays the ten scene oracle fixtures against the library; example 24 and `modular::native_scene` run it from Rust, TypeScript and Python. The typed Rust SDK gives each derived class `as_<base>()`, so `scene.add(&view.as_scene_node()?)` works. The TypeScript scene and math stay until NpFlip: host glue still imports them.

- The session services of the core are native (lane NpCoreServices; design/native-std-port.md, JC4 and 6.4; `crates/gnoblin-core-impl/src/model/{session,process,idle,launches,dialogs,lock,virtual_output,inset}.rs`, `crates/gnoblin-core-abi/schema/{session,process,idle,launches,dialogs,lock,virtual_output,inset}.toml`, `crates/gnoblin-core-impl/src/slots.rs`). `libgnoblin-core` serves `session` and `runtime` (verbs, lock, reload, signals, build identity), `processes` and `process`, `idle` watches and holds, `launches`, `dialogs` and `authentication`, `session_lock`, `virtual_output` and `insets`, and the `gnoblin` service reaches them. The compositor's dialog slot is fail-closed across a reload and after the policy shell has gone: the claim outlives a generation, a request nobody answers within 150 ms is refused and never served by the built-in dialog, a failed replacement leaves the previous provider in place, and `slots::AFTER_GRACE` (default `Deny`) decides what happens after ten seconds with no policy shell. The TypeScript library under `data/js` is unchanged until the flip.

- The native core object model (lane NpCoreModel; design/native-std-port.md, 4.4 and 6.4; `crates/gnoblin-core-impl/src/model`, `crates/gnoblin-core-abi/schema/{collection,client,focus,operation,properties}.toml`). `libgnoblin-core` serves windows, outputs, live collections with `where`, the `gnoblin` service (roots, `find_clients`, `find_outputs`, timers, `now`), the focus history, operations, timers and property bindings as the classes of `gnoblin.model`. A listener of a window is told before the listeners of the collections that hold it, a listener added to `added` is told of the members there from the next turn, a facade signal (`client_title_changed`) stands in the collection's own list, and an object that is gone keeps its last values and refuses a write. `gnoblin-object` gains `redirect` and `replay` on a class, `Cx::listen`, `Cx::unlisten`, `Cx::later_on`, `Library::run_in`. `crates/gnoblin-core-impl/tests/model.rs` replays the 13 object-model oracle fixtures in order. The TypeScript model stays until the flip.

- The UI toolkit of the TypeScript std is native in `libgnoblin-std` (lane NpStdUi; design/native-std-port.md, JS11; `crates/gnoblin-std-impl/src/ui/`, `crates/gnoblin-std-abi/schema/{ui,widgets}.toml`). `canvas`, `box` (`UiBox` in Rust), `grid`, `text`, `icon`, `entry`, `clock`, `bar` and `toast`, the `ui` session functions and the `widget` interface (a shell's own refresh, arrange, paint, natural size and handlers) are classes of the object protocol. Layout, text measured in the library's process by `gnoblin-paint` (decision O9), the paint list's encoding, pointer and key bubbling, key focus, the update pass and the widgets' wake timers are Rust. `tests/ui.rs` replays the eight UI oracle scenarios against the fixtures; `tests/ui_api.rs` covers the rest. `gnoblin-object` gains `Library::with_cx` and `Cx::unpin`. The TypeScript toolkit is not deleted: frame, menus, shell parts and the host glue still import it (NpFlip).

- The object protocol of the native std port, ABI batch B1 (lane NpObjectAbiA; design/native-std-port.md, 4.1, 4.2 and 6.1; docs/abi-compatibility.md; `crates/gnoblin-object`). Both libraries serve classes, properties, methods and signals from a catalog through one table, `GnoblinObjectApi`, got from a function appended to `GnoblinApi` and `GnoblinStdApi` (`object_api(ctx, namespace, out)`), so a class added to a catalog is a schema file and a Rust implementation and needs no function in either table and no edit of `abi.lock`. The values it carries are new kinds of `GnoblinValue` (`VALUE_OBJECT`, `VALUE_CLOSURE`, `VALUE_ENUM`, `VALUE_NONE`, and `VALUE_RECORD` now says its class in `bits`). `gnoblin-object` is the one implementation of both ends (the value and its conversion, the server a library builds its classes with, the client an SDK or a runtime calls through), `gnoblin-schema` reads the keys of a local namespace (`local = true`, `kind`, `[[enum]]`, `[[callback]]`, `returns`, `ctor`, `inert`, `abi`, `export`, a base of another namespace), and `gnoblin-sdkgen` writes the static catalog of `gnoblin.model` and `gnoblin.std` into `OUT_DIR` and one surface lock per catalog file (`crates/gnoblin-{core,std}-abi/schema/lock/`). The first classes are `entry_model` and the focus decisions of `focus_policy`, ported from the TypeScript library; nothing else of the std moves. The three golden shells still match `golden.lock`. `GnoblinObjectApi` also has `track(ctx, cb, user)`, a tracker told every property read of a library, which is how a property binding learns its dependencies across libraries, and a class may extend a class the other library serves: its catalog lists the base's members first and the library forwards them to a base object it makes with its own.

- The typed Rust API of the object protocol (lane NpObjectAbiA; design/native-std-port.md, 4.2 and 2.10; docs/sdk-generator.md; `crates/gnoblin-sdkgen/src/object_api.rs`, `crates/gnoblin-object/src/typed.rs`). `gnoblin-sdkgen` has a target `object-api` that writes, for each namespace a library serves, a Rust type for every catalog type over the protocol's client: an enum, a record, a handle or service that releases its reference when dropped (a getter and a setter for each property, a method for each method, `on_<signal>` and `on_<property>_changed` listeners that end with their guard), an interface as a struct of handlers, and a `callback` as a Rust closure. A shell reaches them with `g.objects()` (`gnoblin_core::objects`, the namespace `gnoblin.model`) and `g.std()?.objects()` (`gnoblin_std::objects`, `gnoblin.std`). A property of a record in a local namespace may say `optional = true`, which the typed API reads as an `Option` and the surface lock records. `gnoblin-object` is part of the SDK tier, `crates/gnoblin-object/fixtures/probe` renders and runs one class of every kind, example 20 (`examples/20-native-object`) uses an entry model and a focus decision from a real session, and `modular::native_object` runs it.

- The object protocol from JavaScript, TypeScript and Python (lane NpObjectAbiB; design/native-std-port.md, 4.2, 3.3 and 6.4; docs/sdk-generator.md, docs/typescript-shell.md, docs/python-shell.md; `crates/gnoblin-runtime-js/src/object_native.rs`, `crates/gnoblin-runtime-python/src/object_native.rs`, `crates/gnoblin-sdkgen/src/object_{script,js,py}.rs`). `gnoblin-sdkgen` has three new targets, `object-js`, `object-py` and `object-types`, that write the classes of each namespace a library serves for the script languages from the same catalog the Rust API is written from: an enum is a string-literal union (an `IntEnum` in Python), a record an interface (a `dataclass`), a handle a class with accessors, methods, `on(event, handler)` and `destroy()`, a service a class of functions, an interface a base class a shell extends (the methods it defines are the ones the library calls), and a callback a function. The classes are embedded in `libgnoblin-runtime-js.so` (`gnoblin-std/generated/objects`, `gnoblin-core/generated/objects`) and `libgnoblin-runtime-python.so` (`gnoblin.objects.std`, `gnoblin.objects.model`) and call the new `__gnoblinObjects` and `_gnoblin_objects`, two generic doors to `gnoblin_object::client::Api` that know no class: a library object is a slot (JavaScript, released from a `FinalizationRegistry`) or a handle (Python, released on the shell thread), a script function is a closure the library calls from the core's dispatch, and an exception in one is logged with its stack or traceback and fails that call alone. The declarations (`objects.d.ts`, `objects.pyi`) come from the same catalog; `editor_library_in` gives the editor the declarations and leaves the JavaScript out, so `gnoblinctl check --types` type-checks a shell that uses them, and `cargo xtask dist install` installs them. `StdLib::object_api` in `gnoblin-std-dyn` gives a runtime the std library's table. Example 20 now has a TypeScript and a Python shell (`examples/20-native-object-ts`, `-py`), run in a real session beside the Rust one by `modular::native_object`; `gnoblin-js/tests/object_classes.rs` runs the classes over the real core and std library in one process; the Python stubs are held to what the runtime reflects; `gen-sdks --check` parses the generated JavaScript with Node; and the sdkgen tests render the probe catalog (one class of every kind) and type-check the declarations against a script that uses them. The cost of a call from a script is measured and recorded in design/native-std-port.md, 4.2 (a property read 235 to 430 ns, a method 660 to 940 ns, a call that takes a record 3.8 to 4.1 us). No ABI or catalog change: `abi.lock` and `surface.lock` are untouched.

- Golden fixtures of the TypeScript std for the native std port (lane NpOracle; design/native-std-port.md, 6.4 and 7.1; `design/native-std-port-tests.md`). `cargo xtask oracle record` runs the TypeScript library over the fake compositor and writes the transactions, log order, tree snapshots and widget paint lists of 51 scenarios (174 fixture files) (toolkit, frame, menus, alerts, launcher, lock screen, scene and math, style, transitions, object model, input matcher) and of the tiling, examples and anim_style parity tests; `cargo xtask oracle check --repeat 3` proves them stable and equal to the committed files (`crates/gnoblin-testkit/src/oracle.rs`, `crates/gnoblin-js/tests/oracle/`, `xtask/src/oracle.rs`). Nothing is deleted. `tests/ts-compat/` holds the declaration emit of the library and `compat.ts`, which type-checks against it (`tests/ts-compat/check.sh`).

- `data/lua` and `cmake/install-lua-data.cmake` are deleted (lane NpLuaDelete; design/native-std-port.md, 2.4). Nothing read them: the Rust consumer `gnoblin-lua` is gone, and the legacy C Lua runtime implements its own `gnoblin.*` API and never loads `share/gnoblin/lua`. This retires decision D9 for `data/lua`, and `cmake/install-session.cmake` no longer installs `share/gnoblin/lua`. `data/default-config` and `data/init.lua.example` stay.

- Nothing the SDK generator writes is committed any more (lane NpGenOut; design/native-std-port.md, sections 3.1 to 3.6; docs/sdk-generator.md; `crates/gnoblin-sdkgen`). The generator moved from `xtask/src/sdk` into the library crate `gnoblin-sdkgen`, which names only the schema and the two ABI crates. `gnoblin-core`, `gnoblin-std-dyn`, `gnoblin-js` and `gnoblin-runtime-python` render their targets in `build.rs`: the Rust files are included from `OUT_DIR`, the TypeScript modules (`gnoblin-core/generated/*`, `gnoblin-std/generated/*`) are embedded in the JavaScript runtime and served under their usual paths beside the installed package (a generated module wins over an installed file of its path), and the Python modules (`gnoblin._generated.*`, `gnoblin.std`) are embedded in the Python runtime and served by a finder at the head of `sys.meta_path`. Deleted: `crates/gnoblin-core/src/generated/`, `crates/gnoblin-std-dyn/src/generated.rs`, `data/js/gnoblin-core/generated/`, `data/js/gnoblin-std/generated/`, `data/python/gnoblin/_generated/`, `data/python/gnoblin/std.py` and `std.pyi`, their entries in `.prettierignore`, `.ruff.toml`, `eslint.config.mjs` and `.pre-commit-config.yaml`; `.gitignore` now names those places so a stray write is not committed. `cargo xtask dist install` renders the TypeScript and Python files into `share/gnoblin/js` and `share/gnoblin/python` for editors (the tree it installs is the one it installed before), and `dist-check` fails a prefix without them. `cargo xtask gen-sdks` writes a readable copy to `target/sdk` (or `--out DIR`); `--check` now proves that the generator renders twice to the same bytes, that the Python compiles, that the public surface of the catalogue matches the new `crates/gnoblin-schema/schema/lock/<namespace>.lock` files (a released member that goes or changes is breaking; `--update` records an addition) and that the C ABI still matches `abi.lock` and the golden shells. `scripts/js-typecheck.sh` renders the generated modules before it runs `tsc`. `cargo xtask deps` has a rule for the generator (a build dependency of the four crates and of nothing else). The hand-written files that were named `generated` are renamed (`gnoblin-config/tests/catalogue.rs`, `gnoblin-js-types/tests/golden/tsconfig.library.json`) or folded into `lib.rs` (`gnoblin-proto`), so `git ls-files | grep generated` is empty. `GNOBLIN_JS_STOCK_DIR` stays for the hand-written library until `data/js` is ported.

#### Compositor and rendering

- An idle output now draws a shader pass as soon as it is ready, and a struck pass returns after its ten seconds without needing another repaint.
- Shader sources that run over budget are now switched off for 10 seconds after three consecutive frames above 50 ms, and refused for the session after three strikes, showing `shader: over-budget` or `shader: refused-for-session` in `skipped`.
- The Vulkan shader pass now compiles pipelines on a worker thread, so the first frame using a new shader source draws without it (reported as `shader: compiling`) instead of stalling, and the output is redrawn once it is ready.
- Fixed the Vulkan renderer using an out-of-range dynamic offset for passes without a shader block when a frame contains multiple plans.
- The Vulkan renderer now supports custom shader passes on layers, with `RendererKind::Vulkan` listing `shader`; sources that fail to compile or whose pipeline the driver refuses are drawn without the pass and reported in the frame's shader timings.
- Adds a `ShaderFilter` style element in Rust, TypeScript and Python that applies a custom shader pass to a window, drawn after the backdrop blur and before shadows, up to four per style, and only while `user_shaders` is enabled.
- Adds a `shader` effect kind that accepts validated WGSL source and up to 8 params, gated behind the off-by-default `compositor.user-shaders` setting; accepted shaders are reported as `skipped` since no renderer draws them yet.
- The GLES renderer now draws the `shader` effect, with passes chained in order, backdrop capture, and a compile failure skipping the pass instead of the frame.
- Gnoblin can now build as a core without std code, with the stock window manager moved to `gnoblin-std-wm`; the `std-providers` feature is on by default, so the standard desktop behaves as before.
- A shell holding the tree can now turn off the compositor's own window animations with the seat's `builtin_motion` (default `true`), and set each window's `frame_policy` (`auto`, `hold` or `throttle`) with `frame_rate` (30 Hz default) and `report_presented`.
- Gnoblin now enforces a GPU memory budget: clients past `compositor.client-memory-limit-mib` or `compositor.client-surface-limit`, or any change the budget cannot hold, are disconnected with `no_memory`, after releasing caches first.
- Each output's screen node now publishes read-only `gpu_memory_bytes` and per-category GPU memory figures, and fixes stop the GLES renderer from leaking textures and the Vulkan renderer from holding staging memory.
- The JavaScript shell now skips idle frames, so an idle frame drops from over 100 µs to under 1 µs, with `entry.wake()` available to request a frame on a timer.
- Windows can now have rounded corners, borders and a frame-following shadow, drawn by the GLES, software and Vulkan renderers, with `corner_radius` and `effects` options; renderers list unsupported parts in `skipped`.
- Views are now drawn on the GPU by the GLES renderer, so live Views cost the compositor thread microseconds instead of tens to hundreds of milliseconds; Views larger than 8192 pixels per side or 32 Mpx show no picture.
- The `Camera` gains stereo and off-axis projection through new `ipd`, `fov_left`, `fov_right`, `fov_up` and `fov_down` properties, with `stereo` cameras drawing two eyes side by side in a View.
- Windows showing a DMA-BUF now fade out on close or unmap instead of vanishing, using the buffer's own contents via `Closing.source`; a fade on real GPU hardware is still pending.
- Views on the CPU rendering path now scan only the pixels where their bounds project, which speeds redraws without changing the output picture.
- `Canvas.measure` now returns width, height and ascent, the stock `Text` and `Titlebar` ellipsize long titles, canvases paint sharp at HiDPI scale, and secure canvases are masked in captures.
- Fixed the GLES renderer not drawing the cursor, which happened because the default framebuffer was left bound after reading back pixels under it.
- Popups, tooltips, combo boxes and subsurfaces from clients are now drawn on the output, following their window's transform, opacity, fading, damage, capture and `wp_alpha_modifier_v1` factor, and are hidden when the window is secure.
- `View.fps` now caps how often a View is redrawn, and `View:capture(path)` writes a PNG of a View's picture to a path inside the runtime directory and returns that path.
- Canvas nodes now paint their content, so stock shell widgets such as `Titlebar`, `Text` and `Icon` render again, with `Canvas:measure` returning real text extents and malformed `Canvas.content` writes refused as `bad-paint`.
- Reduced motion now applies to the shell's animations via `appearance.reduced-motion`, so timed changes land instantly, and the compositor plays shell timed `set` animations for the first time.
- The stock Lua shell adds an XR mode: when the `xr` seat arrives, each window gets a View in a `room` on a 1.6 m ring, with the focused window at the front, and a `xr = false` window rule keeps a window out.
- Adds `gnoblin_render_software::render_view`, which draws a scene subtree through a camera with orthographic, perspective, or asymmetric stereo projection into an image sized to the View.
- Added a `View` node that renders a scene subtree through a `camera` (orthographic when unset) into a cached snapshot, drawn at its rect; the cache refreshes only when the source or camera changes.
- Each frame now applies queued tree transactions in order before drawing, whole or not at all, and a transaction that misses the 4 ms budget or arrives late waits for the next frame; transactions whose targets changed since their base serial are refused with `stale-base`.
- Each frame now applies queued tree transactions whole and in arrival order before drawing, deferring late or over-budget ones to the next frame, while refusing stale-base transactions as `stale-base`.
- Scene tree nodes now track an `owner` and `parented_by`, and the shell is limited to its own id range, its own nodes and a node and depth cap.
- Adds `SampledCurve` in `gnoblin-animation`, a validated list of up to 1024 evenly spaced values for custom animation easing, playable via `TweenPlayer::start_sampled` and `retarget_sampled`.
- Scene paint and hit order now follows sibling order, with the last sibling on top; `z_order` is removed and `raise`, `lower`, `move_to`, `insert_at`, `reparent_at` and `set_order` replace the old z-order setters.

#### Windows, workspaces and shell

- Workspaces, desks and window placement are now handled by the native library, so window membership events and layout arrive one to two frames after the engine sees a window, and floating windows keep their own bounds.
- Gnoblin now picks and starts its shell itself, with `--shell`, `--isolate` and `--list-shells` options, and `gnoblinctl shell reload`, `status` and `watch` let a running shell reload without dropping clients.
- Native tiling layouts (dwindle, strip, arc, carousel) now match the TypeScript layouts, including split resizing, edge snapping, client size limits and touchpad swipe and pinch bindings via `GestureBinding`.
- `DwindleLayout` now follows Hyprland's dwindle layout, with its `dwindle` section options (`forceSplit`, `preserveSplit`, `smartSplit`, `smartResizing`, `splitBias` and others) and `ratio` from 0.1 to 0.9.
- Nested session windows now get a frame on any host: the host draws it if it can, otherwise the session draws a minimal title bar and resize border, controlled by `compositor.nested-decorations` (`auto` or `off`).
- Adds `WindowFrame`, a per-window frame with edges, corners, underlay and overlay that scripts fill via `startDecorations(options)` and a `frame(client, frame)` callback selected by `rules`, with `resizeHandles` opt-in and a `placement` canvas property (`frame`, `over`, `under`).
- Tiling layouts can now be rearranged with the pointer: `Super+MouseLeft+Drag` swaps two tiles on release and `Super+MouseRight+Drag` resizes the nearest divider live, with `triggers` to disable either per layout.
- Window policy in the JavaScript shell now covers `Client.moveWithKeys()` and `resizeWithKeys()`, gestures and reduced motion, binding-only devices, workspace removal and sets, and window refit when bars change the work area.
- The JavaScript shell now answers client window requests (`maximize`, `fullscreen`, `minimize`, `close`, `move`, `resize` and more) as the Lua shell did, with `ClientListener.onRequest` able to override them.
- The JavaScript shell gains window style scopes (`gnoblin.clients.style`, `clients.where().style`, `client.style`) with `unset` to reveal inherited values, `client.computedStyle` and `client.styleWarnings`; only opacity and outer shadows or backdrop blur take effect.
- The JavaScript shell gains a window menu, a not-responding alert offering Wait and Force quit, an `Entry` launcher for `Super+space`, and a titlebar above the client that honours `double_click`, `middle_click` and `right_click`.
- Focus follows the pointer in `sloppy` and `follow-mouse` modes, and window rules now set workspace, floating, size, position, style, fullscreen, maximised, secure and skipFocus in the JavaScript config, with edge snapping on drop when `edgeTiling` is enabled.
- The JavaScript shell gains a UI toolkit with `Canvas`, `Box`, `Text`, `Icon` and `Grid`, and a default `Titlebar` on every window when `decorations` is enabled, plus writable `maximised` and `minimised` on `Client`.
- The stock shell now draws a window menu for Maximise, Minimise, Move, Resize, `Always on top`, `Move to workspace` and Close, plus a `not responding` alert offering Wait or Force quit.
- Tiled windows now get the four tiled edge states and their output's work area as bounds in their configure, while untiled windows get neither.
- While a shell holds the tree, window requests such as `maximize`, `fullscreen`, `minimize`, `close` and `activate` go to the shell, which answers by setting the window's maximised, fullscreen, minimised and size fields.
- The stock shell now handles fullscreen, minimise, activate, close, raise and lower requests for windows, and fullscreen rules cover the whole output including bars.
- The stock Lua shell gains `Window:move_to(ws)` and `Window:move_to_output(screen)` to move windows between workspaces or outputs, `Desk:remove_workspace(ws)` to remove a workspace, and unplugged outputs now hand their windows to another output or restore them when one returns.
- The stock shell keeps dragged windows inside the work area by default, or leaves a 75 px strip on screen with `drag_constraint = "minimum_strip"`; maximised windows float before moving and fullscreen windows refuse drags.
- The stock Lua shell reads focus and titlebar settings from the seat props `focus_mode`, `focus_new_windows`, `raise_on_click`, `auto_raise`, `auto_raise_delay` (milliseconds), `focus_change_on_pointer_rest`, `double_click`, `middle_click` and `right_click`.
- The stock Lua shell gains workspace sets via `define` and `activate`, which validate whole lists and refuse invalid input without losing the previous list.
- Raising or activating a window keeps fullscreen windows on top, and switching workspaces leaves a fullscreen window fullscreen on its workspace.
- Window snapping is available through `Window:snap(zone)` and `Window:unsnap()`, with `Window:maximise()` and `Window:unmaximise()` also added; snapping requires edge tiling to be on.
- Maximised and snapped windows now resize to fit the work area when a bar reserves space at the top of the screen.
- Adds stock tiled layouts `Tiling`, `Monocle`, `GridLayout`, `Dwindle` and `Kiosk` as Workspace classes, though the compositor does not load them yet.
- The stock shell's focus, edge tiling and window rules are added but not yet loaded by the compositor, so none of them run in a session.
- Adds `Strip` and `Carousel3d` workspace layouts, though they stay inactive until the stock shell library is loaded.
- The stock shell gains a lock screen, an overview and a panel module, with `Desk:toggle_overview()` dimming all but the active workspace, though the compositor does not load the stock library yet.
- The stock shell adds a titlebar with a close icon and drag-to-move on every window, a fade before windows are destroyed, a `Super+Space` launcher, and `Super+1` to `Super+9` workspace switching.
- The window manager accepts up to `MAX_WINDOWS` (256) windows, and a window beyond that cap is refused with `TooManyWindows` before any state changes.
- Named workspace sets can be declared with `workspace_sets` in `gnoblin.configure` and activated via `gnoblin.workspaces.activate_set(name)` or `gnoblinctl workspace activate-set NAME`, with unknown names refused as `unknown-set`.
- Windows now expose a `client_type` field (`"wayland"` or `"x11"`) in window records, `gnoblin.find` filters, the `match.client_type` window rule key, and `gnoblinctl window list` and `window match` output.
- Window records now expose `pid` and `process`, and `window:present()` focuses a window started by `gnoblin.process.spawn` without a key press once the user has been idle for 250 ms.
- Window methods `set_opacity`, `set_scale`, `set_rotation`, `set_offset` and `reset_visuals` change how a window is drawn without moving it; clicks follow the drawn position, config reload resets them, and they fail while locked.

#### Input, seats and accessibility

- Shells can now grab a seat's keyboard or pointer through `Seat.grab`, claim pointer buttons with `Seat.claim_pointer`, and replace the client's cursor with `cursor_shape`, `cursor_image`, `cursor_hotspot_x`, `cursor_hotspot_y` or `cursor_hidden`.
- The `input` configuration now applies to real input devices, including acceleration, scrolling, tap, click and handedness settings, and reloads on change; `double-click-time`, `drag-threshold`, custom acceleration curves, typing timeout, tablets, styluses and `orientation-lock` are not yet applied.
- Nested sessions now share the clipboard and primary selection with their host (text and images only, lazily, capped at 16 MiB per paste, and never secrets), and drawing tablets are supported on every backend via the new `InputEvent::Tablet` event and `zwp_tablet_manager_v2`.
- Nodes gain an `input_mode` of `forward` (default), `capture` or `none`: forward passes pointer events through a View to its source surface in surface coordinates, while capture delivers them to the View's pointer handlers instead.
- Input devices can now be configured per device class (`Keyboard`, `Mouse`, `Touchpad`, `Touchscreen`, `Tablet`) and grouped into named `InputContext` objects that scope their own bindings.
- Adds XR panel drag bindings: `SpatialLayout` gains a `rotate` trigger (`Trigger+Thumbstick+Drag`) and `ViewResizeBinding` takes `edges`, while `SpatialLayout.bindings` can replace a binding by id; `gnoblin.outputs.add` now throws `unsupported` because virtual outputs are not yet supported.
- Scripts can claim XR controller inputs (`trigger`, `grip`, `thumbstick`, `primary`, `secondary`) so the shell withholds them, and drag bindings release them when the drag ends or the session goes.
- Pointer drag bindings in the JavaScript shell now work on the real compositor, so `ClientMoveBinding` (Super+Button1) and `ClientResizeBinding` (Super+Button3) move and resize windows, respecting `drag_constraint`.
- Seat.claim accepts pointer chords ending in `Button1` to `Button9`, including bare `Button8` and `Button9`, and the owner receives the drag as `pointer_chord` start, update, end and cancel events while clients are kept from the pointer.
- Scripted key, pointer and gesture bindings via `gnoblin.input` now use one trigger grammar such as `Super+Shift+1`, with drags and gestures delivered per gesture; real-compositor pointer-button chords are not yet accepted.
- Binding-only devices: keys from a claimed device (`Seat:claim_device`, `Seat:bind{ only = true }`) no longer reach clients, seat key state or text input, and only fire the chords bound to that device or to all devices.
- Input devices tagged for a seat that this session does not have now attach to the default input seat (`seat0` unless the session names another), including hot-plugged devices, while keeping their original seat tag.
- A three-finger touchpad swipe now follows the fingers and switches workspace with the stock shell, and a short or unclaimed swipe slides back or does nothing.
- Fixed the XR gaze pointer so the `xr` seat is created on the first frame, letting gaze rays reach the front window and send it pointer enter events.
- Input events now name their originating device, `Seat.device` lists devices as `id:name:caps` on hotplug, and `Seat:claim`, `Seat:claim_gesture` and `Seat:bind` accept a `device` to scope bindings to one keyboard or pad.
- Without a shell, `xdg_toplevel.move` and `resize` start a pointer or touch grab that moves or resizes the window with the device, ending when the last button or touch point is released.
- Fixed multi-keyboard key handling: a key held on one keyboard no longer makes a second keyboard's press of the same key code get lost, and clients now see one press and one release per held key.
- Fixed touch input with multiple fingers so each touch point goes to the surface under it, instead of all points being sent to the surface of the first finger down.
- Seats can claim swipe or pinch gestures with `Seat:claim_gesture(kind, fingers)`, withholding them from `touchpad-gestures` bindings and clients and delivering them to the holder via the `gesture` signal, including pinch rotation.
- The stock shell's touchpad gestures now slide workspaces with a three-finger swipe that follows your fingers and snaps back unless you lift past a quarter of the travel distance.
- The main seat now publishes its window management policy as readonly properties, including `edge_tiling`, `focus_mode`, `raise_on_click`, `auto_raise` and the titlebar gestures `double_click`, `middle_click` and `right_click`, refreshed when the configuration reloads.
- Session lock now checks passwords through the `lock` node's `authenticate` method, rate-limiting repeated denials, and reports results as `ok`, `denied`, `rate-limited`, `not-locked`, `busy` or `failed` to the shell.
- The `gnoblin-xr` controller maps the right trigger to left click and the grip or B button to right click, and the thumbstick scrolls the window under the ray; a press holds its window until released, even if the drag leaves that window.
- The OpenXR session now reads VR controllers for aim rays and select, secondary and scroll input, falling back to a gaze ray from the head when no controller pose is tracked.
- `gnoblin-xr` now drives the owner's pointer from the OpenXR ray, sending motion over the window under the ray and leaving when it misses or enters a new window.
- Writing `focus` on the main seat node now moves the keyboard to that window, and writing `nil` clears it.
- A shell node with `input` set now takes keyboard focus when the shell sets the seat's `focus` to it, and unconsumed keys are sent to the shell as `key` events until a window regains focus.
- The main seat is now exposed as a compositor node named `seat0` under the root, publishing `pointer_x` and `pointer_y` as the pointer moves and `focus` naming the focused window when it changes.
- The owner can send `input` transaction ops that deliver pointer motion, button, scroll and leave events to a window on a chosen seat, independent of that seat's keyboard focus.
- Pointer events over shell nodes are sent to the shell as `tree.event` with kind `enter`, `motion`, `leave`, `press`, `release` or `scroll`, node-local coordinates, and button or scroll deltas where applicable.
- Touchpad swipe and pinch gestures now reach the compositor and run `touchpad_gestures` bindings that use `command`; built-in `action` bindings are validated but do not run yet, and gestures are not yet verified on hardware.
- Adds `gnoblin.input.grab(options?)`, a keyboard grab lease that expires after an idle `timeout` (1 to 60 seconds, default 10) unless renewed or a key arrives, delivering text, composed characters and a close reason to scripts.

#### Outputs and backends

- Output profiles and logical displays are now reconciled natively in the library, so profiles apply on connect and reconnect, and refusal and `supersample` notices appear with the first frames.
- The `gnoblin` binary now selects its backend via `compositor.backend` or `GNOBLIN_BACKEND` (`auto`, `headless`, `nested`, `drm`), so hardware rendering on the GPU is available instead of headless-only.
- The nested backend now draws with the session's renderer (`compositor.renderer = gles` or `vulkan`) and forwards output to the host's `wl_shm` buffers, with the host's seat, scale and shortcuts handled inside the session.
- The JavaScript shell now provides `gnoblin.outputs.profiles` for matching and reapplying output settings, plus `gnoblin.displays` with `single`, `mirror` and `span` modes, `Viewport` content and cameras; `vrr` is reported as not applied.
- Hardware (DRM) outputs can now change mode and scale via `Output.apply`, `defineConfig({ outputs })`, `wlr-randr` and `gnoblinctl`, though no modeset has yet been tested on a real GPU.
- Output modes `Output.apply`, `Output.mirror` and `defineConfig({ outputs })` now resolve whole layouts before applying, roll back on refusal, keep profiles per monitor, and support `mirror_of` and `enabled: false`.
- Adds a JavaScript `gnoblin.outputs` collection of `Output` objects with listeners for connect, disconnect, mode, scale and geometry changes, so each output can show its own workspace and `Bar` can target one.
- Adds `gnoblin-xr` bridge that submits each planned View as a headset quad, falling back to a headless session when Vulkan is unavailable; headset display is not yet verified.
- Each declared output now gets a `screen` node that publishes its geometry and is removed when the output is destroyed.
- Adds an experimental Vulkan renderer behind the `vulkan` feature, selectable with the `compositor.renderer` setting (`auto`, `vulkan`, `gles` or `software`); `auto` keeps the current choice.
- Outputs are now live objects with `alive`, `on`, `watch`, logical monitor index, size, work area and `privacy_screen` state, and shell processes get events when displays are added, changed or removed; the privacy screen setting is session-wide and is now changed with `gnoblin.settings.override("monitors.privacy_screen", value)`.

#### Session, services and recovery

- The JavaScript runtime now commits the configuration, reports each revision and answers `gnoblinctl config reload`, and the `runtime` role is removed; the stock library is read from the installed prefix.
- Native callbacks running past 2 s are logged and counted as strikes, and three strikes within 30 s latch the shell as degraded, so `gnoblinctl shell status` reports it and reload requests are refused while the last good generation keeps serving.
- The JavaScript and TypeScript runtime now ships as `libgnoblin-runtime-js.so` and runs inside the compositor, so `.js`, `.mjs`, `.ts` and `.mts` shells reload on file changes, keeping the running shell if an edit fails to load.
- A locked session whose shell uses only `gnoblin-core` or has no shell now shows the compositor's lock card for password entry instead of a black screen.
- The shared `gnoblin-shell-host` now handles native module loading and registrations for both the core and JavaScript shells, and retired native libraries are kept mapped and counted, with reloads refused past 32 retained libraries until a restart.
- The session lock screen is now drawn by the shell in TypeScript, with a built-in `lock` fallback card (clock and masked password entry) shown when no shell engages the lock screen within a second.
- The JavaScript shell now survives its process dying and runs a stock default config (one floating `desktop` and the stock keys) when `init.ts` is missing or fails at startup; `defineConfig` layers over it, and `gnoblinctl config reload` reloads the shell, rejecting a broken `init.ts` with `config-rejected`.
- The XR extension now survives runtime failures: a missing, refusing, lost or exiting runtime ends only the session, which is retried after 250 ms, doubling to 4 s, while `xr` seat and captures are cleaned up and the program exits only when its owner closes the socket.
- The session lock is now a single `Lock` object with a `lock()` method, which the screensaver service and `gnoblinctl lock` use, while the old `gnoblin.lock` object and its `locked`, `lock_session` and `not-locked` members are removed and now get `unknown-member`.
- When a shell detaches or is restarted, its chord, gesture and device claims are released, except those on a seat owned by another connection, so a replacement shell can claim afresh.
- Fixed `Buses::None` dropping caller-linked agents, which stopped the keyring from starting; it now skips only the built-in D-Bus agents.
- Fixed a security hole where connections that were not the session owner could claim portal UI slots and answer secret, polkit and other prompts; these IPC methods now require the owner.
- Fixed `slot.claim` with provider `none` clearing a slot held by another connection; giving a slot back now requires `slot.release`.
- Shell scripts now run in a restricted Lua sandbox without filesystem or process access, and each handler, binding and timer is stopped with a named report when it exceeds its instruction or time budget.
- When `GNOBLIN_XR=1` is set and `gnoblin-xr` is installed, the guardian starts it as an optional extension that restarts on backoff, is dropped with one warning when retries run out, and never degrades the session.
- The XR extension's GPU crate now picks a swapchain format and uploads `wl_shm` Argb8888 or Xrgb8888 pictures into it through Vulkan.
- Text sent by Lua or shell `call` is now kept secret: passwords are never printed, wiped from memory on drop, and dropped rather than resent when a stale-base retry occurs, while other text such as `Seat.claim` chords is still retried.
- Adds the `gnoblin-lock-auth` crate providing PAM-based password checks (service `gnoblin-lock`) with a rate limiter that waits 2 seconds after three failed attempts.
- The session lock is now a compositor-owned `lock` node in the tree, exposing its `state` (`not-locked`, `pending`, `locked`) so shells attaching during a lock read `locked` and unlocking returns it to `not-locked`.
- The stock lock screen now engages on a session lock request and only disengages when `Lock:unlock(password)` receives an `ok` answer from authentication.
- A queued tree transaction whose holder detaches before it applies is now refused as `not-privileged` and changes nothing, since the holder is checked when the transaction applies rather than when it was queued.
- Gnoblin's shell now talks to its owner over a session socket, with non-blocking polling and clear link errors for refused, closed, or malformed messages instead of panics.
- The shell VM (`gnoblin-lua`) now runs `node.lua` against a mirror of the compositor tree, applying each frame's changes and sending queued transactions as one batch, with refused transactions leaving the mirror unchanged.
- The shell's tree mirror rebuilds from chunked snapshots, applies serial-checked change batches, and requests a fresh snapshot when it detects a gap or stale batch.
- The shell VM is on by default when the stock Lua library is installed, `GNOBLIN_SHELL_VM=0` disables it, and any other value loads that directory instead; on failure the cause is logged and the role keeps serving.
- The shell now waits for the compositor's snapshot before booting, so it no longer starts with an empty window tree if it attaches early.
- The services role now starts the GeoClue agent alongside the polkit agent, following the `location` configuration table, and skips it without a system bus.
- Text printed by child processes without the `gnoblin-log` facade is logged as component `raw`; compositor, runtime and portal output is captured by default, services output needs `GNOBLIN_LOG_RAW_SERVICES=1`, and lines are scrubbed of secrets on a best-effort basis.
- The screensaver locks the session before suspend or hibernate while holding the sleep inhibitor, and on a `Lock` signal for its own logind session, and retakes the inhibitor on resume.
- The Lua configuration now receives `gnoblin.session.lock-requested` when it listens for it, so a config can start a locker without a separate shell process.

#### Configuration

- Setting `gnoblin.compositor.userShaders` now writes `compositor.user-shaders` through the shell configuration, is validated and committed, and takes effect at the next login.
- Reloading within the same core generation now clears styles left by an earlier script generation, so effects, opacity and corner radius no longer linger on windows; `style_reset` is added for `gnoblin-std`.
- The TypeScript std scene now builds and validates script-made nodes itself, including camera `size`, flat camera `zoom` and `roll`, and transition staging, and adds `scene_children` and `scene_parent` functions.
- Floating layout placement now uses the library, `StripLayout.pin` is settable again, `splitWidthMultiplier` clamps instead of refusing, and `cargo xtask dist install` ships editor type definitions.
- The style cascade now runs in the library via `style_*` functions, with `where` selection priority ranked by `style_rank`, while style objects still feed it once per frame and `computedStyle` and `styleWarnings` read results back.
- The `gnoblin-std` drag bindings now rely on the library's drag rules, with new `drag_step`, `drag_clamp`, `drag_resize`, `drag_zone`, `drag_tile` and `gesture_way` functions that work standalone.
- Shell scripts now run through language runtime libraries, selected by file extension or the launcher, with `.js`, `.mjs`, `.ts` and `.mts` mapping to `js` and `.py` to `python`; missing or failing runtimes leave the running script untouched.
- Gnoblin now picks its shell by one fixed set of rules: a `--shell` argument, `GNOBLIN_SHELL`, the `[shell]` settings in `config.toml`, then installed shells in the user, data and packaged default directories, with clear errors listing where it looked.
- `gnoblin-core` no longer imports `gnoblin-std`, so a shell importing only `gnoblin-core` runs; configs must now import `gnoblin-std` to get workspaces, layouts, styles, transitions, frames and `gnoblin.workspaces`.
- The JavaScript shell now caches compiled bytecode under `$XDG_CACHE_HOME/gnoblin/js/`, cutting stock library boot from 36 ms to 6 ms; set `GNOBLIN_JS_CACHE=0` to disable it.
- Scene nodes, views and clients in the JavaScript shell gain writable `position`, `rotation`, `scale` and `transform` properties, `Node.input` modes, and `node.properties.bind()` for reactive properties without polling.
- Adds a `console` global to the JavaScript shell with `log`, `info`, `warn`, `error`, `debug`, `assert` and `table`, logged with `.ts` call sites; `gnoblinctl console` shows what an expression printed.
- Adds `gnoblin.requestTransition(callback, { duration, easing })` to the JavaScript shell, batching property changes into one timed transaction that tweens from current positions; failed callbacks change nothing.
- Adds `autostart`, `protocols` and `touchpadGestures` config keys to the TypeScript shell, a `gnoblinctl config init` command that writes a starter `init.ts` and editor files, and clearer errors for mistyped or wrongly typed keys.
- Stock JavaScript layouts (`DwindleLayout`, `KioskLayout`, `StripLayout`, `Carousel3dLayout`), workspace sets and the workspace slide gesture are now available, with `Super+1` to `Super+9`, `Super+Left`/`Super+Right` and `Swipe3` bound by default and a new `launcher` key for `Super+space`.
- The JavaScript shell now commits `defineConfig` sections such as `dialogs` and `cursor` to the config document, reporting invalid values by key path, and observes consent and secret dialogs through `gnoblin.dialogs.addListener` without allowing or reading them.
- Adds a `dialogs.style` config section to restyle the consent and passphrase dialogs (radius, accent, font, button layout, scale), with values clamped so dialogs stay readable and safe, and a `dialogs.observe` slot that lets a shell deny requests before the built-in dialog appears.
- Setting `GNOBLIN_SHELL_VM=js` runs the JavaScript shell in place of the Lua shell, reloading `init.ts` or `init.js` from `$XDG_CONFIG_HOME/gnoblin` on save and keeping the old config on errors.
- Adds the `gnoblin-js` crate, the engine core of the JavaScript shell: TypeScript modules are stripped to JavaScript, loaded confined to the config root, and evaluated with a time and memory budget.
- Adds hot reload for `gnoblin-js` configs: saving a file reloads it after a short debounce, a failed reload keeps the running config and reports the error location, and three runaway scripts within 30 seconds pause reloads for five minutes.
- Adds a stock JavaScript configuration library with `defineConfig`, `defaultConfig`, and built-in tiling layouts (`MasterStackLayout`, `GridLayout`, `MonocleLayout`, `FloatingLayout`), though key bindings are declared only and not yet wired to input.
- Scripted workspaces are now `Workspace` objects managed through `gnoblin.workspaces` (`add`, `remove`, `clear`, `show`), with windows moved between them without ever being closed, and a config with no workspaces gets a floating `desktop`.
- Shortcuts, touchpad gestures and `window-management.check-alive-timeout` now apply when a config document is committed, so reloads take effect and first logins get shortcuts; a refused table keeps the previous one.
- Fixed a config reload refused by the compositor (such as a duplicate keyring autostart) ending the runtime; it now restores the last accepted handlers, reports `config_rejected` to `gnoblin.config.reload_failed`, and keeps running.
- Config reloads now report accepted settings that were not applied, split into `restart_required` and `no_effect`, in `config.reload`, `config.commit`, and `gnoblinctl config reload` output.
- Scripts can now call `gnoblin.runtime.reload_config()`, which returns an operation reporting success or `config_rejected` while keeping the last accepted configuration in force.
- The main seat exposes its declared workspaces through the read-only `workspaces` property, and the stock Desk builds one Floating workspace per entry, named by id with the declared name as title, or nine workspaces "1" to "9" if none are declared.
- The stock shell now loads in the session, followed by the user's `shell.lua` and `shell/*.lua` files, so window rules, workspace sets and key bindings in those files take effect.
- Invalid window rules in the stock Lua shell are now reported instead of failing silently: a bad rules list is refused whole so the previous rules stay, and a rule naming a missing workspace is reported and leaves the window in place.
- The ten node-core tree classes are now generated from the schema's `tree` namespace, so their properties, defaults, `readonly` flags and base classes cannot drift from it.
- The tree transaction now rejects property values of the wrong type (`wrong-type`), oversized text, byte strings or lists (`too-large`), and non-finite numbers (`not-finite`).
- The schema format now accepts `base` on objects, `default` and `readonly`/`animated` booleans on properties, and existing schemas still parse unchanged.
- Window rules accept a name declared in `workspaces` as their `workspace` target or `workspace-id` match, and an undeclared non-numeric name is refused at configuration commit.
- Adds `gnoblin.settings.override(path, value)` and `window:set_effect(name, value)`, which return a live `Override` layered over committed settings or a window's rules until revoked, with `on("closed")` reporting why it ended.
- Adds `gnoblin.timers.after(seconds)` and `gnoblin.timers.now()` to Lua, and Operations gain `map(fn)`, `timeout(seconds)` and `cancel()`; pending timers end on config reload.
- Added `gnoblin.process.spawn(argv, options?)`, which starts a program and returns a `Process` with `pid`, `status`, `wait()` and `kill(signal?)`, and keeps running across config reloads.
- Windows, workspaces and layer surfaces are now live Lua objects: each read returns the same updated object with `alive`, `on(signal, fn)` for its signals, and `watch(field, fn)` for a declared field.
- Adds `gnoblin.tween()` to animate window opacity, scale, rotation and offset through timelines with `play`, `pause`, `seek` and `kill` controls, plus `gnoblin.ease` and `gnoblin.lerp` easing helpers.
- A small notice at the top centre shows `Config reloaded` after a reload, the first line of a rejected reload's error until clicked or a later successful reload, and `<name> could not start: <reason>` when a shortcut or command fails to start.
- System drop-in Lua files in `gnoblin/conf.d` under each `XDG_DATA_DIRS` entry and in `/etc/gnoblin/conf.d` now load automatically, so packages no longer need a `gnoblin.load` line; failing drop-ins are skipped and listed in the recovery notice.

#### Portals, capture and privacy

- GPU screen capture buffers and the close fade of GPU windows are now covered by tests, where a supersampled output offers a GPU buffer of its composed size on `ext-image-copy-capture` and `zwlr_screencopy`, and the software and Vulkan renderers offer no GPU capture.
- Screen capture under the lock embargo is now covered by 36 documented failure cases; two defects were fixed, so `capture.screen` re-checks the lock before reading pixels and stale claims are dropped on reformat.
- Polkit authentication now uses a compositor-drawn password dialog checked by `polkit-agent-helper-1`, which scripts cannot bypass; `gnoblin.authentication.setProvider`, `request.allow` and `AuthenticationProvider` are removed, and `addListener` may only `decline` requests.
- Adds head-tracked stereo Views via `head.layer_budget`, where the head pose updates the `head` camera at most 60 times a second and observers may set only its pose and read-only `layer_budget`.
- Window screen casts now require a per-request user prompt showing the window's title and app id, streams only that window's rectangle (blacked out when secure, ended by lock or close, following its size and place), and reports `position` and `size` in layout pixels.
- Permission questions now name the requesting window by its `app id` and process ID, and show its title last in quotes, so a window cannot pass itself off as the prompt or another app.
- The stock shell can replace the built-in portal consent and keyring passphrase dialogs, which open with Deny focused, ignore Allow for 500 ms, and fall back to the built-in dialog if the handler fails; disable with `seat.data.prompts = false`.
- A shell can now replace the built-in portal consent dialog by claiming the `portal.ui` slot, with the built-in dialog remaining the default and fallback for any failure.
- Fixed portal requests during a session lock: they are now refused `locked` when no dialog is open, and the first request after unlocking is no longer refused for good.
- Screencopy and image-copy capture now offer GPU (DMA-BUF) buffers when the GLES renderer can render into them, and Views of GPU windows show their real pixels instead of a grey mask, with secure windows still masked.
- Views can now be screen cast through PipeWire as `VIRTUAL` sources selected with the `view` option, with secure nodes masked, consent and the privacy indicator handled as for outputs.
- Fixed portal method names so `ConnectToEIS` on `RemoteDesktop` and `InputCapture` and the `version` property on all `org.freedesktop.impl.portal.*` interfaces match the spec, instead of failing with `UnknownMethod`.
- Fixed a deadlock in the portal's IPC thread that could stop `RemoteDesktop.Start` from ever returning when a graph event arrived during a consent dialog.
- Portal consent now reaches the compositor: remote desktop and screen cast sessions are granted or refused by the user, and unanswered sessions are refused with `no-consent`, while `InputCapture.Start` no longer opens a screen stream.
- Added view capture for the XR extension, reading each View node's panel into a `wl_shm` buffer each frame; a failed connect disables capture without stopping the extension, and the picture does not yet reach the headset.
- Capturing a View via `ext-image-copy-capture` now copies its picture at the View's size, redelivers it when the View resizes, and keeps damage separate from other capture targets.
- While the session lock is active, View captures are now blocked just like output and window captures: new View streams are refused and any running View stream is stopped and does not resume.
- The `gnoblin.location.authorization-finished` event reports whether each location request was `allowed`, `denied` or `timeout`, and Lua location `Request` objects now report the real outcome from `finished()`.

#### Client protocols and compatibility

- Adds four Wayland protocols: `zwlr_virtual_pointer_manager_v1` (restricted, input seen as mouse input), `org_kde_kwin_server_decoration_manager` (behind `protocols.kde-server-decoration`), `org_kde_kwin_appmenu_manager` (exposes `appmenu_service` and `appmenu_path` on windows), and a partial `ext_workspace_manager_v1` that lists workspaces but needs `active_workspace` set by a shell.
- Wayland protocol implementations now live in their own `gnoblin-wayland-*` crates with no change in behaviour, and `gnoblin-wayland-api` provides the shared seam for them.
- The `requestTransition` function of the TypeScript std now wraps the library's transition bracket, with a plain write during a transition releasing it via `transition_release`.
- Adds `wake_fd` and verb/signal calls to the SDKs for Rust, Python and TypeScript, with `Super+Return` now claimed as written and reloads keeping state and client connections.
- Layer-shell surfaces are now drawn, stacked, hit-tested and placed by the protocol, so a mapped bar appears on screen at its reserved zone instead of being skipped.
- TypeScript shells can now use the std library's built-in layouts without it adding a default workspace or adopting windows, and `MasterStackLayout`, `GridLayout`, `MonocleLayout`, `KioskLayout`, `DwindleLayout`, `StripLayout`, `ArcLayout` and `Carousel3dLayout` are now thin wrappers over them.
- Built-in layouts and pointer or gesture bindings now live in `libgnoblin-std`, so every language SDK shares one implementation, with `DwindleOptions::pointer` now a `PointerSource` and `Std::ring` replacing the old ring helper.
- Adds a Python runtime as a separate library, so `.py` shells run inside the compositor using `from gnoblin import gnoblin`, `@gnoblin.bind` and `@gnoblin.on_client_created` decorators, and reloads that cleanly replace the previous shell.
- The native std now supports `Std::transition` with timed CSS easing writes, a scrubbable `Transition`, `Style` cascade with borders, shadows, opacity and corner radius, and cameras and views; the custom shader pass, XR and workspace views are not yet available.
- A shell that holds the scene tree can now place, stack and hide popups, subsurfaces and layer-shell surfaces via `placement_override`, `stack` and `visible`, and the compositor returns control when the shell detaches.
- Named subscriptions are now available on clients, outputs and collections, such as `client.onTitleChanged(handler)` and `gnoblin.onClientCreated(handler)`, each returning a function that detaches the handler.
- Mice now follow pointer locks and confinements and report relative motion, constraints activate when their window has pointer and keyboard focus, `wp_fractional_scale_manager_v1` raises `fractional_scale_exists`, `zxdg_toplevel_decoration_v1` raises `orphaned`, and text input and input method handling is improved.
- Adds XR scripting bindings, a `rotate` trigger on `SpatialLayout`, and `xr.Viewport`, while `gnoblin.outputs.add` now throws an `unsupported` error because virtual outputs are not yet supported.
- Fixes double title bars: `xdg-decoration` is now server-side only while the shell declares it draws decorations, otherwise clients are told client-side so windows are never left undecorated.
- Transitions in the JavaScript shell can now be seeked, paused, resumed, reversed and settled, and `width`, `height`, `rotation` and `scale` animate smoothly, with `delay`, `tween` and `autoplay: false` options and a `track_ended` signal.
- The JavaScript shell gains session, process, timer and event APIs such as `gnoblin.session`, `gnoblin.process.spawn`, `gnoblin.timers.after` and `gnoblin.on`, and `gnoblin.spawn` now returns the `Process`.
- The JavaScript shell gains `gnoblin.idle.watch()`, `gnoblin.idle.inhibit()`, `gnoblin.authentication.setProvider()` and `gnoblin.launches.begin()` for idle handling, authentication and launch feedback; privacy state, keyboard grab, text insertion and tablet-pad help are not yet available.
- The XR program now publishes tracked hand controllers as `controller` tree nodes with aim pose, `hand`, `tracked`, trigger, grip, thumbstick and button state, and a shell-written `claimed` list.
- The JavaScript shell gains XR scripting: `gnoblin.extensions.xr` appears once an `xr` seat has a session, and the stock shell arranges client panels in a room, though drag bindings cannot start on a headset yet.
- Adds a TypeScript scripting host (`gnoblin` module) where scripts see live `clients` and `workspaces` collections with listeners, writable window geometry, `close()` and `focus()`; input and layouts are not yet exposed.
- The `window` class gains a read-only `unresponsive` property, set from `window-management.check-alive-timeout`, and a `kill()` method that sends SIGKILL to a window's client; the `menu` request event now carries its anchor in `x` and `y`, and while a shell holds the tree, menu requests go to it instead of `menu_requested`.
- Requests to `xdg_toplevel.show_window_menu` are now ignored unless their serial matches one the compositor issued for an input event, so clients can no longer open keyboard-grabbing menus at will.
- The XR extension now builds a stereo View's OpenXR layer from its `stereo` camera's eyes, so the picture and layer match; the picture stays fixed in the room and does not follow the head.
- Fixed IPC clients losing the start of a frame when a read timed out mid-frame, which could kill the connection or drop an answer, event, or `Start` call reply.
- Input event details such as `x`, `y`, `button`, `key` and `text` now reach shell handlers, with `press`, `release`, `motion`, `key` and related signals declared and events bubbling to ancestors.
- GPU clients can now share dmabuf buffers via `zwp_linux_dmabuf_v1` instead of falling back to `wl_shm` copies per frame, with buffers the GPU refuses failing the request.
- Fixed protocol errors: `xdg_wm_base.get_xdg_surface` now raises `role` or `invalid_surface_state` correctly, a second `wp_viewporter.get_viewport` raises `viewport_exists`, `wp_viewport` checks its source at every commit, and a shrunk `wl_shm` pool raises `invalid_fd`.
- Fixed floating toplevels getting an 800 by 600 initial size, `xdg_surface.set_window_geometry` placement and hit-testing, subsurface pointer and touch input past the parent, pointer enter, leave and motion events on surface changes, touch-up on destroyed surfaces, and redundant configures.
- Fixed focus, pointer frame and subsurface handling so `xdg_toplevel` activation states, `wl_pointer.frame` grouping, subsurface frame callbacks and input regions behave as clients expect.
- Popups now receive pointer input wherever they hang, sit above their windows, answer frame callbacks, honour `xdg_popup.grab` dismissal, and are repositioned when their window moves.
- X11 clients' window-state requests (maximise, fullscreen, iconify, close, activate, move/resize) now get a real answer from the shell or compositor, and `_NET_WM_STATE`, `_NET_CLOSE_WINDOW`, `_NET_MOVERESIZE_WINDOW` and `_NET_FRAME_EXTENTS` are supported.
- `tree.attach` now takes a `mode` of `holder` (default) or `observer`, letting many read-only observers watch the tree beside the shell; observers may only create and destroy their own seats and send pointer input, and other operations are refused with `not-holder`.
- Extensions given the `view` role now see only `wl_shm`, `gnoblin_view_source_manager_v1` and `ext_image_copy_capture_manager_v1` and no other Wayland global.
- The XR session runtime gains swapchain, view location and frame submission, with `connect_vulkan` opening Vulkan-backed sessions and the headless session unchanged.
- Layers for OpenXR frame submission now drop to `opacity-ignored` or `layer-budget` as needed, with world layers kept ahead of head-locked ones.
- The owner-only `gnoblin_view_source_manager_v1` global is offered only to the owner's privileged connection, creating capture sources for View nodes that `ext-image-copy-capture` can read, while a non-View node is refused with `invalid_node`.
- Window nodes now publish `x`, `y`, `width` and `height` as `set` changes whenever they change, with size being the content size and position the placed frame origin.
- A window node is announced once as `ready` via a `tree.event` of kind `ready` carrying the node id, in the batch after it is created with content.
- Window nodes now publish `focused`, `fullscreen`, `maximised` and `minimised` together, and only when one of them changes.
- Each newly created window node is announced as `ready`, including when a window is recreated and gets a new node.
- A `tree.snapshot` node now includes `props` with its non-default property values as `set` change records, so shells attaching to a populated tree see those values immediately.
- Adds an XR session frame loop that drives OpenXR session states and creates a headless OpenXR session, but nothing yet renders, submits layers or connects to the owner socket.
- The `seat` node can now be created with a readonly `name` in the same transaction as `new`, exposing an extra `wl_seat` global with that name that is removed when the node is destroyed.
- While a shell holds the tree and the session is unlocked, client window requests such as `move`, `resize`, `maximize` and `fullscreen` are delivered to the shell as `tree.event` requests instead of being acted on by the window manager.
- The XR extension's planner turns visible `room` Views into OpenXR layers (world-space quads, head-locked quads, or stereo projections), skips invalid transforms with a reason, and picks View pixel coordinates from rays against layers with `input` enabled.
- The `gnoblin-xr` extension can now connect to the owner socket, attach to the `tree` namespace, and follow scene changes, but it submits no layers yet and `--help` and `--version` exit without connecting.
- A mapped toplevel now appears as a `window` node in the tree, carrying its title and app id read-only, with created, set and destroyed changes published as it maps, retitles and unmaps.
- Node properties `tags` and `effects` now carry values through the tree wire and render transaction, with `effects` limited to 16 blur or shadow passes and shader passes refused with `bad-value`.
- The owner socket now serves the window tree: one shell attaches at a time via `tree.attach`, transactions via `tree.transact` are applied all-or-nothing, and changes are published as `tree.changes` events, though calls and animations are not yet dispatched.
- Node transactions are now validated as a whole, so a batch of up to 4096 operations is applied only if every operation passes, and a refused batch changes nothing; the `failed_op` index and a kebab-case refusal code report the first failure.
- The schema now defines the ten node-core tree classes and the `tree` owner-socket messages in namespace `tree`, though the compositor does not serve them yet.
- Adds `gnoblin.idle.watch(seconds)`, which returns a live `IdleWatch` for idle times from above 0 up to 86400 seconds; each runtime allows 256 watches, and a reload ends them all.
- The stock window manager and portal dialogs now live in the std library and reach the core through a C ABI, with `NullWm` and `no-ui` fallbacks when no std is present.

#### Command line and tooling

- Adds `cargo xtask gen-sdks [--check]`, one generator that regenerates the Rust, Python and TypeScript SDK bindings from the schema catalog, and `just workspace-ci` now fails when any generated SDK file is stale.
- Running `gnoblin ./libshell.so` now loads a native shell inside the compositor's process, with `gnoblin ./libshell.so` as the launcher and `--role compositor` used by the core library; a shell that hangs, panics, exits or fails `init` no longer takes the compositor down, and `<Super>Return` chords, `class_nodes` and screen `width`/`height` work against the real compositor.
- Adds `cargo xtask native-probe` to check native dependency floors against target images; 13 of 22 images are probed so far, alongside a recorded stock Mutter 51 reference session.
- `gnoblinctl config migrate` now writes TypeScript for the shell's own objects and marks settings with no home as `UNMAPPED` comments, exiting with 1; `gnoblinctl check` evaluates the file against the dry-run shell so unknown names and refused bindings fail.
- `gnoblinctl config migrate` converts an `init.lua` to an `init.ts` without running Lua, reporting any constructs it cannot map as commented-out lines and exiting 1 when something was left out.
- Adds a live JavaScript shell console: `gnoblinctl console` opens a prompt into the running shell, and `gnoblinctl console --eval EXPR` prints one value as JSON.
- `gnoblinctl check FILE` now reports syntax errors, unresolved imports, failing import cycles and top-level throws in JS/TS modules at `file:line:column`, with `--types` running an installed `tsc` against the generated `gnoblin.d.ts`, and `gnoblinctl types init` writes that declaration file and a `tsconfig.json`.
- Adds `scripts/qemu-e2e preflight` to report whether a machine can run the QEMU test matrix, and `sync-prefix` now checks and records the Rust or Mutter prefix kind before copying.

#### Packaging and install

- Shells built against ABI 1.0 now run unchanged with newer minor library versions, while libraries of a different major version are refused with a clear message naming both versions.
- Adds the `libgnoblin-std.so.1` desktop library and the `gnoblin-std` Rust SDK, providing live client, output and workspace collections, layouts, output and workspace policies, and move and resize pointer bindings; it is loaded only when a shell calls `g.std()`.
- Adds the `libgnoblin-std.so.1` desktop library and the `gnoblin-std` Rust SDK, offering live client, output and workspace collections, layouts, move and resize bindings, and `g.std()` loading; the layouts for dwindle, strip and arc, snapping and split-resize are not yet available.
- The `gnoblin` binary now loads `libgnoblin-core.so.1` from beside it, and `cargo xtask stage` and `cargo xtask dist install` build and install the library, shells and JavaScript data; `cargo xtask dist-check` verifies the layout.
- Adds the `gnoblin-xr` extension scaffold, which finds the active OpenXR runtime, detects `monado-service` and `wivrn-server` on PATH, and supervises the runtime with up to 3 restarts; no rendering yet.
- Gnoblin now ships `.deb` packages for Debian testing, `.apk` packages for Alpine edge (release assets only), and arm64 builds of every package.
- Signed package repositories are now published for Fedora, openSUSE Tumbleweed, Debian testing and Arch, each with a `stable` channel (from release tags) and a `nightly` channel (from daily main builds).

#### Documentation and tests

- Examples 2, 6, 8 and 9 now run in real sessions against the shipped `gnoblin` binary with a real Wayland client, and the modular framework's final status is documented; the Python no-workspaces tiler no longer writes negative sizes.
- Added user and developer documentation for the modular framework, covering Rust, Python and TypeScript shells, `gnoblin --watch` reloads, installed file layout, and the seven shell discovery tiers.
- Python and TypeScript now ship generated std bindings for `libgnoblin-std`, with classes, records, enumerations and typed layout and pointer bindings; importing without `libgnoblin-std.so.1` fails with a clear error.
- Examples 4, 5, 7, 11 and 17 now cover JavaScript and TypeScript user shells, including live reload of edited shells and modules and running a shell that imports `gnoblin-core`.
- The idle-frame benchmark now fails when an idle frame with 300 windows costs 300 µs or more.
- Running `gnoblinctl types init` now writes the real TypeScript types for the JavaScript shell to `.gnoblin/lib/gnoblin`, with imports like `gnoblin/scene` resolving to source with hover docs, and replaces older `gnoblin.d.ts` and `tsconfig.json` files.
- Settings documentation now covers the shell's objects, with `Keyboard`, `Mouse`, `Touchpad`, `OutputProfile`, `input.bind` and `gnoblin.dialogs` shown, while the TypeScript config and window rule pages are removed.
- Screenshot script gains `reload-tiled` and `transition-mid` pictures, the 52 px height loss of client-side decorated windows is diagnosed, and the docs images are re-taken.
- Adds a screenshot script that captures the JavaScript shell running in a headless session across layouts, decorations, shadows, client title bars, the window menu and a locked session, with four pictures added to `docs/images/js-shell/`.
- CI now installs one shared system package list via `scripts/ci-install-system-libraries.sh` for the Rust workspace, pipewire, msrv, wlcs and Fedora parity jobs, so bare Ubuntu runners no longer fail on missing libraries.
- Continuous integration now runs the `gnoblinctl-cli` test suite alongside the other ctest checks, so changes to the `gnoblinctl` command-line tool are tested automatically.
- Continuous integration now runs a PipeWire job that installs the PipeWire daemon and headers and runs the export and view capture end-to-end tests against a private daemon.
- Parity tests now compare the popup and focus scenarios against stock Mutter 51 when a launcher is set, and record `focus_new_windows` (default `prevent`) as the one reviewed divergence from Mutter.
- Adds XR failure-mode tests covering runtime failures mid-session, lost swapchains, missing head pose or controllers, View changes while streaming, overlapping and zero-size Views, undrawable stereo cameras, owner loss, and the retry loop with backoff and reset after a healthy session.
- Recorded XR tree sessions can now be replayed one change batch at a time, checking that layers and panel placement stay correct as the head moves and turns.
- The View redraw cost profile now includes a stereo column.
- Effect rendering (shadow, blur, tint, opacity and their combination) is now covered by pixel-comparison golden tests, with tint and opacity also checked against the software renderer.
- Views and effects now have failure-mode coverage: an enormous `fps` yields a one-millisecond slot, negative or fractional `fps` is refused with `wrong-type`, a stalled PipeWire consumer no longer blocks capture, and a View whose source is destroyed sends no pixels until a new source is attached.
- Added a new `docs/shell-api/effects.md` page and extended `docs/shell-api/views.md` with `fps` bounds and redraw cost; `Node:add_effect` now errors on the `shader` effect and on `source`, `entry` or `uniforms` fields, naming `blur`, `shadow` or a `Canvas` as replacements.
- The stock shell's consent and passphrase prompts are now tested end to end over real D-Bus, covering portal access dialogs, pinentry passphrase entry and keyring unlock.
- Tests now cover portal `deny` and `allow` responses inside the arming window, refusal of non-owner IPC connections via `ipc_owner_uid`, and real-VM checks of the stock dialogs.
- The Wayland Conformance Suite (wlcs) now runs against the compositor, with 467 of 702 core tests passing and each remaining failure listed with a reason and tracking issue.
- Portal tests now run end to end against a live compositor over a private D-Bus bus, covering RemoteDesktop, ScreenCast and InputCapture consent, input, sessions and cleanup.
- Adds a whole-stack XR smoke test (`scripts/xr-smoke.sh`) running the real compositor, stock shell, two clients, `gnoblin-xr` and Monado together, plus test tools `xr-smoke-client` and `xr-smoke-tree`; `gnoblin-xr` gains test-only `GNOBLIN_XR_DUMP_DIR` and frame/layer count logging under `GNOBLIN_LOG=xr=debug`.
- Seat `focus` writes from the shell are now covered by tests: windows are accepted, `nil` clears focus, and invalid targets are refused with `bad_value` or `wrong_type`.
- Adds a replay test harness that records timed input sessions, applies them deterministically, and checks that per-frame tree hashes match across runs.
- Keyring, GPG, polkit, location and session-lock requests are now exposed to Lua as a single `event.request` object with `id`, `kind`, `status`, detail fields and a `finished()` operation.

### Fixed

- `modular::native_input` and `modular::native_pointer` (examples 28 and 30, Rust and Python) no longer fail now and then (lane NpInputFlake; Beads `gnoblin-c9xt`, `gnoblin-03sf`; `crates/gnoblin-wayland/src/keyboard.rs`, `crates/gnoblin/tests/support/native_shell.rs`, `crates/gnoblin/tests/modular/{native_input,native_pointer}.rs`). A reload hands a shell's chords to the next generation in one transaction, the old generation's releases and then the new one's claims, and the compositor applies a transaction in a frame of its own. The tests pressed on the shell's own log: a script's `generation 2 is running` is written while the generation is only staged, and the drag's `cancel` while the old one ends, both before the transaction is sent. A press that came before the frame was routed on a claim of the generation that had already ended, so no one ran it (example 28 never saw `generation 2 handles Super+b`), or a drag began on a claim the frame then dropped (example 30 never saw `generation 2 drag start`). The compositor now logs `the shell's seat claims were applied: N made, M released` once the calls of a transaction are in force, and the tests wait for the claims of the generation before they press (`Gnoblin::wait_claims`). The handover itself is unchanged; the log line is the only change to the product. On a 2-CPU box the full modular suite failed in 3 of 40 runs before, and the native_input tests failed in 5 of 10 runs with two busy loops beside them.
- The window menu's `closed` signal is told to a listener whose shell no longer holds the card, and `modular::native_menus` no longer fails now and then (lane NpMenuFlake; Beads `gnoblin-0vok`; `crates/gnoblin-std-impl/src/menus/windows.rs`, `crates/gnoblin/tests/modular/native_menus.rs`). A signal is told from the library's next turn, and killing the card ended its object at once when no shell held a handle to it, which dropped the listeners before the turn: a script that added `menu.on("closed", ...)` and let go of `menu` when its handler returned (the handle is released as the handler returns) was never told of a close that came before something else held the card. `close` now holds the card until the turn that tells `closed` has run. `the_menu_says_when_it_closes_to_a_shell_that_has_let_go_of_it` (`crates/gnoblin-std-impl/tests/menus_api.rs`) failed before. The modular test also pressed outside the card as soon as the shell logged that it opened, before the compositor had the layer's size from the toolkit's next turn, so the press hit what lay under it; it now waits for the layer in the tree.
- `modular::native_scene` and `modular::native_style` no longer fail now and then for TypeScript (lane NpTsFlake; design/native-std-port.md, NpTsFlake; `crates/gnoblin/tests/support/native_shell.rs`, `crates/gnoblin/tests/modular/{native_scene,native_style}.rs`). The two tests read the tree once, right after the lines the shell logged about the nodes it made, but the shell sends what its start wrote from its first dispatch after it serves and the compositor applies it on its own thread, so the snapshot missed the nodes in about one run in four on a 2-CPU box (the TypeScript shell reaches its first dispatch about 2 ms after Python's). The tests now wait until the tree holds the nodes (`wait_for_nodes`). The product is unchanged.

#### Compositor and rendering

- Fixed a nested desktop leaving trails of old cursors behind a moving pointer, and a stopped pointer freezing on an old picture; the hardware backend also no longer shows a black screen when a flip is in flight or refused.
- Windows now fade out when their client exits or destroys its buffer before the toplevel, instead of vanishing at once; GPU-buffer windows still leave immediately.
- Views created by the shell are now drawn, shown and offered to `gnoblin_view_source_manager_v1` for capture, where previously none of them rendered.
- A View created with no width no longer crashes the compositor thread when it is drawn, since a blank `Image` with zero width now stays empty.
- Window scene nodes now get unique ids from the compositor range, so they no longer collide with shell nodes (`duplicate-node`); windows whose node cannot be created are logged and skipped.

#### Windows, workspaces and shell

- A titlebar that appears after its window is placed no longer sits above the screen, the frame keeps its place when the bar comes or goes, and windows parked by an unplugged output return to their workspace as the same windows.
- Fixed windows drawn outside their window geometry, such as foot's title bar or GTK shadows, being held too small and drawn in the wrong place: a window's `x`, `y`, `width` and `height` now match its window geometry, so sizes written back are the sizes clients receive and maximised or fullscreen windows fill the output.
- Fixed tiled layouts leaving a black column at the screen's far edge, so master-stack, grid, dwindle and strip tiles now reach the edges of the area.
- Fixed floating windows that were too big for a free spot being placed on top of each other, so a later window could completely hide an earlier one.
- Maximised toolkit windows are no longer configured back to their old size after drawing the maximised size, and a maximised window stays at the work area whatever size its client commits.
- Fixed the shell VM reading back its own writes incorrectly during an in-flight transaction, which could make `Titlebar.draw` fail with `no node N`, stop `on_ready` early, or place windows at (0, 0) beside the previous one.
- A window rule that names a workspace now places the window on that workspace instead of leaving it on the active one, as `Window:rule_workspace` is read before the window is inserted once.
- Fixed `Desk:show` and `Desk:step` so they order workspaces by their position in the Desk (`Desk:position`) instead of the native workspace index, which could be unset on shell workspaces.
- Unparented windows now receive `ready` once, on the first frame that sees them, so the stock floating shell's `Window:on_ready` runs.
- A closed window now runs its `Window:on_destroyed` callback and is removed from the tree exactly once.
- Nodes restored at attach now get `ready` once, parents first, so `Screen:on_ready` runs, and the stock shell places each mapped window on its own spot.
- Windows opened without a shell attached no longer stack at the output origin; they cascade from (32, 32) by 32 px per existing window on the output and workspace, clamped to the work area.
- Pointer focus now follows `focus_mode` (`click`, `hover`, `hover-strict`) and `focus_change_on_pointer_rest`, with clicks focusing and holding focus until release.
- Activation requests from `xdg_activation_v1` are now sent to the shell as an `activate` request, which focuses and raises the window only if it is on the active workspace.
- Window rules now apply once a new window is on a desk, so a rule sending it to another workspace takes effect, and a rule that stops matching clears the field it set, such as `secure`.
- Window records beyond the 32nd window no longer trigger a debug assertion.
- `gnoblin.workspaces.create`, `next`, `previous`, `Workspace:activate` and `rename` now return the live `Workspace` that `gnoblin.workspaces.by_id` returns, with `window_count`, instead of a plain record with a `windows` key.

#### Input, seats and accessibility

- Fixed a copy made twice in a row in a nested session still being pasted on the host, which replaced the session's copy with an empty host selection.
- A press on a shell node now keeps the pointer's motion and release going to that node until the button is released, even when the pointer leaves it, so drags started on resize handles or titlebars no longer break.
- Controller trigger, grip, face button and stick presses are now held for 150 ms so a shell drag claim can discard them instead of sending a stray `BTN_RIGHT` click, so these clicks land 150 ms later; `XRSession.chordSettleMs` is capped at 60 ms and rejects non-numeric values.
- Touchpad swipe gestures no longer invert vertical movement, so a swipe up now matches the `touchpad-gestures` path coordinates, which may flip configs written against the old behaviour.
- A claimed chord now sends `chord` press and release events to the shell with the claim id, hiding both edges from the client until the claim ends, and a destroyed seat releases its claims so chords reach the client again.
- Pointer presses and motion over a window the shell rotated or moved now reach it where it is drawn, not its unrotated placement rectangle.
- Fixed `gnoblin.input.grab` being refused indefinitely for a key that was already released while the diagnostic console was open; held keys are now tracked per keyboard and cleared when a keyboard is removed.

#### Outputs and backends

- Mode changes on hardware outputs are refused unless the `GNOBLIN_DRM_MODESET=1` environment variable is set for the session, since DRM modesetting is untested on real hardware and can blank the display; scale-only changes still work.
- Passing `primary` to `output.apply` is now refused with an error instead of being silently ignored, since the layout derives the primary output.

#### Session, services and recovery

- A shell with a syntax error no longer tears down the session: it stays failed, `gnoblinctl shell status` reports the error, and saving the file or running `gnoblinctl shell reload` loads the fix without restarting.
- Script calls are now limited by their CPU time rather than wall time, so a busy machine no longer causes spurious budget strikes, while calls stalled past eight times their limit are still stopped.
- A password check now runs in its own transaction ahead of the frame's other writes, so a `stale-base` refusal no longer discards the secret and forces a retry.
- The session lock stays locked and never unlocks on a wrong or empty password when the shell hangs, crashes, freezes or fails, and the password is handed to the compositor within three seconds of the shell's last heartbeat (immediately if the shell is gone).
- A window that appears under the lock no longer leaves the session with no way to type a password, because the compositor's lock card now takes the keyboard one second after the shell's lock screen stops holding input.
- Scripts can no longer replace `globalThis.__gnoblinHost`, JS role diagnostics are capped, and a client that stops reading is dropped instead of stalling the session or lock card.
- The session lock card's takeover timing is now measured and documented: in release builds it takes the password about 3.4 s after the shell's last healthy beat and about 1.3 s after the keyboard leaves the lock screen, matching the documented "3 seconds" and "1 second" plus half a second.
- Fixed a drag past `REQUEST_DRAG_LIMIT_MS` that overflowed the stack every frame, the not-responding alert no longer wakes every 250 ms, the stock shell now rests when idle, and a `Clock` widget updates at each minute or second boundary.
- The `GNOBLIN_POLKIT_HELPER` environment variable is removed, so the polkit authentication helper is now only found in the distribution paths and can no longer be overridden from the session environment.
- Fixed the JavaScript shell losing writes after refusals: refused transactions are now resent in full (except `secret` calls, which fail), placements dropped after retries are laid out again, and a lost password check now fails promptly so the lock screen's card can take over.
- Fixes the JavaScript shell sending transactions too early and dropping writes after retries, sends idle holds before watchers see idleness, and now reports writes that are dropped after resends.
- Fixed a JavaScript shell reload leaving most keys dead: the new generation keeps its seat claims until takeover, and the swap releases the old claims first, so `Super+1` to `Super+9`, media keys and the three-finger swipe keep working.
- Fixed the guardian logging every INFO line from a child's stderr as `WARN`, so healthy starts no longer look like a page of warnings.
- Idle inhibit holds are now released automatically when the connection that made them ends, so a crashed shell no longer leaves the display's idle inhibition on; `uninhibit` releases only the caller's own holds.
- While the session is locked, the compositor no longer reads back views or keeps view textures alive, and queued captures no longer draw or read a view before being refused.
- Fixed a click's focus change being lost when the seat changed mid-transaction; refused writes are now resent, up to `STALE_RETRIES` times in a row.
- Session lock password checks now answer `denied` for wrong passwords, `rate-limited` after three failures, and `failed` when the check cannot run, keeping the session locked and logging a single warning without the password.
- Shell startup now runs under its own wider load budget, so a busy machine no longer makes the shell fail to start, while runaway user shell files are still stopped and logged.
- Closing a client while the XR room is up now removes its View and tears down the room when the seat goes, and `Node:destroy` no longer fails on destroyed nodes or leaves stale references behind.
- When a shell detaches, the nodes it created are now destroyed with its connection, so a restarted shell no longer builds a duplicate tree; windows it parented move back to the root, while observer seats remain.
- After a shell restart, windows are placed again and stay mapped, keeping their client size.
- Fixes a bug where an attaching shell received the compositor's seat and screen node changes twice, once in the attach snapshot and again as `changes` events.

#### Configuration

- A real reload now clears the shadows and borders a window's earlier script styled, instead of leaving them on the window.
- When the config directory has an `init.lua` that nothing reads, Gnoblin warns once, naming the file, what runs instead and `gnoblinctl config migrate`.
- Errors thrown inside the stock library now report the correct `.ts` file and line, and its types are stripped once per process instead of once per generation.
- Reloading the JavaScript shell's `init.ts` no longer starts its top-level `gnoblin.spawn` programs twice or skips them when the file fails to load; programs are started only when a new config takes over, unchanged programs keep running, and programs the new file no longer starts receive SIGTERM.
- The JavaScript shell's time limits are relaxed: module loads may take up to 30 s (was 5 s), listener calls up to 1 s (was 100 ms) and frames up to 5 s (was 500 ms), and loading the default config now uses the module-load limit.
- Fixed a bug where re-enabling decorations during a config reload could place the titlebar above the screen without moving window content down.
- A stale-base retry now resends `Canvas.measure` and `Seat.claim` calls instead of dropping them, so text widths and chord claims no longer go missing; only password-carrying calls are still dropped.
- Fixed the Lua `core.insert` binding so its arguments follow the `(parent, node, index)` order used by `node.lua`, which previously swapped the parent and child on every insert.
- Replies to `config.commit` that remove window rules now arrive only after the compositor has applied the removal, so a window mapped right after the reply no longer gets placed under the removed rule.
- Configuration reloads no longer wait for running tweens, which previously could delay a reload by 15 seconds and then make it fail; the tween is now cancelled when the new configuration takes effect, with `"cancelled"` outcome and `"config_reloaded"` reason, and a failed reload leaves it playing.
- A `gnoblin.configure` call rejected by the validator inside an `Operation` callback now fails only that `Operation`, without logging `deferred Lua operation callback failed`.
- Reloading the configuration with `gnoblinctl config reload` no longer rejects `input.keyboard.xkb_options` values such as `caps:escape` that were accepted on the first load.

#### Portals, capture and privacy

- Fixed a flaky portal test where dialog keys could arrive before the compositor opened the dialog, leaving it open indefinitely; keys are now sent only after the compositor acknowledges the offer.
- Fixed a security issue where any D-Bus client could make the GeoClue agent show a location prompt; now only the connection owning `org.freedesktop.GeoClue2` is answered, and other senders get `org.freedesktop.DBus.Error.AccessDenied`.
- Fixed a security issue where View captures showed pixels of secure windows; captures now mask them grey, controlled by `capture.secure-treatment`.
- Fixed a security issue where content below a secure node could escape redaction in View captures and screen capture; such nodes are now masked consistently across all capture paths.
- Fixed a security issue where a screen capture requested right after a `secure` mark was acknowledged could still include pixels from the secure window for one more frame.
- Secure window content in Views is now masked in every capture, on-screen display and renderer path, including while the session is locked.
- Fixed a security issue where a PipeWire screen-capture consumer could receive a buffer captured before a `secure` mark, because the last filled buffer was held in the daemon across the mark.
- View captures now copy the requested region and honour the buffer's `wl_shm` format, while buffers of an unoffered format or too small are refused with a `failed` event.
- Windows the shell marks `secure` are now masked in screen capture, including marks written from a stock-shell rule or a user shell file, and clearing the mark unmasks them.
- Security: the `view` role now sees only `wl_shm`, `ext_image_copy_capture_manager_v1` and `gnoblin_view_source_manager_v1`, so it can no longer map windows, lock the session or type input.
- The portal backend now destroys the ScreenCast consent dialog and its hidden parent window when a screen cast session ends, instead of keeping them alive.
- A shell process could answer prompts or authorization requests it never received through `prompt.respond`, `prompt.cancel`, `auth.*` and `location.authorize_app`, because the recipient check was skipped; the compositor now enforces it.

#### Client protocols and compatibility

- Shells built before the shader fields were added keep working after upgrading, without needing a rebuild, because the effect array's stride now comes from each shell's own `GnoblinEffect` record size.
- Setting `protocols.xdg-decoration = false` now keeps `zxdg_decoration_manager_v1` out of the registry, where previously the setting was ignored and clients always saw the protocol.
- A refused write no longer discards the rest of the JavaScript shell's frame: the other ops are resent, calls get `failed` answers, and `onApplied` now fires for every call.
- The wlcs test descriptor now lists the `zwp_tablet_manager_v2` global, matching what clients see in the registry once tablets are served.
- Fixed `wl_subsurface.set_position` to apply with the parent's commit, and attaching a buffer to an unconfigured `xdg_surface` now raises `xdg_surface.unconfigured_buffer` immediately.
- Fixed a race where the JavaScript shell could read stale values of `decoration`, `focused` and window size, so the stock titlebar was sometimes not mounted for a window.
- Fixes the `xr` seat being created on the first frame and listed in attach snapshots, `gnoblin_view_source_manager_v1.create_source` accepting 64-bit node ids and current scene state, and stock shell chords like `<Super>Left` and `<Super>space` now parsing so their bindings work.
- The XR frame loop now runs frames as soon as the session begins in `Ready`, not only after `Synchronized`, `Visible` or `Focused`, so Monado clients get frames.
- Seat and node `claim` and `unclaim` calls from the shell are now captured in operation order instead of being refused as unknown methods.
- A callback added with `Operation:on_complete` after the operation had finished now runs, instead of the runtime calling the Operation table and logging an error.
- The `monitor.list` bridge method no longer fails with a "Lua monitor record has no integer index" error and again lists each monitor.

#### Command line and tooling

- `gnoblinctl check` and `config init`/`config migrate` work again for a `shell.ts` that imports `gnoblin-std`, since validation now runs through the `gnoblin --role check` launcher, which reports failures with the same kinds and positions.
- `cargo xtask deps` passes its own test again, now correctly asserting that `gnoblin-std-portal` is refused access to egui.
- Fixed `cargo xtask dupes --deny` failing by renaming the portal's `serve::Options` to `ServeOptions`, which no longer duplicates the name of the input EIS options type.
- Fixes `gnoblinctl` and `gnoblin-wasm` command and interface catalogs so they no longer include the owner-only `tree` namespace, resolving clashes with the public `lock` and `destroyed` classes.

#### Documentation and tests

- Fixes CI gate failures: a duplicate test-only `View` type is renamed `EngineLink`, `ext-clippy` and `ext-test` now pass under `--locked`, `_gnoblin_core.pyi` declares `verb_call` and `verb_subscribe`, chord claims match in both `<Super>Tab` and `Super+Tab` spellings, and several racy tests are now deterministic.
- Script API values that nothing applies, such as `vrr`, `Camera2D` and `Camera3D` transforms, `Viewport.clearColor`, `background`, `filter` and inset shadows, are now documented as "Not applied yet" and each has a recorded reason.
- Compositor tests no longer fail when the GPU memory publication lands first, as `gnoblin_wayland::Config::gpu_memory_period` now defaults to one second and can be set to zero to disable publication.
- Fixed flaky `gnoblin-js` tests that assumed a fast machine by making the shell's script clock injectable via `JsShellConfig::clock`, so timing-sensitive tests use a manual clock and honest scripts get a 5 s budget.
- Fixed flaky `gnoblin-lua-sandbox` tests that failed on busy machines because the wall-clock deadline ended them before the instruction budget did.
- Shell key routing is now tested end to end: a shell node with `input` holding the seat's focus receives keys sent over EI as `tree.event` key events, and no key reaches it once the session is locked.
- Adds a deterministic test for the portal's IPC thread deferral, confirming consent offers are served before a deadline and that queued graph events and dialog answers are applied in the order they were read.
- The `scripts/xr-smoke.sh` test now starts its private session bus correctly, so the portal backend no longer fails to connect and the XR smoke test passes all 25 assertions.
- The docs site builds again, and its pages no longer describe removed features such as layer surfaces, the frame rule, and the `layout` field.
- Shell VM tests now widen the wall-clock handler deadline and wait longer for state, so `require("gnoblin.shell")` no longer fails on slow or busy machines.
- The `gnoblin-client` loopback now reuses the shared `StreamFrame` type instead of a duplicate frame type, and `cargo xtask dupes --deny` no longer warns about stale allow entries.
- The guardian tests now start a private `dbus-daemon` for each session, so they no longer fail with a 60-second timeout when `xdg-desktop-portal-gnoblin` is installed beside `gnoblin`.
- Fixes the EIS real-client pointer tests so they target the window's drawn rectangle rather than its configured geometry, so pointer events land on the surface again.
- The `candidate-workspace-list` parity scenario no longer expects the workspace `layout` property, which was removed along with the layout engine.
- Flaky shell VM tests now wait for the state they assert instead of fixed frame counts or the shell's `placed` flag before reading results.
- The Lua API reference and editor hints now document `gnoblin.windows.focused`, `windows.by_id`, `windows.snap_context`, `gnoblin.outputs.*` and `gnoblin.capabilities.list`, which already worked but were missing.

### Removed

#### Compositor and rendering

- Removed user shaders and the `shader` effect kind, which were already refused by config and had no callers; for tinting, use a `blur` effect's `tint` or draw a `Canvas`.
- Effects listed in a node's `effects` are now drawn for windows, canvases and Views, where previously they were stored but never applied.
- A node's `skipped` list now reports an effect the active renderer cannot draw as `<kind>: unsupported-by-renderer`, and the main seat exposes a read-only `effect_kinds` list so shells can adapt before setting an effect.
- The compositor no longer draws its own workspace slide; the shell's `Desk:show` tween is now the only slide, and windows hidden or shown by the shell fade like an unmap.

#### Windows, workspaces and shell

- Removes `Client.animate`; use `client.set(values, { duration, easing })` for timed property writes, which now go through the same transition engine as built-in animations.
- Without a shell, the compositor now applies only minimal window policy (cascade, maximise, fullscreen, focus on click); `focus_mode = hover`, `auto_raise` and `focus_change_on_pointer_rest` no longer act there, and a single workspace (id 1) is exposed.
- Removed `window_management.new_window_placement` and `center-new-windows`; configs that set either are now refused, and new windows are placed by the shell through `gnoblin.shell.floating` (`Floating`, `Cascade`, `Centred`).

#### Input, seats and accessibility

- Touchpad gesture action names such as `action` in `touchpad-gestures` are removed; gestures now need a `command`, and documents that still set `action` are refused with migration guidance.

#### Session, services and recovery

- The external keyring unlock prompter is removed; the built-in keyring now requests its unlock password through the `prompt.secret` slot and fails closed if no shell claims it.

#### Configuration

- Removed `defineConfig` and the declarative configuration layer; `init.ts` now runs directly against live objects, and the keyring stays on by default.
- The Lua shell and API are removed; a config directory with only `init.lua` now starts on the stock config with a warning, and `GNOBLIN_CONFIG` or `config validate` refuse `.lua` files, pointing to `gnoblinctl config migrate`.
- Documents setting `extension-dirs` or `extension_dirs` are now refused with the same migration message as `extensions`, pointing to `require` from module roots or a program on the owner socket.
- Removed `window_management.auto_maximize`, `window_management.workspaces_only_on_primary`, `window_management.button_layout` and their `config.toml` keys, so maximise-on-open, workspaces-on-primary and titlebar button layout are no longer applied; use the `workspaces` list instead.
- The `prompts` section and its `prompts.enabled` key are removed, and configurations that set them are now refused at load; `keyring.enabled` still controls the built-in keyring.
- Removes the `when` key from `touchpad_gestures` entries, so configurations that set it are refused at load, and gesture bindings now run only while unlocked and never reserve gestures on the lock screen.

#### Portals, capture and privacy

- Connections that are not the session owner are now refused all seven consent requests (`remote.grant`, `remote.refuse`, `remote.connect`, `remote.end`, `cast.grant`, `cast.start`, `cast.end`) with `denied`, and leave no state that could affect the owner's later requests.

#### Client protocols and compatibility

- The `gnoblin-blur-fade-v1` protocol is no longer provided, since blur now fades with its window's opacity.
- The `gnoblin_session_host_v1` protocol is removed, so nested sessions are now ordinary Wayland clients of their host and ordinary sessions on their own, at any depth, with the host treating a nested window like any other window.
- The tree's `workspace` class, the `Window.workspace` property and the `tree.workspace.destroyed` signal are removed, since workspaces are managed only through `gnoblin.workspaces`.
- Nodes gain a read-only `skipped` list naming each effect the renderer will not draw, such as blur or shadow on a node placed in 3D, with the reason `unsupported-effect-on-3d`.
- The unused `gnoblin_window_frame_v1` protocol and its `frame-renderers` and `protocols.window-frame-renderer` config keys are removed; delete both from your configuration, as Gnoblin draws window decorations itself through the `Titlebar` shell node.
- The `layer-refused` IPC error code is removed, so no call returns it, and later `GnoblinError` codes shift down one number; errors are still sent by name.

#### Command line and tooling

- The `gnoblinctl capabilities` command is removed; use `gnoblinctl status` for session details and `gnoblin.capabilities.list()` in a shell for shell-API capabilities.

#### Packaging and install

- The compositor, Wayland engine and core library no longer depend on any std crate; `gnoblin-std-wm` and `gnoblin-std-portal` are now only development dependencies of stock-desktop test suites, and `just core-only` checks that a std crate is absent from the core build.
- The `gnoblin-wm` crate has been removed, with no user-visible behaviour change, since window placement, focus, rules, workspaces and drag now come from the stock shell.

#### Documentation and tests

- Removed the non-existent `gnoblin-core/schema` declaration modules from `gnoblin.d.ts`, so imports that type-checked but failed at run time are gone; `gnoblin.d.ts` now declares only the `gnoblin` and `console` globals.
- The never-published `gnoblin.layer.created`, `gnoblin.layer.changed` and `gnoblin.layer.removed` events are removed from the docs, and layer-shell behaviour is now documented in the protocols page.
