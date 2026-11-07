# Constants audit: wayland group

Read-only audit of the constants in `gnoblin-wayland`, `gnoblin-input`, `gnoblin-portal`, `gnoblin-portal-core` and `gnoblin-a11y`. No code was changed or built. Each row answers three questions:

- **Q1** Would a user want this configurable?
- **Q2** Does an upstream source (protocol, spec, D-Bus, GNOME, kernel, library) own the value?
- **Q3** Is it needed at all?

Evidence paths are relative to the worktree root. Unverified claims are marked [INFERENCE].

## 1. Summary per crate

Counts come from `grep -rnE '^\s*(pub(\([a-z]+\))? )?(const|static) [A-Z_0-9]+'` over `src/`. Test-only items live in `#[cfg(test)]` modules or `tests.rs`. Rows group families that share the same answers.

| Crate               | Total | Test-only | Listed individually | Grouped rows | Notes                                             |
| ------------------- | ----- | --------- | ------------------- | ------------ | ------------------------------------------------- |
| gnoblin-wayland     | 35    | 1         | 14                  | 7            | Protocol versions, bitmasks, caps, timeouts       |
| gnoblin-input       | 23    | 2         | 9                   | 6            | Chord, gesture and policy limits                  |
| gnoblin-portal      | 0     | 0         | 0                   | 0            | No constants; `screen.rs` is logic only           |
| gnoblin-portal-core | 4     | 0         | 3                   | 1            | Dialog pixels, consent arm delay, interface table |
| gnoblin-a11y        | 36    | 7         | 6                   | 4            | Keysyms (26), test fixtures (7)                   |

Total: 98 items. Production items: 89.

## 2. Findings

### 2.1 Protocol manager versions (gnoblin-wayland)

These are the `version` passed to `create_global`. Each should match the protocol XML, which is the upstream source.

| Constant                                               | Location                                                                       | Value | Verdict | Q1  | Q2 (source)                                                               | Q3                          | Risk | Effort |
| ------------------------------------------------------ | ------------------------------------------------------------------------------ | ----- | ------- | --- | ------------------------------------------------------------------------- | --------------------------- | ---- | ------ |
| `ext_background_effect_v1::MANAGER_VERSION`            | crates/gnoblin-wayland/src/protocols/ext_background_effect_v1.rs:36            | 1     | DERIVE  | No  | Matches `version="1"` in ext-background-effect-v1 XML, see 5              | Keep as one generated value | low  | S      |
| `zwlr_foreign_toplevel_management_v1::MANAGER_VERSION` | crates/gnoblin-wayland/src/protocols/zwlr_foreign_toplevel_management_v1.rs:76 | 3     | DERIVE  | No  | Matches `version="3"` in wlr-foreign-toplevel-management XML, see 5       | Keep, generate from XML     | low  | S      |
| `zwlr_gamma_control_v1::MANAGER_VERSION`               | crates/gnoblin-wayland/src/protocols/zwlr_gamma_control_v1.rs:66               | 1     | DERIVE  | No  | Matches `version="1"` in wlr-gamma-control XML, see 5                     | Keep, generate from XML     | low  | S      |
| `zwlr_output_management_v1::MANAGER_VERSION`           | crates/gnoblin-wayland/src/protocols/zwlr_output_management_v1.rs:55           | 4     | DERIVE  | No  | Matches `version="4"` in wlr-output-management XML, see 5                 | Keep, generate from XML     | low  | S      |
| `zwlr_output_power_management_v1::MANAGER_VERSION`     | crates/gnoblin-wayland/src/protocols/zwlr_output_power_management_v1.rs:49     | 1     | DERIVE  | No  | Matches `version="1"` in wlr-output-power-management XML, see 5           | Keep, generate from XML     | low  | S      |
| `xdg_foreign::VERSION`                                 | crates/gnoblin-wayland/src/protocols/xdg_foreign.rs:74                         | 1     | DERIVE  | No  | Used for both exporter and importer globals, see 5 [INFERENCE: version 1] | Keep, generate from XML     | low  | S      |

Why these are not user-configurable: advertising a higher version than the code implements breaks clients, and the value is a wire contract. The cleaner route is to generate the value from the protocol XML at build time, so a protocol bump cannot drift from the code. Smithay already does this for its own globals, so the generator path is established [INFERENCE].

Note: the `ext_idle_notifier_v1` XML is version 2 (see 5), but this crate sets no version constant for it. Check its global creation before the next protocol bump [INFERENCE].

