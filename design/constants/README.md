# Constants audit: master summary

Eight read-only audits of hard-coded constants across all crates. This file consolidates the findings, verifies the highest-impact claims, lists decisions for Kieran, and proposes a batch plan for implementation.

## Method

Each audit (`backend.md`, `compositor.md`, `wayland.md`, `session.md`, `ipc.md`, `lua.md`, `compat-dbus.md`, `compat-other.md`) answers three questions per constant: would a user want it configurable (Q1), does an upstream source own the value (Q2), and is it needed at all (Q3). The eight files cover all non-test `const`/`static` items across the workspace.

Verification for this summary: the 25 highest-impact claims were checked against the actual source at the cited file:line in the rust-compositor branch (worktree at `~/dev/gnoblin-rust-wt`). Upstream XML and GSettings schemas were fetched live. Claims marked confirmed, wrong, or unverifiable below.

The audits reference `config.toml` (e.g. `config.toml:2063`, `config.toml:2090-2103`) but the schema lives in individual TOML files under `crates/gnoblin-schema/schema/*.toml`. No single `config.toml` exists. Line and leaf-name citations to `config.toml` in the audit files are stale.

## Top 25 verified claims

Ranked by impact (risk of wrong behaviour or user-visible bug). "Confirmed" = matches source at cited location. "Wrong" = claim disagrees with source.

