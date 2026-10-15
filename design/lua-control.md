# Lua control surface audit

## Finding

Lua already controls many day-to-day window-manager operations. This is more than a configuration-only API: it can move and resize windows in two dimensions, change workspaces and monitors, make a window sticky or above others, make trusted focus and interactive-drag requests, make input delivery decisions, declare animations, and request lock or logout.

The control path is typed and mediated by Gnoblin; Lua does not receive `MetaWindow`, `ClutterActor`, or libinput objects.

The gap for making Lua the control language is ownership and coverage. Lua cannot configure outputs, set general runtime focus policy, arrange layer-shell surfaces, access clipboard or screen-capture streams, or run idle and frame callbacks. Several Mutter preferences are only set by configuration load/reload. Session power actions, focus history and policy reads, and config path/default/restore reads now have Lua equivalents; `gnoblinctl init` remains CLI-only.

This audit treats the checked-in LuaCATS headers and native-control handlers as the implementation source of truth. `design/lua-api.md` is a target design and contains proposed signatures alongside current APIs; a signature there is not proof that the runtime implements it. The source submodule is pinned at `15761641006132ab027e86b997fa22be06e7e8e9`, but its checkout is absent in this worktree.

Mutter mechanisms below are therefore named from Gnoblin's applied patches and their referenced Mutter functions. The table does not claim to have re-audited unpatched upstream implementation at that pin.

## Capability coverage

`None` means no Lua state or operation for the capability. `Read only` means Lua can inspect state but cannot change it. `Partial` means Lua can change a restricted subset, or only through a trusted context/configuration path. `Full` means the Lua surface covers the described operation; it does not mean Lua owns hardware access or can bypass compositor validation.

### Window move

- **Mutter or Gnoblin mechanism (file and function):** `patches/mutter/99-typed-window-api/0001-dispatch-typed-window-operations.patch`: `meta_window_move_frame`; Gnoblin handler in `src/control/gnoblin-control-requests.c`
- **Current Lua access:** Full for 2D logical position: `Window:move(position)`; interactive move is context-bound
- **What is needed:** Nothing beyond the existing 2D behavior.

### Window resize

- **Mutter or Gnoblin mechanism (file and function):** Same typed-operation patch: `meta_window_move_resize_frame`; `patches/mutter/99-typed-window-api/0065-native-window-resize-user-operation.patch` and `0076-animate-typed-window-resizes.patch`
- **Current Lua access:** Full for 2D frame size; interactive resize requires a live shortcut context
- **What is needed:** Nothing beyond the existing 2D behavior.

### Focus and activation policy

- **Mutter or Gnoblin mechanism (file and function):** `src/control/gnoblin-control-focus.c` context/token validation; Mutter `meta_window_activate_full` in `patches/mutter/52-focus-transfer/0001-honour-app-activation.patch`; policy preferences are applied from Gnoblin config
- **Current Lua access:** Partial: `Window:focus(context)` and `begin_move` are trusted-context operations; Lua can read `gnoblin.focus.policy()` and `gnoblin.focus.history(filter)`. No general runtime setter or activation-policy callback
- **What is needed:** Add an explicit focus-policy declaration/runtime update. Keep activation tokens and one-use user contexts as mandatory authorization inputs; do not expose unconditional focus-stealing.

### Stacking and layers

- **Mutter or Gnoblin mechanism (file and function):** Mutter restack signal `display_restacked` in `src/control/gnoblin-control-api.c`; `meta_window_raise` is used by `patches/mutter/30-layer-shell/0002-layer-shell-honor-wlr-stacking-layers.patch`
- **Current Lua access:** Partial: inspect layers/windows; set window above/sticky; no arbitrary stack index or layer assignment
- **What is needed:** Add typed raise/lower/relative-order operations where Mutter permits them, plus explicit z-order rules for spatial surfaces. Preserve layer-shell protocol ownership and activation rules.

### Workspaces