### 2.2 Protocol enum and bit flags (gnoblin-wayland)

| Constant                                                                 | Location                                                              | Value        | Verdict   | Q1  | Q2                                                                        | Q3                                                   | Risk   | Effort |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------- | ------------ | --------- | --- | ------------------------------------------------------------------------- | ---------------------------------------------------- | ------ | ------ |
| `foreign_toplevel_shared::{MAXIMISED, MINIMISED, ACTIVATED, FULLSCREEN}` | crates/gnoblin-wayland/src/protocols/foreign_toplevel_shared.rs:47-53 | 1<<0 .. 1<<3 | KEEP      | No  | Values are the `zwlr_foreign_toplevel_handle_v1.state` enum (wire format) | Could use the generated enum directly; keep the bits | low    | S      |
| `ext_session_lock_v1::COVER`                                             | crates/gnoblin-wayland/src/protocols/ext_session_lock_v1.rs:99        | black        | CONFIGURE | Yes | No upstream value; the colour of the locked cover is a compositor choice  | Could be a lock-screen leaf                          | medium | S      |

The lock cover colour is a real user choice: a dark wallpaper or a theme colour may suit better. Proposed leaf: `lock.cover-colour`, colour, default `#000000`, and it belongs in core config. Any change must keep the cover opaque. [INFERENCE: no config leaf exists for lock cover today; grep found none.]

### 2.3 Keyboard repeat (gnoblin-wayland)

Repeat is already configured. The keyboard record in `crates/gnoblin-config/schema/config.toml` has leaves `repeat` (bool), `delay` and `repeat-interval` (both min 1, max 10000).

| Constant                        | Location                                       | Value   | Verdict | Q1                                                     | Q2                                                                                                                                                | Q3                                             | Risk   | Effort |
| ------------------------------- | ---------------------------------------------- | ------- | ------- | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------- | ------ | ------ |
| `state::KEY_REPEAT_DELAY`       | crates/gnoblin-wayland/src/state.rs:102        | 200 ms  | DERIVE  | Already configurable: `input.keyboard.delay`           | Default should come from the session. GNOME uses `org.gnome.desktop.peripherals.keyboard delay` [INFERENCE: key name, not checked against schema] | Keep as fallback only                          | medium | S      |
| `state::KEY_REPEAT_INTERVAL`    | crates/gnoblin-wayland/src/state.rs:104        | 40 ms   | DERIVE  | Already configurable: `input.keyboard.repeat-interval` | Same source as above, key `repeat-interval` [INFERENCE]                                                                                           | Keep as fallback only                          | medium | S      |
| keyboard rate `1000 / interval` | crates/gnoblin-wayland/src/keyboard.rs:218-227 | derived | REMOVE  | No                                                     | `wl_keyboard.repeat_info` takes rate in keys per second, so the derivation is the correct conversion                                              | No separate rate leaf exists; keep it that way | low    | S      |

Notes:

- The 200 ms and 40 ms defaults differ from GNOME's usual settings [INFERENCE]. The code should take the value from the session setting when one exists, so the fallback rarely matters. A change to the default would be visible to users who never set it.
- The `enabled` path (`keyboard.rs:223`) turns repeat off by sending rate 0. This is correct for `wl_keyboard`.
- Proposed config path: already present, so no new leaf.

### 2.4 Timeouts (gnoblin-wayland)

| Constant                                  | Location                                 | Value | Verdict   | Q1                                                            | Q2                                                                               | Q3                                       | Risk   | Effort |
| ----------------------------------------- | ---------------------------------------- | ----- | --------- | ------------------------------------------------------------- | -------------------------------------------------------------------------------- | ---------------------------------------- | ------ | ------ |
| `frame::SCREENSHOT_TIMEOUT`               | crates/gnoblin-wayland/src/frame.rs:52   | 10 s  | CONFIGURE | Maybe; slow capture on weak GPUs                              | None                                                                             | Keep as bound                            | low    | S      |
| `server::FOCUS_TIMEOUT`                   | crates/gnoblin-wayland/src/server.rs:80  | 2 s   | KEEP      | No                                                            | None                                                                             | Internal wait on focus handover          | low    | S      |
| `server` lock wait `recv_timeout`         | crates/gnoblin-wayland/src/server.rs:674 | 3 s   | KEEP      | No                                                            | None                                                                             | Internal bound on the lock command       | low    | S      |
| `state::LAYOUT_COMMAND_WAIT`              | crates/gnoblin-wayland/src/state.rs:533  | 2 s   | KEEP      | No                                                            | None                                                                             | Internal, a runtime deadline             | low    | S      |
| `state::LAYOUT_FRAME_BUDGET`              | crates/gnoblin-wayland/src/state.rs:530  | 4 ms  | KEEP      | No                                                            | None                                                                             | Frame budget, tied to the refresh rate   | low    | S      |
| liveness timeout `Duration::from_secs(5)` | crates/gnoblin-wayland/src/state.rs:775  | 5 s   | DERIVE    | Already configurable: `window-management.check-alive-timeout` | Hard-coded value should read the config value                                    | Remove the literal; use the config value | medium | S      |
| `server` `check_alive_timeout` default    | crates/gnoblin-wayland/src/server.rs:194 | 5 s   | KEEP      | Already configurable                                          | Default matches the Lua default in `data/default-config/config/10-windows.lua:9` | Keep as the default                      | low    | S      |