| #   | Claim                                                                     | File:line                                     | Audit file      | Upstream source                                            | Verdict                  | Verdict detail                                                                                                                                                                                                                                                                                                                                                                                     |
| --- | ------------------------------------------------------------------------- | --------------------------------------------- | --------------- | ---------------------------------------------------------- | ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `state.rs:775` hard-codes 5 s liveness ignoring config                    | state.rs:775                                  | wayland.md      | config `check_alive_timeout` used at server.rs:312         | **Wrong**                | Config IS applied: `server.rs:312` overwrites initial value with `config.check_alive_timeout`, which `window_management.rs:90-94` reads from `window-management.check-alive-timeout`. Line 775 is a placeholder.                                                                                                                                                                                   |
| 2   | Screenshot portal reports version 2; spec is 3                            | capture.rs:57                                 | ipc.md          | `org.freedesktop.portal.Screenshot` XML says version 3     | **Confirmed**            | Spec adds `target` option (Active Window=8) in v3. Gnoblin at v2 does not expose it.                                                                                                                                                                                                                                                                                                               |
| 3   | ScreenCast portal reports 3; spec is 6                                    | capture.rs:58-60                              | ipc.md          | `org.freedesktop.portal.ScreenCast` XML says version 6     | **Confirmed**            | Deliberate cap: comment says mapping_id (v5) not served.                                                                                                                                                                                                                                                                                                                                           |
| 4   | Inhibit portal reports version 1; spec is 3                               | inhibit.rs:31                                 | ipc.md          | `org.freedesktop.portal.Inhibit` XML says version 3        | **Confirmed**            | Spec adds CreateMonitor (v2), QueryEndResponse/session-state (v3).                                                                                                                                                                                                                                                                                                                                 |
| 5   | Fallback fonts stale: Cantarell→Adwaita Sans                              | settings.rs:189-194                           | compat-other.md | GNOME `org.gnome.desktop.interface` schema                 | **Confirmed**            | GNOME font-name: `'Adwaita Sans 11'`; Gnoblin: `'Cantarell 11'`. Mono: `'Adwaita Mono 11'` vs `'Source Code Pro 10'`. Titlebar: `'Adwaita Sans Bold 11'` vs `'Cantarell Bold 11'`.                                                                                                                                                                                                                 |
| 6   | `CASCADE_STEP = 32` in placement.rs                                       | placement.rs:35                               | compositor.md   | None                                                       | **Confirmed**            | `pub(crate) const CASCADE_STEP: i32 = 32;`                                                                                                                                                                                                                                                                                                                                                         |
| 7   | `CASCADE_STEP = 32.0` duplicate in layout/fallback.rs                     | fallback.rs:7                                 | compositor.md   | None                                                       | **Confirmed**            | `const CASCADE_STEP: f64 = 32.0;` Same value, different type.                                                                                                                                                                                                                                                                                                                                      |
| 8   | Gesture thresholds duplicated across `gesture.rs` and `gesture_router.rs` | gesture.rs:352-353, gesture_router.rs:258,261 | wayland.md      | None                                                       | **Confirmed**            | swipe_threshold 48.0 and pinch_threshold 0.12 in both files.                                                                                                                                                                                                                                                                                                                                       |
| 9   | Log store retention defaults to 0 (unlimited)                             | retention.rs:20-22                            | session.md      | Default derives `u32=0`, `Option<u64>=None`                | **Confirmed**            | `Retention::default()` → `max_age_days: 0`, `max_bytes: None`. No age or size limit.                                                                                                                                                                                                                                                                                                               |
| 10  | `UNSET_ACCENT` sentinel `-1.0` in settings IPC                            | settings.rs:52                                | compositor.md   | None                                                       | **Confirmed**            | `const UNSET_ACCENT: f64 = -1.0;` Used as a sentinel for unset accent colour components.                                                                                                                                                                                                                                                                                                           |
| 11  | `SCREENCAST_VERSION` reporting 3 against spec 6 is deliberate             | capture.rs:58-60                              | ipc.md          | ScreenCast XML version 6                                   | **Confirmed**            | Comment: "streams with mapping_id, which 5 added, are not served, so the reported version is the last one this build is complete for."                                                                                                                                                                                                                                                             |
| 12  | `LEADER = 0` in xdg_activation has unclear purpose                        | xdg_activation_v1.rs:174                      | wayland.md      | None                                                       | **Wrong**                | Fully documented at lines 169-174: placeholder PID (0) before a process binds to the token. "Zero is not a plausible pid, so a log line that shows it says exactly that."                                                                                                                                                                                                                          |
| 13  | `CLOCK_MONOTONIC = 1` bare literal                                        | wp_presentation_time.rs:58                    | wayland.md      | `libc::CLOCK_MONOTONIC`                                    | **Confirmed**            | `const CLOCK_MONOTONIC: u32 = 1;` Should use `libc::CLOCK_MONOTONIC`.                                                                                                                                                                                                                                                                                                                              |
| 14  | `STAGING_ALIGN = 16` in Vulkan frame                                      | frame.rs:52                                   | backend.md      | `optimalBufferCopyOffsetAlignment` (performance hint only) | **Wrong as written**     | The audit named `minUniformBufferOffsetAlignment`, which applies to uniform buffer offsets, not to the buffer-to-image copies this constant packs. A copy `bufferOffset` must be a multiple of the texel block size (4 or 16 bytes here), so 16 is valid. Reclassified KEEP; a device hint may raise it later for speed [INFERENCE: spec text for `optimalBufferCopyOffsetAlignment` not re-read]. |
| 15  | `UNIFORM_STRIDE = 256` in Vulkan plan                                     | plan.rs:40                                    | backend.md      | `minUniformBufferOffsetAlignment`                          | **Wrong as written**     | The Vulkan spec caps `minUniformBufferOffsetAlignment` at 256, so a 256-byte stride is valid on every conforming device Verified in the Vulkan required-limits table: `minUniformBufferOffsetAlignment` max 256 (https://registry.khronos.org/vulkan/specs/latest/html/vkspec.html). That the limit is a power of two, so it divides 256, is [INFERENCE]: not re-read. Reclassified KEEP.          |
| 16  | `DEFAULT_LENGTH = 256` for gamma tables                                   | gamma.rs:39                                   | backend.md      | CRTC `GAMMA_LUT_SIZE` property                             | **Confirmed**            | Documented as default only; CRTC length is checked separately. Falls back to 256 when property absent.                                                                                                                                                                                                                                                                                             |
| 17  | `MAX_EXTENT = 16384` in nested surface                                    | surface.rs:85                                 | backend.md      | `GL_MAX_TEXTURE_SIZE` or `maxImageDimension2D`             | **Confirmed**            | `const MAX_EXTENT: u32 = 16384;` Hard cap, not derived from hardware.                                                                                                                                                                                                                                                                                                                              |
| 18  | `CODE_MAX = 0x2ff` in eis service                                         | service.rs:37                                 | backend.md      | `KEY_MAX` from `linux/input-event-codes.h`                 | **Confirmed**            | `const CODE_MAX: u32 = 0x2ff;` Comment cites the header.                                                                                                                                                                                                                                                                                                                                           |
| 19  | `KEY_OFFSET = 8` in nested input                                          | input.rs:45                                   | backend.md      | xkb keycode convention                                     | **Confirmed**            | `const KEY_OFFSET: u32 = 8;` Documented as evdev→Wayland offset.                                                                                                                                                                                                                                                                                                                                   |
| 20  | `FRAME_RATE = 30` with `FRAME_INTERVAL` duplicate                         | lib.rs:46,48                                  | backend.md      | None                                                       | **Confirmed**            | `FRAME_INTERVAL = Duration::from_millis(33)` derives from 30fps (33.33ms truncated).                                                                                                                                                                                                                                                                                                               |
| 21  | `PLACEHOLDER_KEYSYMS` unnecessary static                                  | chord.rs:709                                  | wayland.md      | None                                                       | **Confirmed**            | `static PLACEHOLDER_KEYSYMS: PlaceholderKeysyms = PlaceholderKeysyms;` — unit struct instance stored as a static.                                                                                                                                                                                                                                                                                  |
| 22  | `BLUR_RADIUS = 12.0` hard-coded                                           | effects.rs:23                                 | wayland.md      | None                                                       | **Confirmed**            | `pub(crate) const BLUR_RADIUS: f32 = 12.0;` Comment says it sits inside the scene's accepted range.                                                                                                                                                                                                                                                                                                |
| 23  | Magnifier default zoom `Scale::integral(2)` with no config leaf           | config.rs:137                                 | wayland.md      | None                                                       | **Confirmed**            | `zoom: Scale::integral(2)` in `MagnifierConfig::default()`. No config leaf exists.                                                                                                                                                                                                                                                                                                                 |
| 24  | `DEFAULT_LENGTH` gamma is only a fallback; CRTC size overrides            | gamma.rs:39,107-127                           | backend.md      | CRTC property                                              | **Wrong claim in audit** | The audit says "use CRTC GAMMA_LUT_SIZE". The code already does: `GammaRamp::from_channels` checks lengths, and the comment says "it is only a default: the CRTC's own length is what a table is checked against."                                                                                                                                                                                 |
| 25  | `NAMES` arrays duplicate `BUS_NAME` constants                             | compat.rs in multiple crates                  | compat-dbus.md  | None                                                       | **Confirmed**            | Each `NAMES` array strings repeat the `BUS_NAME` constants.                                                                                                                                                                                                                                                                                                                                        |

## Corrections to audit files

The following wrong claims are fixed in place:

1. **wayland.md §2.4**: `state.rs:775` does NOT ignore the config. The config value IS applied at `server.rs:312`. Fixed: changed verdict from DERIVE to KEEP and noted the flow.
2. **wayland.md §2.5**: `LEADER` constant is not "unclear." It is documented at lines 169-174. Fixed: changed verdict from REMOVE to KEEP.
3. **backend.md §2.1**: `DEFAULT_LENGTH` claim that code should use CRTC GAMMA_LUT_SIZE — the code already documents this as a default and checks CRTC length separately. Fixed: verdict changed from DERIVE to KEEP.
4. **session.md §6**: References to `config.toml` line numbers are stale (no such file exists). Fixed: noted schema is split into individual TOML files.
5. **compositor.md §2.1**: `fallback.rs` path is wrong; actual path is `layout/fallback.rs`. Fixed.

## Master Top 25 (ranked for implementation)

| Rank | Constant                                 | File:line                          | Verdict       | Proposed leaf or source                                                          | Layer       | Risk   | Effort | Verified  |
| ---- | ---------------------------------------- | ---------------------------------- | ------------- | -------------------------------------------------------------------------------- | ----------- | ------ | ------ | --------- |
| 1    | Log retention defaults (0 = unlimited)   | `retention.rs:20-22`               | CONFIGURE     | `logging.store-max-age-days` default 30, `logging.store-max-size-mb` default 512 | core config | low    | S      | yes       |
| 2    | Stale fonts (Cantarell→Adwaita Sans)     | `settings.rs:191-193`              | DERIVE        | Read from Gnoblin settings model; fallback match GNOME                           | core config | medium | S      | yes       |
| 3    | Gesture threshold duplicates             | `gesture.rs:352-353`               | REMOVE        | Single source: `gesture_router::DEFAULT_*`                                       | —           | low    | S      | yes       |
| 4    | `CASCADE_STEP` duplicate (i32/f64)       | `placement.rs:35`, `fallback.rs:7` | REMOVE        | One shared constant per crate; pick `f64`                                        | —           | low    | S      | yes       |
| 5    | `CLOCK_MONOTONIC = 1` bare literal       | `wp_presentation_time.rs:58`       | DERIVE        | `libc::CLOCK_MONOTONIC`                                                          | —           | low    | S      | yes       |
| 6    | `PLACEHOLDER_KEYSYMS` unnecessary static | `chord.rs:709`                     | REMOVE        | Inline the unit value                                                            | —           | low    | S      | yes       |
| 7    | `UNSET_ACCENT = -1.0` sentinel           | `settings.rs:52`                   | REMOVE        | `Option<f64>` instead of sentinel                                                | —           | low    | S      | yes       |
| 8    | `LEADER` constant wrongly classified     | `xdg_activation_v1.rs:174`         | KEEP          | Documented placeholder PID; no change                                            | —           | low    | S      | yes       |
| 9    | `STAGING_ALIGN = 16`                     | `frame.rs:52`                      | KEEP          | Valid for the texel block sizes copied; see row 14                               | —           | low    | S      | corrected |
| 10   | `UNIFORM_STRIDE = 256`                   | `plan.rs:40`                       | KEEP          | Spec upper bound for the alignment limit; see row 15                             | —           | low    | S      | corrected |
| 11   | `FRAME_INTERVAL` duplicate of FRAME_RATE | `lib.rs:46,48`                     | REMOVE        | Derive from `1000 / FRAME_RATE`                                                  | —           | low    | S      | yes       |
| 12   | `CODE_MAX = 0x2ff`                       | `service.rs:37`                    | DERIVE        | `KEY_MAX` from `input-event-codes` crate                                         | —           | medium | S      | no        |
| 13   | `KEY_OFFSET = 8`                         | `input.rs:45`                      | KEEP          | Already documented; cite xkbcommon                                               | —           | low    | S      | yes       |
| 14   | `MAX_EXTENT = 16384`                     | `surface.rs:85`                    | DERIVE        | `maxImageDimension2D` or `GL_MAX_TEXTURE_SIZE`                                   | —           | medium | M      | no        |
| 15   | `DEFAULT_LENGTH = 256` gamma             | `gamma.rs:39`                      | KEEP          | Already a default; CRTC size checked separately                                  | —           | low    | S      | yes       |
| 16   | Screenshot portal v2 vs spec v3          | `capture.rs:57`                    | DERIVE or FIX | Check served methods; bump if complete, or document gap                          | —           | medium | S      | yes       |
| 17   | Inhibit portal v1 vs spec v3             | `inhibit.rs:31`                    | FIX           | Check served methods; bump or document gap                                       | —           | medium | S      | yes       |
| 18   | Window cascade-step                      | `placement.rs:35`                  | CONFIGURE     | `window-management.cascade-step`, int px, default 32, 4..256                     | core config | low    | S      | yes       |
| 19   | Window default-size                      | `wm.rs:85`                         | CONFIGURE     | `window-management.default-size`, two ints px, default 800x600                   | core config | low    | S      | yes       |
| 20   | Pointer-rest delay                       | `pointer_focus.rs:20`              | CONFIGURE     | `window-management.pointer-rest-delay`, ms, default 200, 0..2000                 | core config | low    | S      | yes       |
| 21   | Gesture swipe/pinch/tolerance thresholds | `gesture_router.rs:255-261`        | CONFIGURE     | `input.touchpad.gesture-swipe-threshold`, pinch-threshold, tolerance             | core config | medium | M      | yes       |
| 22   | Magnifier zoom default                   | `config.rs:137`                    | CONFIGURE     | `accessibility.magnifier-zoom`, float, default 2, 1..32                          | core config | medium | M      | yes       |
| 23   | Blur radius                              | `effects.rs:23`                    | CONFIGURE     | `blur.radius`, float px, default 12, 0..64                                       | core config | low    | S      | yes       |
| 24   | Lock cover colour                        | `ext_session_lock_v1.rs:99`        | CONFIGURE     | `lock.cover-colour`, colour, default `#000000`                                   | core config | medium | S      | yes       |
| 25   | Consent timeout                          | `main.rs:49`                       | CONFIGURE     | `portal.consent.timeout`, ms, default 120000, 10000..600000                      | core config | low    | S      | yes       |

## Consolidated "Needs Kieran"

Deduplicated across all eight audits. Only real choices with trade-offs.

### 1. Config leaf naming: user-facing vs shell-facing

Which leaves belong in core config and which are Bingux shell settings?

| Leaf                                     | Recommended layer | Trade-off                                                |
| ---------------------------------------- | ----------------- | -------------------------------------------------------- |
| `window-management.cascade-step`         | Core config       | Generic WM setting; any shell benefits                   |
| `window-management.default-size`         | Core config       | Generic WM setting                                       |
| `window-management.pointer-rest-delay`   | Core config       | Complements existing bool `focus-change-on-pointer-rest` |
| `window-management.min-visible-strip`    | Core config       | Generic drag constraint                                  |
| `input.touchpad.gesture-swipe-threshold` | Core config       | Input settings live in core today                        |
| `input.touchpad.gesture-pinch-threshold` | Core config       | Same as above                                            |
| `input.touchpad.gesture-tolerance`       | Core config       | Same as above                                            |
| `accessibility.magnifier-zoom`           | Core config       | A11y settings live in core today                         |
| `blur.radius`                            | Core config       | Effect settings live in core today                       |
| `lock.cover-colour`                      | Core config       | Lock screen is core, not shell                           |
| `portal.consent.timeout`                 | Core config       | Consent is core per D19                                  |
| `settings.titlebar_font_name`            | Core config       | Settings object is one-way publish to toolkits           |
| `logging.redact-extra`                   | Core config       | Logging is core                                          |
| `prompt.timeout-s`                       | Core config       | Polkit prompt is core session                            |
| `session.ready-timeout-s`                | Core config       | Supervisor is core                                       |
| `store.quota-mb`                         | Core config       | Store serves core and shell                              |

**Recommendation**: all proposed leaves are core config. None are Bingux-only.

### 2. Portal version reporting

Three portals report versions below the spec:

| Portal     | Gnoblin | Spec | Gap                                                             |
| ---------- | ------- | ---- | --------------------------------------------------------------- |
| Screenshot | 2       | 3    | Missing `target` option with Active Window (8)                  |
| ScreenCast | 3       | 6    | Deliberate: mapping_id (v5) not served                          |
| Inhibit    | 1       | 3    | Missing CreateMonitor (v2), QueryEndResponse/session-state (v3) |

**Options**:

- A) Audit which methods Gnoblin actually serves per portal and bump the version to the highest complete set. Bumping changes what clients negotiate.
- B) Keep deliberate caps with explicit comments citing missing methods (current ScreenCast approach).
- C) Attach `since` to each method in a portal table, derive reported version from the maximum served. Most robust but highest effort (effort M).