- **Mutter or Gnoblin mechanism (file and function):** `meta_workspace_activate` and workspace manager operations in `patches/mutter/90-native-workspaces/0001-list-and-switch-workspaces-in-native-compositor.patch` and `91-native-workspace-management/0001-manage-numeric-workspaces-in-native-compositor.patch`; handlers in `src/control/windows/gnoblin-control-workspaces.c`
- **Current Lua access:** Full for current model: create, remove, rename, activate, next/previous, move windows; workspace records are revisioned
- **What is needed:** Nothing beyond the current model. Retain stable workspace IDs rather than using position as identity.

### Monitor and output configuration

- **Mutter or Gnoblin mechanism (file and function):** `meta_monitor_manager_get_logical_monitors` and monitor snapshot code in `patches/mutter/99-typed-window-api/0007-native-monitor-snapshot-records.patch`; stable connector lookup in `0006-stable-monitor-connector-ids.patch`
- **Current Lua access:** Read only for topology, current geometry, scale, refresh rate, and transform; partial runtime override for privacy screen and tablet mapping. No output profile/mode/enable/arrangement API
- **What is needed:** Integrate the output configuration API with Lua and transactional validation/rollback.

### Layer-shell arrangement

- **Mutter or Gnoblin mechanism (file and function):** `meta_wayland_init_layer_shell` in `patches/mutter/30-layer-shell/0001-layer-shell-wire-wlr-layer-shell-into-the-build.patch`; snapshot getter and Lua `lua_layers_list` in `src/config/lua/gnoblin-lua-queries.c`
- **Current Lua access:** Read only for layer snapshots and animation policy; partial configuration through window rules and `preserve_active_window`. Anchors, exclusive zones, size, and surface role remain client/protocol state
- **What is needed:** Add compositor-mediated surface placement and docking targets for Lua policy, with protocol constraints and configure/ack lifecycle.

### Keyboard input

- **Mutter or Gnoblin mechanism (file and function):** `native_input_event_type`, `native_input_send_head`, and `native_input_forward_all` in `src/control/gnoblin-control-input-routing.c`; interception/replay in `patches/mutter/99-typed-window-api/0103-lua-input-policy-event-routing.patch`
- **Current Lua access:** Full for matched events' forward-or-consume decision; shortcut capture/bind, key source selection, and text insertion are exposed. No synthetic remapping
- **What is needed:** Add validated remapping only with delivered modifier/key state transitions and release pairing. Preserve lock and capture embargoes.

### Pointer input

- **Mutter or Gnoblin mechanism (file and function):** `native_input_event_type` and `native_input_send_head` in `src/control/gnoblin-control-input-routing.c`; trusted focus/drag token handling in `src/control/gnoblin-control-focus.c`
- **Current Lua access:** Full for matched button, motion, and scroll delivery decisions; partial for pointer warp, cursor shape, and hit-target selection, which are not general Lua operations
- **What is needed:** Add device/cursor control and controller-to-pointer mapping as explicit operations.

### Touch

- **Mutter or Gnoblin mechanism (file and function):** `native_input_event_type`, `native_input_is_stream_begin`, and `native_input_is_terminal` in `src/control/gnoblin-control-input-routing.c` serialize touch events and retain stream decisions
- **Current Lua access:** Partial: touch event types and stream decisions exist, but no touch injection or Lua-owned gesture recognizer API is exposed
- **What is needed:** Add documented touch phases and cancellation/ownership helpers if needed; retain begin/update/end consistency.

### Gestures

- **Mutter or Gnoblin mechanism (file and function):** `patches/mutter/78-touchpad-events/0001-forward-touchpad-gesture-events.patch`; `src/control/gnoblin-control-events.c` (`filter_native_touchpad_gestures`, `run_native_touchpad_action`); input router encodes gesture fields
- **Current Lua access:** Partial: touchpad gestures are observable and configuration can bind supported native actions; event-policy hooks can consume/forward matched gestures. Lua cannot register a general gesture recognizer with accumulated motion state
- **What is needed:** Add a gesture-session object with start/update/end/cancel, device identity, normalized deltas, and timeout/reload cancellation.