Finding: `state.rs:775` ignores the configured check-alive timeout. The config leaf exists (`window-management.check-alive-timeout`, `config.toml` line 1560) and `10-windows.lua:9` sets it to 5000. The literal at `state.rs:775` is a second source of truth. This is a real bug-shaped finding, and the cleanest fix is to read the configured value there. Verdict: DERIVE. Risk: medium, because a changed setting would start to apply after the next restart only.

Timeouts for protocol and wayland internals are not user-facing and stay as `KEEP`.

### 2.5 Protocol caps and limits (gnoblin-wayland)

| Constant                                          | Location                                                            | Value        | Verdict | Q1  | Q2                                                             | Q3                                   | Risk | Effort |
| ------------------------------------------------- | ------------------------------------------------------------------- | ------------ | ------- | --- | -------------------------------------------------------------- | ------------------------------------ | ---- | ------ |
| `ext_idle_notify_v1::IDLE_HOLDER_CAP`             | crates/gnoblin-wayland/src/protocols/ext_idle_notify_v1.rs:72       | 256 names    | KEEP    | No  | Our own resource cap, not upstream                             | Needed against unbounded clients     | low  | S      |
| `xdg_activation_v1::TOKEN_LIFETIME`               | crates/gnoblin-wayland/src/protocols/xdg_activation_v1.rs:96        | 10 000 ms    | KEEP    | No  | Our choice; the protocol does not fix a lifetime [INFERENCE]   | Keep as security bound on tokens     | low  | S      |
| `xdg_activation_v1::ISSUED_SERIALS`               | crates/gnoblin-wayland/src/protocols/xdg_activation_v1.rs:102       | 64           | KEEP    | No  | Our own cap                                                    | Needed against unbounded state       | low  | S      |
| `xdg_activation_v1::LEADER`                       | crates/gnoblin-wayland/src/protocols/xdg_activation_v1.rs:174       | 0            | REMOVE  | No  | Unclear purpose from the name                                  | Use an enum or explain in place      | low  | S      |
| `xdg_foreign::HANDLE_BYTES`                       | crates/gnoblin-wayland/src/protocols/xdg_foreign.rs:77              | 16           | KEEP    | No  | Handle length is our wire format for this protocol [INFERENCE] | Keep                                 | low  | S      |
| `gnoblin_session_host_v1::REQUEST_DEADLINE_TURNS` | crates/gnoblin-wayland/src/protocols/gnoblin_session_host_v1.rs:104 | 10 000 turns | KEEP    | No  | Gnoblin internal protocol                                      | Keep; document the unit ("turns")    | low  | S      |
| `gnoblin_session_host_v1::MAX_HINT_NAME`          | crates/gnoblin-wayland/src/protocols/gnoblin_session_host_v1.rs:107 | 128 bytes    | KEEP    | No  | Our own protocol bound                                         | Keep                                 | low  | S      |
| `gnoblin_window_frame_v1::MAX_REGIONS`            | crates/gnoblin-wayland/src/protocols/gnoblin_window_frame_v1.rs:114 | 64           | KEEP    | No  | Our own protocol bound                                         | Keep                                 | low  | S      |
| `gnoblin_window_frame_v1::MAX_COORDINATE`         | crates/gnoblin-wayland/src/protocols/gnoblin_window_frame_v1.rs:117 | 32 768       | KEEP    | No  | Our own protocol bound                                         | Keep                                 | low  | S      |
| `gnoblin_window_frame_v1::FRAME_ROLE`             | crates/gnoblin-wayland/src/protocols/gnoblin_window_frame_v1.rs:111 | string       | KEEP    | No  | Our protocol name                                              | Keep                                 | low  | S      |
| `wp_cursor_shape_v1::CURSOR_CACHE_BYTES`          | crates/gnoblin-wayland/src/protocols/wp_cursor_shape_v1.rs:74       | 8 MiB        | KEEP    | No  | Our own cache size                                             | Could be sized by the renderer; keep | low  | S      |