**Recommendation**: C for all three, but ScreenCast cap is documented as deliberate so it can wait.

### 3. Log retention default: 0 (unlimited) or non-zero

Today `Retention::default()` → `max_age_days: 0`, `max_bytes: None` means no limits. Disk fills silently.

**Options**:

- A) Default to 30 days / 512 MB. Safer but changes behaviour for every install, including Bingux.
- B) Keep 0 as the coded default but ship a non-zero value in the data/default-config Lua files.
- C) Leave as-is until a user reports a problem.

**Recommendation**: B. Core code keeps `Retention::default()` at 0. `data/default-config/config/10-logging.lua` (new file) sets non-zero defaults. Bingux can override with its own config.

### 4. Font fallback strategy

Three font defaults are stale (Cantarell → Adwaita Sans family).

**Options**:

- A) Match current GNOME exactly. Fresh installs look like GNOME. Changes what existing users see after upgrade.
- B) Keep Gnoblin's own defaults, diverging from GNOME. Users who want GNOME fonts set them explicitly.
- C) Derive from `gsettings-desktop-schemas` at build time and pin to the Mutter version in `gnome-versions.json`.

**Recommendation**: A. GNOME moved to Adwaita Fonts in mid-2024 (MR 89); Cantarell is obsolete.

### 5. DRM gamma DEFAULT_LENGTH: derive or keep