### Tablet and pen

- **Mutter or Gnoblin mechanism (file and function):** `input_device_record` and `normalize_input_value` in `src/control/gnoblin-control-input-routing.c` and `src/control/gnoblin-control-input-config.c`; tablet output patch `0090-lua-tablet-output-mapping.patch`
- **Current Lua access:** Partial: device inspection, tablet/stylus/pad preferences and output mapping, pad-help event; no full pen pose/pressure routing or controller abstraction
- **What is needed:** Add normalized pen/tablet event fields and per-device policy hooks; add controller devices through the same explicit device/capability model rather than pretending they are mice.

### Cursor

- **Mutter or Gnoblin mechanism (file and function):** `meta_prefs_set_gnoblin_cursor_config` in `patches/mutter/70-config-preferences/0002-live-gnoblin-cursor-preferences.patch`; preference application is config-driven
- **Current Lua access:** Partial: theme/size config, launch busy-cursor feedback, locate-pointer event. No runtime theme/size setter or cursor positioning/shape API
- **What is needed:** Add typed cursor preference override plus per-seat cursor image/visibility operations; leave actual rendering and seat authority native.

### Animation and effects

- **Mutter or Gnoblin mechanism (file and function):** Lua methods in `src/config/lua/gnoblin-lua-animation.c`; compositor runtime in `patches/mutter/99-typed-window-api/0024-native-animation-runtime.patch`; effect rules in `src/control/windows/gnoblin-control-window-rules.c` (`native_apply_window_rules`)
- **Current Lua access:** Partial: named lifecycle animations and preview controls; rules can set opacity, blur, shader, corners, shadows, and frames. No general live effect graph or frame-by-frame Lua callback
- **What is needed:** Add bounded runtime effect properties and an animation timeline/event contract. Keep GPU work and frame pacing in Mutter; Lua supplies policy/targets, never executes on the render thread.

### Snapping and tiling

- **Mutter or Gnoblin mechanism (file and function):** `native_socket_create_snap_context` in `src/control/gnoblin-control-text-snap.c`; drag state in `src/control/windows/gnoblin-control-window-drag.c`; Mutter `meta_window_move_resize_frame`
- **Current Lua access:** Partial: trusted context can compute and commit a target rectangle; edge tiling and native keybindings are config preferences. No layout tree or persistent tiled-set API
- **What is needed:** Add declarative layout trees or typed snap zones, membership/constraints, and resize handles. Do not call a single rectangle commit a general tiling model.

### Decorations and frames

- **Mutter or Gnoblin mechanism (file and function):** Native renderer service in `patches/mutter/61-window-frames/`; Gnoblin `native_apply_window_rules` in `src/control/windows/gnoblin-control-window-rules.c`; rule schema in `src/config/lua/api/config.h`
- **Current Lua access:** Partial: config-time frame renderer registry, per-window frame/corner/shadow rules; no general runtime frame style setter or renderer lifecycle control
- **What is needed:** Add revisioned runtime decoration properties and renderer capability/ready/failure events; keep client-side decoration negotiation explicit.

### Session power

- **Mutter or Gnoblin mechanism (file and function):** `src/cli/gnoblinctl.c` `run_power_action` calls systemd-logind `CanSuspend`/`Suspend`, `CanHibernate`/`Hibernate`, `CanReboot`/`Reboot`, `CanPowerOff`/`PowerOff`
- **Current Lua access:** Full for `gnoblin.session.suspend()`, `hibernate()`, `reboot()`, and `poweroff()`. These queue operations; the native session supervisor checks logind capability and requests the action. Lua also has session lock and logout
- **What is needed:** Add `gnoblin.session.power(action)` as an asynchronous, capability-checked request to a Gnoblin session service. Keep authorization and logind availability errors explicit.