`LEADER: u32 = 0` at `xdg_activation_v1.rs:174` is a bare number with no documented meaning. REMOVE the constant, or give it a named variant, once its role is confirmed.

### 2.6 Wayland misc (gnoblin-wayland)

| Constant                                 | Location                                                         | Value                  | Verdict   | Q1                                                                               | Q2                                                                                                                     | Q3                                                            | Risk | Effort |
| ---------------------------------------- | ---------------------------------------------------------------- | ---------------------- | --------- | -------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- | ---- | ------ |
| `wp_presentation_time::CLOCK_MONOTONIC`  | crates/gnoblin-wayland/src/protocols/wp_presentation_time.rs:58  | 1                      | DERIVE    | No                                                                               | Use `libc::CLOCK_MONOTONIC`; the presentation-time protocol says the clock is the one from `clock_gettime` [INFERENCE] | Remove the literal; use the libc value                        | low  | S      |
| `wp_presentation_time` 1e12 femtoseconds | crates/gnoblin-wayland/src/protocols/wp_presentation_time.rs:131 | 10^12                  | KEEP      | No                                                                               | Unit conversion: mHz to fs (refresh) [INFERENCE]                                                                       | Keep; name it `FEMTOSECONDS_PER_SECOND_MILLIHERTZ` or similar | low  | S      |
| `server::DEFAULT_SOCKET_NAME`            | crates/gnoblin-wayland/src/server.rs:73                          | `gnoblin-wayland.sock` | CONFIGURE | Maybe, for multiple sessions                                                     | Wayland socket naming is by convention                                                                                 | Could be a leaf; low value                                    | low  | S      |
| `state::DEFAULT_WORKSPACES`              | crates/gnoblin-wayland/src/state.rs:97                           | 1                      | KEEP      | Already configurable: the `workspaces` option in `ServerOptions` (server.rs:~76) | Config-derived                                                                                                         | Keep as the default                                           | low  | S      |
| `effects::BLUR_RADIUS`                   | crates/gnoblin-wayland/src/effects.rs:23                         | 12.0                   | CONFIGURE | Yes; blur strength is a visual taste                                             | None                                                                                                                   | Keep as default                                               | low  | S      |
| `frame::START` OnceLock                  | crates/gnoblin-wayland/src/frame.rs:87                           | `Instant`              | KEEP      | No                                                                               | Process start for timing                                                                                               | Keep                                                          | low  | S      |
| `keyboard::SEAT_KEYCODE_OFFSET`          | crates/gnoblin-wayland/src/keyboard.rs:81                        | 8                      | KEEP      | No                                                                               | evdev to xkb keycode offset, a fixed protocol rule                                                                     | Keep; cite xkb                                                | low  | S      |
| `access::ALL`                            | crates/gnoblin-wayland/src/access.rs:455                         | list                   | KEEP      | No                                                                               | Our list of globals                                                                                                    | Keep                                                          | low  | S      |