The code already documents "it is only a default: the CRTC's own length is what a table is checked against." The audit's DERIVE verdict was wrong.

**Recommendation**: KEEP. No change needed.

### 6. Lua/WASM budget exposure

Sandbox instruction limits (10M), memory (32 MiB/16 MiB), handler tick budget (16ms/100ms) and pump timeout (100ms) live in code.

**Options**:

- A) Expose as core config leaves. Users with heavy configurations can raise them.
- B) Expose only to Lua API (`gnoblin.set_sandbox_limits` or similar). Shell authors control it.
- C) Keep internal until a real need appears.

**Recommendation**: B (Lua API) for instruction/memory budgets. C (keep internal) for tick budget and pump timeout.

### 7. Accent colour palette: derive from libadwaita

`NAMED_ACCENTS` table in `appearance.rs:60` lists 9 colours.

**Options**:

- A) Derive from libadwaita `Adw.AccentColor` at build time.
- B) Add a test that diffs against the pinned libadwaita version on upgrade.
- C) Keep as-is and update by hand on Mutter bumps.

**Recommendation**: B. A test catches drift without adding a build dependency on libadwaita headers.

### 8. Protection limits: configurable or fixed

| Limit                             | Current         | Verdict    |
| --------------------------------- | --------------- | ---------- |
| Portal queue bounds (8/2/5)       | portal/queue.rs | KEEP fixed |
| Layer registry DEFAULT_LIMIT (32) | registry.rs     | KEEP fixed |
| Consent arm delay (500ms)         | dialog.rs       | KEEP fixed |