### Lock and logout

- **Mutter or Gnoblin mechanism (file and function):** Session operations in `src/control/gnoblin-control-requests.c`; status/activity readers `lua_session_status` and `lua_session_activity` in `src/config/lua/gnoblin-lua-state-queries.c`
- **Current Lua access:** Partial: Lua requests both; logout is an operation, while lock is forwarded to a subscribed shell and is not confirmed by the request result
- **What is needed:** Add lock completion/result semantics only if Gnoblin owns the lock UI/service; preserve current distinction between request and confirmed locked state.

### Clipboard

- **Mutter or Gnoblin mechanism (file and function):** `data_device_set_selection` and `primary_device_set_selection` in `patches/mutter/73-session-lock-privacy/0001-session-lock-embargo-clipboard-and-drag-offers.patch`
- **Current Lua access:** None as a Lua clipboard read/write API. Permission rules can describe remote-desktop clipboard grants; text insertion is a separate trusted target operation
- **What is needed:** Add a permission-gated clipboard broker with MIME, size, origin, and lock-state rules; do not expose raw selection objects.

### Screen capture

- **Mutter or Gnoblin mechanism (file and function):** `meta_wayland_init_screencopy` in `patches/mutter/30-screencopy/0001-screencopy-wire-wlr-screencopy-into-the-build.patch`; bounded `native_runtime_begin_thumbnail` / `native_thumbnail_idle` in `src/control/gnoblin-native-control.c`
- **Current Lua access:** Partial: capture protocol can be enabled/disabled; Lua can request a bounded window thumbnail and inspect privacy state. No stream/frame API or output capture operation
- **What is needed:** Add a permission-aware capture session API with bounded buffers, cancellation, damage/revision metadata, and explicit CPU/GPU path reporting. Do not treat a thumbnail as a capture stream.

### Idle

- **Mutter or Gnoblin mechanism (file and function):** `session.activity` snapshot/publication in `src/control/gnoblin-control-runtime-channel.c` and `src/control/gnoblin-native-control.c`; `lua_session_activity` in `src/config/lua/gnoblin-lua-state-queries.c`
- **Current Lua access:** Read only: `gnoblin.session.activity()` and activity events. Lua cannot set idle thresholds or inhibit/schedule idle actions
- **What is needed:** Add subscriptions/threshold timers and an inhibition lease API, with lock/suspend policy owned by the session service.

### Startup

- **Mutter or Gnoblin mechanism (file and function):** `validate_autostart_entries` in `src/config/lua/gnoblin-lua-named.c`; `gnoblin_control_command_spawn` / `gnoblin_control_launch_command` in `src/control/gnoblin-control-command.c`; session startup in `src/session/gnoblin-runtime-autostart.c`
- **Current Lua access:** Partial: autostart is declared at config load; Lua can run/capture commands and inspect launch feedback. No runtime autostart lifecycle or restart policy API
- **What is needed:** Add managed service/process records (`start`, `stop`, `restart`, status, exit events) only if the session supervisor owns that lifecycle.

### Per-frame hooks

- **Mutter or Gnoblin mechanism (file and function):** Mutter plugin lifecycle callbacks in `src/compositor/plugin/gnoblin-plugin.c` (`map`, `minimize`, `unminimize`, `destroy`, `size_changed`, effect cancellation, workspace cancellation); layer frame callback patch `patches/mutter/30-layer-shell/0005-layer-shell-emit-frame-callbacks-for-visible-layer-surfaces.patch`
- **Current Lua access:** None. Lua gets lifecycle and input events, not a frame-clock callback
- **What is needed:** Keep Lua off the paint path.
    - If frame policy is required, expose declarative frame-boundary state or sampled notifications with strict rate/budget limits; never synchronously call Lua per rendered frame.

## `gnoblinctl` operations without a Lua equivalent