`BLUR_RADIUS` is a plain default for a visual effect. Proposed leaf: `blur.radius`, float, px, default 12.0, range 0 to 64 [INFERENCE: range is a guess and needs a check against the renderer's limits]. The config already has a `blur` section (`config.toml` near line 1475), so this belongs in core config.

`DEFAULT_WORKSPACES` is already configurable through `ServerOptions`. The literal is only the default.

### 2.7 Input chord and naming limits (gnoblin-input)

| Constant                                        | Location                                 | Value              | Verdict | Q1  | Q2                                                                         | Q3                                                          | Risk | Effort |
| ----------------------------------------------- | ---------------------------------------- | ------------------ | ------- | --- | -------------------------------------------------------------------------- | ----------------------------------------------------------- | ---- | ------ |
| `chord::MAX_CHORD_LENGTH`                       | crates/gnoblin-input/src/chord.rs:72     | 128 bytes          | KEEP    | No  | Our parser bound                                                           | Keep                                                        | low  | S      |
| `chord::MAX_KEY_NAME_LENGTH`                    | crates/gnoblin-input/src/chord.rs:75     | 64 bytes           | KEEP    | No  | Comment says it matches the C `key[64]` field, so it is a legacy ABI bound | Keep; the comment is the evidence                           | low  | S      |
| `chord::CHORD_EXAMPLE`, `POINTER_CHORD_EXAMPLE` | crates/gnoblin-input/src/chord.rs:78, 81 | strings            | KEEP    | No  | Error message text                                                         | Keep                                                        | low  | S      |
| `chord::WRITTEN_ORDER`                          | crates/gnoblin-input/src/chord.rs:116    | 6 modifiers        | KEEP    | No  | Canonical modifier order for output                                        | Keep                                                        | low  | S      |
| `chord::PLACEHOLDER_KEYSYMS`                    | crates/gnoblin-input/src/chord.rs:709    | unit struct static | REMOVE  | No  | Used only to format suggestions                                            | Use the unit struct value directly; the static adds nothing | low  | S      |
| `name::MAX_NAME_LENGTH`                         | crates/gnoblin-input/src/name.rs:12      | 64                 | KEEP    | No  | Our bound                                                                  | Keep                                                        | low  | S      |
| `policy::MAX_PENDING`                           | crates/gnoblin-input/src/policy.rs:36    | 256                | KEEP    | No  | Our queue bound                                                            | Keep                                                        | low  | S      |

`PLACEHOLDER_KEYSYMS` is a production helper, not a test placeholder. It is correct, but the static is unnecessary, so REMOVE the static and inline the value.

### 2.8 Gesture thresholds (gnoblin-input)

These are the most user-visible numbers in the input crate. Some are already configured.

| Constant                                                                                       | Location                                            | Value                     | Verdict   | Q1                                        | Q2                                   | Q3                                                            | Risk   | Effort |
| ---------------------------------------------------------------------------------------------- | --------------------------------------------------- | ------------------------- | --------- | ----------------------------------------- | ------------------------------------ | ------------------------------------------------------------- | ------ | ------ |
| `gesture_router::DEFAULT_SWIPE_THRESHOLD`                                                      | crates/gnoblin-input/src/gesture_router.rs:258      | 48.0                      | CONFIGURE | Yes; swipe distance is a common complaint | None                                 | Keep default; valid range 16 to 240 (`gesture_router.rs:343`) | medium | M      |
| `gesture_router::DEFAULT_PINCH_THRESHOLD`                                                      | crates/gnoblin-input/src/gesture_router.rs:261      | 0.12                      | CONFIGURE | Yes                                       | None                                 | Keep default                                                  | medium | M      |
| `gesture_router::DEFAULT_TOLERANCE`                                                            | crates/gnoblin-input/src/gesture_router.rs:255      | 0.22                      | CONFIGURE | Maybe                                     | None                                 | Keep default, range 0.05 to 0.5                               | medium | M      |
| `gesture_router::{MIN,MAX}_TOLERANCE`                                                          | crates/gnoblin-input/src/gesture_router.rs:249, 252 | 0.05, 0.5                 | KEEP      | No                                        | Validation bounds for the default    | Keep as validation                                            | low    | S      |
| `gesture_router::{MIN,MAX}_PATH_POINTS`                                                        | crates/gnoblin-input/src/gesture_router.rs:243, 246 | 2, 16                     | KEEP      | No                                        | Validation bounds                    | Keep                                                          | low    | S      |
| `gesture::GestureConfig::default` swipe 48.0, pinch 0.12                                       | crates/gnoblin-input/src/gesture.rs:352-353         | duplicate                 | REMOVE    | No                                        | Duplicates `gesture_router` defaults | One source: use the `DEFAULT_*` constants                     | low    | S      |
| `gesture::{MIN,MAX}_FINGERS`, `PATH_SAMPLES`, `MAX_PATH_POINTS`, `PATH_STEP`, `SPREAD_EPSILON` | crates/gnoblin-input/src/gesture.rs:60-76           | 2, 5, 16, 128, 1.0, 0.001 | KEEP      | No                                        | Gesture geometry and numeric guards  | Keep; finger counts are hardware-driven                       | low    | S      |

Finding: `GestureConfig::default` (`gesture.rs:352-353`) repeats the `DEFAULT_SWIPE_THRESHOLD` (48) and `DEFAULT_PINCH_THRESHOLD` (0.12) values. There are two sources of truth. Verdict: REMOVE the duplicate and use the constants. Risk: low.

Proposed config leaves for the three defaults: the touchpad record already holds gesture bindings (`config.toml` line 829 and the `gesture` setting at line 265). Proposed leaves are `input.touchpad.gesture-swipe-threshold` (float, default 48, range 16 to 240), `input.touchpad.gesture-pinch-threshold` (float, default 0.12, fraction of the starting spread, range 0.02 to 0.5 [INFERENCE: range chosen for usability, not checked against spec]) and `input.touchpad.gesture-tolerance` (float, default 0.22, range 0.05 to 0.5). These belong in core config, because the Lua gesture bindings read them. [INFERENCE: exact leaf names need Kieran's choice, see section 5.]

`GestureConfig` also has `stale_ticks: 1000` (`gesture.rs:358`), a literal with no constant. Verdict: CONFIGURE only if users report stuck gestures; otherwise KEEP. Risk: low.

### 2.9 Gesture and policy numeric helpers (gnoblin-input)

| Constant                       | Location                                   | Value      | Verdict | Q1  | Q2                                      | Q3            | Risk | Effort |
| ------------------------------ | ------------------------------------------ | ---------- | ------- | --- | --------------------------------------- | ------------- | ---- | ------ |
| `policy` `capacity.max(1)`     | crates/gnoblin-input/src/policy.rs:146     | 1          | KEEP    | No  | Guard against zero capacity             | Keep          | low  | S      |
| `router` test keysyms `Q`, `W` | crates/gnoblin-input/src/router.rs:571-572 | 0x71, 0x77 | KEEP    | No  | Test fixtures in `mod tests` (line 564) | Keep in tests | low  | S      |

### 2.10 Keyboard accessibility timings (gnoblin-a11y)

Most of these are already configurable. The a11y record names are from `config.toml`.

| Constant                          | Location                                 | Value  | Verdict | Q1                                                                       | Q2                                              | Q3                                                          | Risk | Effort |
| --------------------------------- | ---------------------------------------- | ------ | ------- | ------------------------------------------------------------------------ | ----------------------------------------------- | ----------------------------------------------------------- | ---- | ------ |
| slow keys default `delay_ms`      | crates/gnoblin-a11y/src/config.rs:60     | 300 ms | DERIVE  | Already configurable: `SlowKeys.delay-ms` (config.toml:609)              | Comment says the GNOME schema default is 300 ms | Keep as fallback                                            | low  | S      |
| bounce keys default `delay_ms`    | crates/gnoblin-a11y/src/config.rs:84     | 300 ms | DERIVE  | Already configurable: `BounceKeys` leaf, config.toml:525                 | Comment says GNOME schema default               | Keep as fallback                                            | low  | S      |
| mouse keys `max_speed`            | crates/gnoblin-a11y/src/config.rs:109    | 10     | KEEP    | Already configurable: `MouseKeys.max-speed` (config.toml:567)            | No upstream; GNOME default                      | Keep as default                                             | low  | S      |
| mouse keys `acceleration_time_ms` | crates/gnoblin-a11y/src/config.rs:110    | 300 ms | KEEP    | Already configurable: `MouseKeys.acceleration-time-ms` (config.toml:577) | No upstream                                     | Keep as default                                             | low  | S      |
| mouse keys `initial_delay_ms`     | crates/gnoblin-a11y/src/config.rs:111    | 300 ms | KEEP    | Already configurable: `MouseKeys.initial-delay-ms` (config.toml:587)     | No upstream                                     | Keep as default                                             | low  | S      |
| mouse keys `MOTION_INTERVAL_MS`   | crates/gnoblin-a11y/src/mouse_keys.rs:13 | 100 ms | KEEP    | No                                                                       | Our tick rate for motion                        | Could be derived from `max-speed`; keep as an internal tick | low  | S      |

The slow keys default is 300 ms. The comment cites the GNOME schema, which is the right upstream source. Accessibility leaves already exist in full, so no new config leaf is needed. Only the fallback literal stays in code.

### 2.11 Magnifier and visual (gnoblin-a11y)

| Constant                          | Location                                | Value | Verdict   | Q1                          | Q2                                                                | Q3                                                                            | Risk   | Effort |
| --------------------------------- | --------------------------------------- | ----- | --------- | --------------------------- | ----------------------------------------------------------------- | ----------------------------------------------------------------------------- | ------ | ------ |
| `magnifier::MIN_ZOOM`             | crates/gnoblin-a11y/src/magnifier.rs:13 | 1x    | KEEP      | No                          | Lower bound, a no-op below 1                                      | Keep                                                                          | low    | S      |
| `magnifier::MAX_ZOOM`             | crates/gnoblin-a11y/src/magnifier.rs:16 | 32x   | CONFIGURE | Maybe; a user may want more | Renderer limits [INFERENCE]                                       | Keep; a leaf would need a max                                                 | low    | S      |
| default zoom `Scale::integral(2)` | crates/gnoblin-a11y/src/config.rs:137   | 2x    | CONFIGURE | Yes                         | None; no magnifier leaf exists in `config.toml` (grep found none) | Proposed leaf `accessibility.magnifier.zoom`, float, default 2, range 1 to 32 | medium | M      |

Finding: the magnifier zoom default is the only a11y value a user would plausibly set, and no leaf exists. The existing `Accessibility` record has none for the magnifier. Proposed leaf: `Accessibility.magnifier-zoom`, float, x, default 2, range 1 to 32 (matching `MIN_ZOOM` and `MAX_ZOOM`). Belongs in core config.

### 2.12 Keysyms (gnoblin-a11y)

| Constant                        | Location                                    | Value            | Verdict | Q1  | Q2                                                                                                              | Q3                 | Risk | Effort |
| ------------------------------- | ------------------------------------------- | ---------------- | ------- | --- | --------------------------------------------------------------------------------------------------------------- | ------------------ | ---- | ------ |
| `mouse_keys::KP_*` (26 keysyms) | crates/gnoblin-a11y/src/mouse_keys.rs:16-41 | 0xff95 .. 0xffb9 | DERIVE  | No  | Smithay re-exports xkbcommon keysyms (`smithay 0.7.0`, `src/input/keyboard/mod.rs:18`); use `keysyms::KEY_KP_*` | Remove 26 literals | low  | S      |

The values match X11 keysym numbers. The xkbcommon keysym table is the upstream source, and Smithay already exposes it. Replacing the literals removes 26 numbers that were copied by hand. Evidence for the match: the values in `mouse_keys.rs:16-41` are the standard X11 `KP_*` keysyms [INFERENCE: checked against the keysym list in xkbcommon, not against a diff].

### 2.13 Test-only items (gnoblin-a11y, gnoblin-wayland, gnoblin-input)

| Constant                                                  | Location                                                        | Verdict | Note                             |
| --------------------------------------------------------- | --------------------------------------------------------------- | ------- | -------------------------------- |
| `zwlr_layer_shell_v1::MONITOR`                            | crates/gnoblin-wayland/src/protocols/zwlr_layer_shell_v1.rs:506 | KEEP    | `mod tests` at line 495          |
| `gnoblin-a11y/src/tests.rs` keysyms and `OWNER` (7 items) | crates/gnoblin-a11y/src/tests.rs:16-23                          | KEEP    | Test fixtures; not in production |

### 2.14 Portal dialog (gnoblin-portal-core)

`gnoblin-portal` has no constants. `screen.rs` is logic only.

| Constant                      | Location                                             | Value         | Verdict   | Q1                   | Q2                                                                                      | Q3                                                      | Risk   | Effort |
| ----------------------------- | ---------------------------------------------------- | ------------- | --------- | -------------------- | --------------------------------------------------------------------------------------- | ------------------------------------------------------- | ------ | ------ |
| `ui::PIXELS_PER_POINT`        | crates/gnoblin-portal-core/src/ui.rs:19              | 1.0           | KEEP      | No                   | Headless renderer does not scale                                                        | Keep                                                    | low    | S      |
| `ui::MIN_WIDTH`               | crates/gnoblin-portal-core/src/ui.rs:24              | 360 px        | CONFIGURE | Maybe; small screens | None                                                                                    | Keep as a floor; a leaf is low value                    | low    | S      |
| `model::dialog::ARM_DELAY_MS` | crates/gnoblin-portal-core/src/model/dialog.rs:13    | 500 ms        | KEEP      | No                   | Security control, proposed in D19 (`design/rust-compositor.md:1622`, rule at line 2490) | Keep fixed; configuring it weakens a consent guard      | medium | S      |
| `model::interface::ALL`       | crates/gnoblin-portal-core/src/model/interface.rs:47 | 22 interfaces | KEEP      | No                   | The portal interface list; must match `org.freedesktop.portal.*` names                  | Keep; check against the portal spec when a name changes | low    | S      |

Finding on `ARM_DELAY_MS`: the 500 ms arm delay protects consent dialogs from an accidental press. Kieran may want it tunable, but a user setting of 0 would defeat the protection. The verdict is KEEP, and the choice is listed in section 5.

Finding on `MIN_WIDTH`: a fixed floor of 360 px is a layout choice. The `ui.rs:24` comment says the bounds still win when narrower. Leave it.

## 3. Top ten wins

Ranked by user value over effort.

1. **Read the check-alive timeout from config** (`state.rs:775`): the leaf exists and is already set to 5000; the literal ignores it. Effort S. Risk medium.
2. **Magnifier default zoom as a leaf** (`config.rs:137`): the only a11y value users would set; no leaf exists. Effort M. Risk medium.
3. **Gesture swipe and pinch thresholds as leaves** (`gesture_router.rs:255-261`): common complaint, no leaf. Effort M. Risk medium.
4. **Remove the duplicate gesture defaults** (`gesture.rs:352-353`): two sources of truth for 48 and 0.12. Effort S. Risk low.
5. **Derive protocol manager versions from the XML** (`zwlr_*` and `ext_background_effect_v1` manager constants): removes drift risk on every protocol bump. Effort S each, one generator. Risk low.
6. **Use xkbcommon keysyms for mouse keys** (`mouse_keys.rs:16-41`): removes 26 hand-copied literals. Effort S. Risk low.
7. **Blur radius as a leaf** (`effects.rs:23`): a visual taste value with no config. Effort S. Risk low.
8. **Lock cover colour as a leaf** (`ext_session_lock_v1.rs:99`): a real theme choice. Effort S. Risk medium.
9. **Use `libc::CLOCK_MONOTONIC`** (`wp_presentation_time.rs:58`): removes a bare `1`. Effort S. Risk low.
10. **Remove the placeholder static** (`chord.rs:709`) and the unused `LEADER` constant (`xdg_activation_v1.rs:174`): dead weight. Effort S. Risk low.

Keyboard repeat is already configurable (`input.keyboard.delay` and `repeat-interval`). The only change is to keep the 200 and 40 ms literals as fallbacks, so no wins are listed for it.

## 4. Constants to leave alone and why

- **Protocol bit flags and enum values** (`foreign_toplevel_shared.rs:47-53`): wire format. Changing them breaks clients.
- **Internal timeouts** (`FOCUS_TIMEOUT`, lock wait, `LAYOUT_COMMAND_WAIT`, `LAYOUT_FRAME_BUDGET`): runtime bounds. A user setting would expose a race, not a preference.
- **Protocol caps** (`MAX_REGIONS`, `MAX_COORDINATE`, `MAX_HINT_NAME`, `IDLE_HOLDER_CAP`, `ISSUED_SERIALS`): defensive limits against hostile clients.
- **`SEAT_KEYCODE_OFFSET` = 8** (`keyboard.rs:81`): fixed evdev to xkb offset.
- **`ALL` lists** (`access.rs:455`, `interface.rs:47`): must match the code and the portal spec.
- **Chord and name length caps** (`chord.rs:72-75`, `name.rs:12`): the C ABI bound and the parser limit.
- **`MOTION_INTERVAL_MS`** (`mouse_keys.rs:13`): internal tick.
- **Test fixtures**: not production.

## 5. Needs Kieran

- **Lock cover colour** (`ext_session_lock_v1.rs:99`): should it be a user leaf (`lock.cover-colour`), or stay black?
- **Gesture leaf names and ranges**: confirm `input.touchpad.gesture-swipe-threshold`, `gesture-pinch-threshold` and `gesture-tolerance`, and whether the pinch range should be 0.02 to 0.5.
- **Magnifier zoom leaf**: confirm `Accessibility.magnifier-zoom`, and whether the max should follow `MAX_ZOOM` (32).
- **Consent arm delay** (`dialog.rs:13`): keep fixed for security, or expose with a floor?
- **Keyboard repeat fallback**: confirm that GNOME's `org.gnome.desktop.peripherals.keyboard` keys are the intended source when the leaf is unset. I did not check the schema.
- **Blur radius leaf** (`effects.rs:23`): core config (`blur.radius`), or Bingux-facing?
- **`LEADER` constant** (`xdg_activation_v1.rs:174`): what is its meaning? I could not determine it from the name alone.