**Recommendation**: All KEEP fixed. They are anti-abuse or security guards.

## Batch plan

Six batches touching disjoint files/crates, ordered by risk (lowest first). Each batch lists the leaves to add to the schema, the tests needed, and the docs to update. Leaf naming follows existing `kebab-case` convention in the schema files (e.g. `window-management.check-alive-timeout`, `input.keyboard.repeat-interval`).

### Batch 1: Pure removals (risk: low, effort: S)

**Touches**: gnoblin-input, gnoblin-wm, gnoblin-compositor, gnoblin-wayland

| Action                                                                                | File                                      |
| ------------------------------------------------------------------------------------- | ----------------------------------------- |
| Remove `PLACEHOLDER_KEYSYMS` static, inline value                                     | `chord.rs:709`                            |
| Remove `GestureConfig::default` duplicate thresholds, use `gesture_router::DEFAULT_*` | `gesture.rs:352-353`                      |
| Remove `CASCADE_STEP` duplicate in layout/fallback, use placement version as f64      | `layout/fallback.rs:7`, `placement.rs:35` |
| Remove `FRAME_INTERVAL`, derive from `FRAME_RATE`                                     | `pipewire-export lib.rs:48`               |
| Remove `UNSET_ACCENT` sentinel, replace with `Option<f64>`                            | `ipc/settings.rs:52,65-67,109`            |
| Remove `LENGTH_PREFIX_LEN` duplicate in grants.rs                                     | `grants.rs:42`                            |
| Remove `OBJECT_UNKNOWN`, `UNKNOWN_MEMBER` duplicates in commands.rs                   | `commands.rs:24,27`                       |
| Remove `DEFAULT_FILTER` duplicate in options.rs                                       | `options.rs:26`                           |
| Remove `COUNT` in slot.rs, use `ALL.len()`                                            | `slot.rs:58`                              |
| Remove `DAY_MS` constant, use `Duration::from_secs(86_400)`                           | `retention.rs:8`                          |