I ran `gnoblinctl help` and `gnoblinctl help COMMAND` for each command with actions, then compared that inventory with `src/cli/gnoblinctl-cli.c`, `src/config/lua/api/*.h`, and the runtime handlers. “Equivalent” here means a typed Lua API operation with the same session behavior, not that a script can spawn the CLI executable. Read-only API properties count as equivalents.

### `ping`

- **Lua equivalent:** None
- **Notes:** Transport liveness probe; no runtime policy operation.

### `completion`

- **Lua equivalent:** None
- **Notes:** Shell completion text generation is CLI-only.

### `config path`

- **Lua equivalent:** `gnoblin.config.path()`
- **Notes:** Returns the active configuration file path.

### `config default`

- **Lua equivalent:** `gnoblin.config.default()`
- **Notes:** Returns the embedded default Lua source.

### `config restore-default`

- **Lua equivalent:** `gnoblin.config.restore_default()`
- **Notes:** Queues an operation that backs up and replaces the standard config tree, then reloads it.

### `init`

- **Lua equivalent:** None
- **Notes:** CLI-only by design. It creates a config tree before any session or Lua runtime exists, so there is no Lua caller. In a running session, `gnoblin.config.restore_default()` recreates the tree.

### `session suspend`

- **Lua equivalent:** `gnoblin.session.suspend()`
- **Notes:** Queues a capability-checked request for the native session supervisor to call logind.

### `session hibernate`

- **Lua equivalent:** `gnoblin.session.hibernate()`
- **Notes:** Queues a capability-checked request for the native session supervisor to call logind.

### `session reboot`

- **Lua equivalent:** `gnoblin.session.reboot()`
- **Notes:** Queues a capability-checked request for the native session supervisor to call logind.

### `session poweroff`

- **Lua equivalent:** `gnoblin.session.poweroff()`
- **Notes:** Queues a capability-checked request for the native session supervisor to call logind.

### `window menu`

- **Lua equivalent:** No matching Lua menu operation
- **Notes:** The CLI rejects it because it lacks a trusted shell input context. Lua has trusted move/resize context helpers, but no general menu invocation in `src/config/lua/api/*.h`.

### `window interactive-move`

- **Lua equivalent:** Context-bound equivalent only
- **Notes:** The CLI rejects it; `Window:begin_move(context)` is available only from a live trusted shortcut callback.

### `window interactive-resize`

- **Lua equivalent:** Context-bound equivalent only
- **Notes:** Same restriction; `Window:begin_resize(edge, context)` is the Lua operation.

### `window match`

- **Lua equivalent:** No dedicated equivalent
- **Notes:** Lua can inspect window records and define `gnoblin.window_rule`; it cannot request this CLI's rule-match diagnostic directly.

### `status`

- **Lua equivalent:** Partial
- **Notes:** Combine `gnoblin.session.status()` and `gnoblin.runtime.status()`; no single combined status record.

### `--version` (installed build)

- **Lua equivalent:** Partial
- **Notes:** `gnoblin.version()` reports the running compositor/runtime identity, not the standalone installed CLI build when no session is running.

Other listed actions have direct runtime equivalents: focus history/policy, reload, logout, session activity/lock, privacy, permissions, window state and geometry operations, workspace operations, monitor/layer/input snapshots, shortcut operations, grants, launches, animations, and the Lua console's runtime API. The CLI's output format, socket path, and request timeout are transport/presentation options, not session capabilities.

## Configuration without runtime control

`gnoblin.configure(settings)` declares/merges the config document during configuration evaluation. `gnoblin.settings` is a read-only committed snapshot. Neither is a generic live setter. Reload applies supported settings; that is different from a Lua runtime operation. The following config areas have no general runtime setter. The exceptions below are the specific runtime overrides that exist today.

### `window_management`: focus mode/new-window focus, raising delays, auto-maximize, liveness timeout, primary-only workspaces, edge tiling, center-new-windows, modal attachment, X11 workarounds, drag work-area constraint, titlebar actions and button layout; initial `workspaces` ID/name declarations