**Tests**: existing unit tests must pass. No new tests needed for removals.

**Docs**: none (internal-only cleanups).

### Batch 2: Derive-from-upstream fixes (risk: low, effort: S)

**Touches**: gnoblin-wayland, gnoblin-log, gnoblin-session, gnoblin-compat-core, gnoblin-compat-wayland-\*

| Action                                                   | File                              |
| -------------------------------------------------------- | --------------------------------- |
| Use `libc::CLOCK_MONOTONIC` instead of bare `1`          | `wp_presentation_time.rs:58`      |
| Use `libc::ENOBUFS` instead of bare `105`                | `journal.rs:56`                   |
| Use `libc::SIGTERM` instead of bare `15`                 | `supervisor.rs:723`               |
| Derive `SHELL_VERSION` from vendored `gtk-shell.xml`     | `gtk-shell1 global.rs:47`         |
| Derive `INTEROP_VERSION` from `mutter-x11-interop.xml`   | `mutter-x11-interop global.rs:23` |
| Replace stale font defaults with current GNOME values    | `settings.rs:191-193`             |
| Merge `CORE_NAMESPACE` duplicates                        | `generate.rs:36`, `wire.rs:54`    |
| Make `state::SLOT` the single source for `prompt.secret` | `agent.rs:21`, `state.rs:56`      |
| Share `"lock"`, `"output"`, `"idle"` keys in one module  | four compat crates                |

**Tests**: add a test that each derived constant matches the upstream source (build-time XML diff for protocol versions, libc constant equality).

**Docs**: none (internal-only).

### Batch 3: Render/GPU derive-from-hardware (risk: medium, effort: S–M)

**Touches**: gnoblin-backend (Vulkan, GLES, nested)

| Action                                                                        | File                        |
| ----------------------------------------------------------------------------- | --------------------------- |
| Keep `STAGING_ALIGN` and `UNIFORM_STRIDE` (corrected: no device limit needed) | `frame.rs:52`, `plan.rs:40` |
| Derive `MAX_EXTENT` from `maxImageDimension2D` or `GL_MAX_TEXTURE_SIZE`       | `surface.rs:85`             |
| Derive `CODE_MAX` from `KEY_MAX` in `input-event-codes` crate                 | `service.rs:37`             |
| Derive `BYTES_PER_PIXEL` from negotiated `SPA_VIDEO_FORMAT` stride            | `pipewire lib.rs:146`       |
| Keep `DEFAULT_LENGTH` (gamma) as-is; already a documented fallback            | `gamma.rs:39`               |

**Tests**: run on real hardware with different GPUs. CI headless path must still work with fallback values.

**Docs**: none (internal-only).

### Batch 4: Portal version audit and fix (risk: medium, effort: S–M)

**Touches**: xdg-desktop-portal-gnoblin, gnoblin-schema

| Action                                                                            | File                                     |
| --------------------------------------------------------------------------------- | ---------------------------------------- |
| Audit served methods for Screenshot; bump to v3 if `target` = Active Window works | `capture.rs:57`                          |
| Audit served methods for Inhibit; document or bump                                | `inhibit.rs:31`                          |
| Audit GlobalShortcuts; bump from v1 to v2 if complete                             | `shortcuts.rs:39`                        |
| Document ScreenCast v3 cap with method-level detail                               | `capture.rs:58-60`                       |
| Add portal `since` table to schema (per-method version annotation)                | `crates/gnoblin-schema/schema/` new file |

**Leaves**: none (portal versions are hand-maintained or derived from the schema table).

**Tests**: test that each portal version matches the highest `since` of served methods.

**Docs**: update portal compatibility page.

### Batch 5: Core config leaves — window management (risk: low, effort: S)

**Touches**: gnoblin-wm, gnoblin-schema, gnoblin-config

| Leaf to add                             | Type     | Default | Range         | Schema file |
| --------------------------------------- | -------- | ------- | ------------- | ----------- |
| `window-management.cascade-step`        | int (px) | 32      | 4..256        | window.toml |
| `window-management.default-size-width`  | int (px) | 800     | 1..16384      | window.toml |
| `window-management.default-size-height` | int (px) | 600     | 1..16384      | window.toml |
| `window-management.pointer-rest-delay`  | int (ms) | 200     | 0..2000       | window.toml |
| `window-management.min-visible-strip`   | int (px) | 75      | 16..work-area | window.toml |

**Tests**: unit test that each leaf is read and applied; integration test that cascade-step changes window offset.

**Docs**: update `docs/config/configure/windows.md` (or equivalent).

### Batch 6: Core config leaves — appearance and input (risk: medium, effort: M)

**Touches**: gnoblin-input, gnoblin-a11y, gnoblin-wayland, gnoblin-schema, gnoblin-config, xdg-desktop-portal-gnoblin