- **Runtime status:** Config/reload only. `gnoblin.focus.policy()` reads effective policy; no setter. Typed window/workspace operations change individual objects, not these policies or the initial workspace declaration.

### `layer_shell.preserve_active_window`

- **Runtime status:** Config/reload only.

### `cursor.theme`, `cursor.size`; `compositor.locate_pointer`, `locate_pointer_key`, visual/audible bell and bell type

- **Runtime status:** Config/reload only. Locate-pointer and bell events can be observed, but Lua does not set these preferences at runtime.

### `frame_renderers` registry and `window_rule` frame/corner/shadow/blur/shader/opacity/workspace/animation properties

- **Runtime status:** Config-time declarations/rules. Animation preview controls are runtime test controls, not setters for live window rules or renderer registrations.

### `animation` declarations and `window_rule.animation` mappings

- **Runtime status:** Declarations are evaluated as config. Preview/seek/play/pause/stop are runtime controls for preview sessions only.

### `input.mouse.*`, `input.touchpad.*`, `input.trackball.*`, `input.pointing_stick.*`, `input.keyboard.*`, `input.tablets.*`, `input.styluses.*`

- **Runtime status:** Every documented leaf under these paths is config/reload policy. `input.select_source`, `input.set_orientation_lock`, and tablet output mapping have narrow runtime paths; there is no generic `input.set_preferences`.

### `input_sources.sources`, `input_sources.per_window`, `touchpad_gestures.*`

- **Runtime status:** Config/reload only.
    - Lua can select an already configured source and observe/decide supported gesture events, but cannot replace the source list or gesture recognizer configuration at runtime.

### `keybindings` and static `shortcuts` declarations

- **Runtime status:** Config/reload for static bindings. `gnoblin.shortcuts.bind/unbind` provides a separate dynamic runtime shortcut subset; it does not edit the full config document.

### `monitors.privacy_screen`

- **Runtime status:** Has runtime override `gnoblin.monitors.set_privacy_screen(value)`; monitor topology, mode, scale, transform, and placement remain read-only.

### `autostart` entries and trigger (`on_login`)

- **Runtime status:** Config/reload only. `gnoblin.commands.run/capture` launches an individual command but does not manage autostart entries or their lifecycle.

### `protocols.*` advertisement switches, including `wlr_layer_shell`, foreign-toplevel protocols, decoration/frame protocols, `wlr_screencopy`, background effects, `ext_data_control`, `ext_idle_notify`, output power management, and session lock

- **Runtime status:** Config/reload only. They govern protocol advertisement; they are not per-client runtime toggles.

### `portals.default`, `portals.interfaces`, `permissions.default`, and configured permission rules

- **Runtime status:** Config/reload for policy. Runtime APIs can inspect policy, answer pending authorization, and revoke individual grants; they do not edit these policy declarations.

### `location.enabled`, `location.max_accuracy`, `auth.polkit_agent`, `prompts.enabled`

- **Runtime status:** Config/reload only, aside from answering an already-pending authorization, authentication, or passphrase-prompt request.

### `xwayland.allow_grabs`, `grab_access_rules`, `disable_extensions`, `allow_byte_swapped_clients`, `scaling_factor`

- **Runtime status:** Config/reload only.

The exact accepted keys and defaults are documented in the linked config pages:

[window management](/config/configure/window_management), [input](/config/configure/input), [input sources](/config/configure/input_sources), [touchpad gestures](/config/configure/touchpad_gestures), [cursor](/config/configure/cursor), [compositor](/config/configure/compositor), [layer shell](/config/configure/layer_shell), [monitors](/config/configure/monitors), [frame renderers](/config/configure/frame_renderers), [protocols](/config/configure/protocols), [autostart](/config/configure/autostart),

[portals](/config/configure/portals), [permissions](/config/configure/permissions), [location](/config/configure/location), [authentication](/config/configure/auth), and [Xwayland](/config/configure/xwayland).