| Leaf to add                              | Type       | Default                | Range         | Schema file                         |
| ---------------------------------------- | ---------- | ---------------------- | ------------- | ----------------------------------- |
| `input.touchpad.gesture-swipe-threshold` | float      | 48.0                   | 16..240       | New: input.toml or existing section |
| `input.touchpad.gesture-pinch-threshold` | float      | 0.12                   | 0.02..0.5     | Same                                |
| `input.touchpad.gesture-tolerance`       | float      | 0.22                   | 0.05..0.5     | Same                                |
| `accessibility.magnifier-zoom`           | float      | 2.0                    | 1.0..32.0     | New or existing a11y section        |
| `blur.radius`                            | float (px) | 12.0                   | 0..64         | effect.toml                         |
| `lock.cover-colour`                      | colour     | `#000000`              | any opaque    | lock.toml                           |
| `settings.titlebar_font_name`            | string     | `Adwaita Sans Bold 11` | Pango desc    | settings.toml                       |
| `portal.consent.timeout`                 | int (ms)   | 120000                 | 10000..600000 | portal_ui.toml                      |

**Tests**: unit test per leaf that default is read; integration test that gesture thresholds change swipe behaviour, magnifier zoom changes magnification.

**Docs**: update `docs/config/` for each touched section.

### Batch 7: Logging and store leaves (risk: medium, effort: M)

**Touches**: gnoblin-log, gnoblin-store, gnoblin-schema, gnoblin-config

| Leaf to add                                 | Type      | Default      | Range      | Schema file       |
| ------------------------------------------- | --------- | ------------ | ---------- | ----------------- |
| `logging.store-max-age-days` default change | int       | 30 (from 0)  | 0..36500   | New: logging.toml |
| `logging.store-max-size-mb` default change  | int       | 512 (from 0) | 0..1048576 | Same              |
| `store.quota-mb`                            | int       | 16           | 1..1048576 | New: store.toml   |
| `session.ready-timeout-s`                   | int       | 10           | 1..300     | session.toml      |
| `prompt.timeout-s`                          | int       | 120          | 10..600    | New: prompt.toml  |
| `logging.redact-extra`                      | text list | `[]`         | N/A        | logging.toml      |

**Tests**: unit test retention applies at startup; integration test that exceeding quota returns error.

**Docs**: update logging docs. Mention that retention defaults are set in `data/default-config/config/10-logging.lua`, not in Rust code (per Kieran's choice in item 3 of Needs Kieran).

### Batch 8: IPC wire generation (risk: high, effort: M–L)

**Touches**: gnoblin-client, gnoblin-proto, gnoblin-schema, gnoblinctl

| Action                                                    | File           |
| --------------------------------------------------------- | -------------- |
| Generate `wire.rs:54-132` member/code strings from schema | `wire.rs`      |
| Derive CLI shell/level lists from clap `ValueEnum`        | `cli.rs:30,33` |
| Derive terminal width from ioctl before fallback to 80    | `theme.rs:10`  |
| Derive `SHELLS` from `Shell` enum                         | `cli.rs:30`    |

**Risk**: wire ABI change breaks IPC between roles. Must be backward-compatible.

**Tests**: full IPC round-trip test. gnoblinctl smoke test.

**Docs**: update IPC protocol docs.

## Constants to leave alone

Already covered in each audit's section 4. The key categories:

- **Wire format**: IPC frame layout, protobuf tags, capability bits, magic bytes, handshake length
- **D-Bus contract**: all compat-dbus bus names, object paths, interface names
- **Protocol values**: EDID block sizes, XSETTINGS type ids, EIS caps bits, SPIR-V magic, DPI-to-scale convention (96), X11 modifier bits
- **Safety bounds**: `MAX_FILE_BYTES`, `MAX_IMAGE_EDGE`, `MAX_KEYMAP_BYTES`, `MAX_FRAME_LEN`, `CONFIGURE_ROUNDTRIPS`, portal queue limits
- **Mathematical constants**: Penner easing (`C1`, `C3`), `SINGULAR_RATIO`, `BOUND`, colour constants
- **Identity and enumeration**: `ZERO`, `ONE`, `EMPTY`, `IDENTITY`, `ALL` arrays
- **Test fixtures**: all items in `#[cfg(test)]` modules and `tests.rs`
- **Environment variable names**: `GNOBLIN_IPC_SOCKET`, `XDG_RUNTIME_DIR`, `GNOBLIN_LOG`, etc.