## Gap ranking and small-PR sequence

Ranking weighs the owner's stated goal: make Lua the control language for window management. Signatures below are proposals, not existing API. Each PR should update its LuaCATS header and generated signature/docs artifacts when implemented.

### 1. Output control

- **Value and bounded outcome:** Foundational.
    - Give Lua an output namespace for displays; validate and roll back configuration in Gnoblin's output service.
    - Coordinate the shape with `design/outputs.md` before implementation.
- **Proposed LuaCATS-style signatures:** `---@class OutputSelector`<br>`---@field id string Stable output ID`<br>`---@class OutputConfiguration`<br>`---@field outputs OutputSpec[] Desired enabled outputs and modes`<br>`function gnoblin.outputs.list() end`<br>`function gnoblin.outputs.configure(configuration) end`<br>`function gnoblin.outputs.by_id(id) end`

### 2. Focus, activation, stacking policy

- **Value and bounded outcome:** Makes focus/raise behavior programmable while preserving user-activation proof. Add policy hooks before expanding privileged mutations.
- **Proposed LuaCATS-style signatures:** `function gnoblin.focus.set_policy(policy) end`<br>`function gnoblin.focus.on_request(callback) end`<br>`function Window:raise(context) end`<br>`function Window:lower(context) end`

### 3. Layout, snapping, and tiling

- **Value and bounded outcome:** Promote current one-shot rectangle snap to reusable layout rules and constrained resize behavior.
- **Proposed LuaCATS-style signatures:** `function gnoblin.layouts.define(layout) end`<br>`function gnoblin.layouts.apply(layout, target) end`<br>`function Window:set_layout_area(area) end`<br>`function SnapContext:commit(request) end` (existing)

### 4. Runtime compositor preferences

- **Value and bounded outcome:** Remove config/reload as the only route for cursor, input-device, and decoration preferences. Add typed overrides per group and return the effective source/revision. Focus policy is covered by PR 2.
- **Proposed LuaCATS-style signatures:** `function gnoblin.cursor.set_theme(theme, size) end`<br>`function gnoblin.input.set_device_preferences(device, preferences) end`<br>`function Window:set_decoration(decoration) end`<br>`function gnoblin.settings.on_changed(callback) end`

### 5. Capture and clipboard sessions

- **Value and bounded outcome:** Complete shell workflow with explicit permissions and bounded resource lifetime. Keep capture separate from clipboard APIs.
- **Proposed LuaCATS-style signatures:** `function gnoblin.capture.begin(options) end`<br>`function Capture:next_frame() end`<br>`function Capture:stop() end`<br>`function gnoblin.clipboard.read(options) end`<br>`function gnoblin.clipboard.write(items) end`

### 6. Session lifecycle and idle policy

- **Value and bounded outcome:** Add missing power operations and give Lua explicit idle timers/inhibitors. Keep logind and lock capability checks in native services.
- **Proposed LuaCATS-style signatures:** `function gnoblin.session.power(action) end`<br>`function gnoblin.idle.on(threshold_ms, callback) end`<br>`function gnoblin.idle.inhibit(reason, options) end`

### 7. Managed startup

- **Value and bounded outcome:** Allow policy-driven session components with supervision rather than detached one-off command spawning.
- **Proposed LuaCATS-style signatures:** `function gnoblin.services.start(spec) end`<br>`function gnoblin.services.stop(id) end`<br>`function gnoblin.services.restart(id) end`<br>`function gnoblin.services.list() end`

### 8. Frame-adjacent policy

- **Value and bounded outcome:** Lowest priority and should not become a per-frame Lua callback. Use this only for measured needs that cannot be expressed as events or animation declarations.
- **Proposed LuaCATS-style signatures:** `function gnoblin.render.on_state_changed(callback) end`<br>`function gnoblin.animations.schedule(spec) end`

PR 1 should land first because the output coordinate system underpins later work. PRs 2–8 can then proceed as separate vertical slices.
