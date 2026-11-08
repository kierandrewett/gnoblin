# Multi-seat and device-scoped bindings

Status: **proposal, 2026-11-08.** Not decided. Kieran confirms the open points in
section 5 and the decision row D31 in `design/rust-compositor.md`.

Line numbers were read at commit `f64e1afb9`. The Smithay claims were read at rev
`65aa86f` in the cargo git checkout.

Labels used below: **[FACT]** is read from the code or a source file named in the
text. **[INFERENCE]** is a reasoned guess not yet checked. **[UNVERIFIED]** means the
claim needs a test or a source read before anyone relies on it. **[JUDGEMENT]** is the
author's recommendation.

## Summary

Kieran wants two things:

1. Several people at once, each with their own keyboard, pointer, keyboard focus and
   visible cursor, on one compositor.
2. Bindings that match one device, so a second keyboard (a macro pad) can drive
   shortcuts without typing into windows.

Today Gnoblin has one `wl_seat`, one keyboard, one pointer, one cursor, one focused
window and one lock. Input devices lose their identity before they reach the seat,
so no binding can name a device. Smithay already supports several seats. The gap is
in Gnoblin's own state and in the input path between libinput and the seat.

**[JUDGEMENT]** Build in two steps. Phase 1 adds device identity and device-scoped
bindings on the single seat. Phase 2 onward adds real seats. Device identity is needed
either way, so phase 1 is not wasted if the seat work is later cut back.

## 1. Current state

### 1.1 One seat, one of everything

- `crates/gnoblin-wayland/src/state.rs` creates one seat with `new_wl_seat(&display_handle, seat_name)`
  at line 746, then one keyboard (`add_keyboard`, line 748), one pointer (line 750)
  and one touch (line 751). The comment at lines 429-433 says these are the
  "one seat this slice creates".
- The seat name is one string, `seat_name` (line 462). The default is `"seat0"`
  (`crates/gnoblin-wayland/src/server.rs:249`). The name is reused by `MenuRequest`
  (`state.rs` line 2330).
- Per-pointer state is single: `pointer_position` (line 532), `held_buttons` (line 534,
  a count shared by all buttons, used for the focus grab in `button_grab`, line 2921),
  and `touch_points` (line 537).
- One cursor: `cursor_image` (line 497). The frame chooses one cursor through
  `pointer_cursor(output)` (lines 3194-3200), and only while the session is unlocked.
  The cursor is drawn as a frame overlay through `set_cursor_image`
  (`crates/gnoblin-render-frame/src/frame.rs` line 528).
- `design/input-status.md` section 8 says `wp_cursor_shape_v1` and `wl_cursor` requests
  are recorded and never painted, and that drawing them needs a cursor plane on the
  DRM backend. [FACT: the doc says so; not re-tested here.]
- The clipboard "follows the keyboard" (`state.rs` line 1527). There is one
  selection.

### 1.2 Input path drops device identity

- `crates/gnoblin-backend/src/lib.rs`: `InputEvent` has `DeviceAdded { id, kind }`
  and `DeviceRemoved { id }` (lines 348 and 358, `DeviceKind` at 369). The events
  that carry input have no device field: `PointerMotion` (line 280), `PointerButton`
  (line 302), `Key` (line 309), `PointerAxis` (line 322) and `Touch` (line 334).
- `crates/gnoblin-backend-drm/src/libinput/mod.rs`: one libinput context, opened
  through the session's seat (`Libinput::open`, line 147). `translate()` (line 280)
  turns keyboard, pointer and touch events into `DeviceEvent`s for the mapper and
  does not pass the device on. Gesture events keep the device id in
  `RawEvent::Gesture` (`crates/gnoblin-backend-drm/src/lib.rs` lines 517-519).
  Tablet tool events are not routed through the mapper (`libinput/mod.rs` lines
  409-412). The comment at lines 404-407 says they are for `gnoblin-input` to read from the raw
  channel, and that this is not wired yet.
- The mapper (`libinput/map.rs`, `Mapper` at line 126) keeps one pointer and one
  set of held buttons.
- `state.rs` `input()` (line 3211) handles `DeviceAdded` and `DeviceRemoved` by doing
  nothing (line 3357). The same is true of the other path at line 2469.
- Smithay can give the device for each libinput event. `smithay` at rev `65aa86f`
  has `fn device()` on each libinput event (`src/backend/libinput/mod.rs` line 108),
  and the `input` crate 0.10 has `Device::seat()` (`src/device.rs` line 449 in the cargo registry).
  [FACT] Gnoblin's mapper throws this away.
- A libinput context is given one seat (`udev_assign_seat`, `src/context.rs` in the
  `input` 0.10 cargo registry, line 285). Devices on a second seat need a second context. [INFERENCE] That context
  needs its own libseat session. [UNVERIFIED] Whether a non-active seat's devices can
  be opened without root on the test machines.

### 1.3 Focus and pointer state in the window manager

- `gnoblin-wm` `Wm` (`crates/gnoblin-wm/src/wm.rs`, struct at line 297): one
  `focus: Option<WindowId>` (line 304), one `pointer: PointerFocus` (line 313),
  one `focused()` (line 515). `pointer_focus.rs` `PointerFocus` (line 47) holds one
  `over`, one `pending`, and one `grabbed` flag.
- No type in `gnoblin-wm` names a seat. A grep for `seat` in the crate finds only a
  doc comment in `effect.rs` (line 7).
- Pointer grabs for move and resize (`drag.rs`) are single.

### 1.4 Keys, bindings and the router

- `wayland/keyboard.rs`: one `KeyboardHandle` (`state.rs` line 429) and one xkb state.
  `dispatch_key` (line 562) takes a keycode with no device. `route_key_edge` (line 602)
  sends `RouterEvent::Key { key_code, symbol, modifiers, state, repeat }` to the router
  with no device.
- `remember_layout` (line 516) uses `keyboard.current_focus()` (line 520).
  `open_focused_window_menu` (line 655) uses `wm.focused()`. Both assume one seat.
- `crates/gnoblin-input/src/router.rs`: `Event::Key` and `Event::Button` (line 105)
  have no device. `route` (line 301). The owned presses map is
  `BTreeMap<Pressed, Target>` (line 224), and `Pressed::Key(KeyCode)` (enum at line 214)
  is keyed by keycode alone. Two keyboards holding the same keycode collide here.
- `Scope` (line 48) picks which device class a grab takes: `Keyboard`, `Pointer` or
  `Both`. It does not pick a device.
- `binding.rs` `Binding` (line 65): `id`, `name`, `chord`, `trigger`,
  `allow_when_inhibited`. No device matcher.
- `chord.rs` `matches_key` (line 396) and `matches_button` (line 404) match on symbol,
  modifiers and button only.
- The gesture recogniser already cancels per device (`device_removed` in
  `crates/gnoblin-input/src/gesture.rs` line 560). So per-device cancel has a precedent.

### 1.5 Config and object model

- Config schema: `crates/gnoblin-config/schema/config.toml`. Section `input` (line 2022)
  has `mouse`, `touchpad`, `trackball`, `pointing-stick`, `keyboard`, `tablets`,
  `styluses`. There is no `seat` section.
- `shortcuts` (line 2491) is a map of `Shortcut` records (record at line 1462, used at line 2498):
  `binding`, `command`, `trigger`, `enable`. No device field.
- `keybindings` (line 2481) is a schema key; this review did not trace how the code reads it.
- `design/lua-input-policy.md` already says the compositor "matches event class and
  device selectors before sending any request" (lines 18-21), and keeps "physical input
  identity" for each queued event. So the Lua design expects device selectors. The
  code does not have them yet.
- `design/rust-compositor.md` lines 3149-3158 (the window-menu signal) already carry
  a `seat` name: the requesting seat for `client`, the key event's seat for
  `keyboard`, and "the default seat" for `extension`. A seat concept is therefore
  already in a decided design, but nothing implements it.
- `design/lua-control.md` line 95 asks for "per-seat cursor image/visibility
  operations". This is the only other seat reference in the design docs.

### 1.6 Lock

- `crates/gnoblin-lock/src/lock.rs`: `Lock` (line 38) holds `subjects`, a set of
  `Subject` (line 41). `Lock::new` (line 61) starts with `Subject::Session` (line 65).
  `Subject` is `Session` or `Client` (`ids.rs` line 20). There is no seat type.
- `design/session-locking.md` line 44 says a session lock covers the seat. ext-session-lock
  is a per-session protocol, so one lock covers every seat that the session has.

### 1.7 Protocols and other subsystems

- EIS (`crates/gnoblin-input-eis`): remote input takes "the seat's own path" and the
  lock gate (crate doc, `src/lib.rs` lines 33-59). Its `DeviceId` (`src/service.rs` line 602) is an EI
  device, not a seat device. [UNVERIFIED] Whether an EIS client can name a seat. The
  grant text says "the seat advertises the grant", so one seat is assumed.
- IME: `protocols/zwp_input_method_v2.rs` and `zwp_text_input_v3.rs` exist. The
  protocols are per seat. [UNVERIFIED] Whether Gnoblin keeps one input method.
- Accessibility keyboard features (the `Accessibility` record, schema line 740, used at line 1158) are
  one configuration for the session. [INFERENCE] They would become per seat.
- Xwayland (`crates/gnoblin-xwayland`): the crate has keyboard-grab settings
  (`settings.rs` line 21, `windows.rs` line 96) and no seat handling. [FACT] No
  multi-seat code. [INFERENCE] Xwayland binds one seat. X11 core has one pointer and
  one keyboard. XInput2 has multiple master devices, but Xwayland's mapping to
  Wayland seats is not verified here.
- Toolkits: GTK 4 and Qt 6 Wayland backends are believed to handle several
  `wl_seat`s, but Qt's support is partial. [UNVERIFIED] Phase 5 must measure this.

### 1.8 Smithay support

Read at rev `65aa86f` (`~/.cargo/git/checkouts/smithay-312425d48e59d8c8/65aa86f`):

- `SeatState::new_wl_seat(display, name)` can be called more than once
  (`src/wayland/seat/mod.rs` line 179). Each call makes a separate seat global.
- Each seat has its own `KeyboardHandle`, `PointerHandle` and `TouchHandle`
  (`src/input/mod.rs` lines 464, 597, 692). Each keyboard handle keeps its own focus
  (`src/input/keyboard/mod.rs` `set_focus`, line 1300).
- The `SeatHandler` trait (`src/input/mod.rs` line 155) takes the `Seat` argument in
  its `focus_changed` and `cursor_image` callbacks (lines 167 and 170). So the trait
  is already per seat.

[FACT] Smithay does not block several seats. The cost is in Gnoblin's `State`, which
has one of each field.

## 2. Requirements

Each requirement is labelled as either **required** (Kieran's words) or **proposed**
(this document's addition).

### 2.1 Seats

- **Required:** several seats on one compositor, each with its own keyboard, pointer,
  keyboard focus and visible cursor.
- **Proposed:** outputs are shared by all seats. This is what "several people at
  once" means here: one screen, several people. The model where each user has their
  own screens is a different design (one compositor per seat, as logind multi-seat
  does). It is out of scope. See section 5.
- **Proposed:** each device belongs to exactly one seat. The seat is chosen by rule:
    - a libinput/udev `ID_SEAT` tag if one is set (the logind model);
    - otherwise a Gnoblin `seats` rule that matches the device by name, vendor/product,
      physical path or kind;
    - otherwise the default seat `seat0`. Hot-plugged devices go there too.
- **Proposed:** a seat can be created and removed without restarting the session.
- **Proposed:** a `seats` config section and matching Lua API (`gnoblin.seats`).

### 2.2 Focus

- **Proposed:** each seat has its own keyboard focus window. Two seats can focus
  the same window. Each seat's typing goes to its own focus.
- **Proposed:** the window manager keeps one stacking order and one tiling layout for
  all seats. Layout engines stay global. The "active" window (the decoration and
  title highlight) is a separate, global choice. See section 5.
- **Proposed:** `remember_layout` and `open_focused_window_menu` use the focus of the
  seat that caused them, not `keyboard.current_focus()` or `wm.focused()`.

### 2.3 Pointer and cursor

- **Proposed:** each seat has its own pointer position, pointer focus, button state
  and touch points.
- **Proposed:** move and resize grabs belong to one seat. A second seat can move
  another window while the first seat is still dragging.
- **Proposed:** each seat draws its own cursor, with its own shape. A seat may get a
  colour or a label so people can tell cursors apart. This is optional. See section 5.
- **Proposed:** pointer constraints and relative pointer apply per seat. A lock held
  by one seat does not stop another seat's pointer.

### 2.4 Clipboard, drag-and-drop, input methods

- **Proposed:** each seat has its own selection (clipboard and primary) and its own
  drag-and-drop. The "clipboard follows the keyboard" rule (`state.rs` line 1527)
  becomes "the clipboard follows the seat that the client has focus on".
- **Proposed:** each seat has its own text input and input method, or the input method
  is shared with a seat name. See section 5.

### 2.5 Lock

- **Proposed:** a session lock locks every seat. Reason: `ext-session-lock` is per
  session. A lock that covered only one seat would leave other seats able to read
  and drive windows that the lock was meant to hide.
- **Proposed:** a lock blocks input from every seat. Lock state is checked before each
  seat's events are delivered.

### 2.6 Shortcuts and accessibility

- **Required:** bindings that match one device (for example a macro keyboard).
- **Proposed:** a binding with no device matches every device, as today.
- **Proposed:** a binding with a seat matches only that seat's devices.
- **Proposed:** keys from a matched device that fire a binding are consumed. They
  never reach a client. Keys that match no binding follow the device's seat (or are
  dropped if the device is marked binding-only; see section 3, option B).
- **Proposed:** chord modifiers come from the matched device's own xkb state, not the
  seat's. A macro keyboard with its own layout gets its own modifiers.
- **Proposed:** keyboard accessibility features (sticky keys, slow keys, bounce keys)
  are per seat. [INFERENCE] Other compositors may scope these per seat; not checked here.

### 2.7 Remote input

- **Proposed:** EIS injection targets a named seat or the default seat. An EIS client
  cannot inject into a seat it was not granted. [INFERENCE] The grant model in
  `gnoblin-input-eis` can carry a seat name.

### 2.8 Xwayland

- **Proposed:** an X11 client sees exactly one seat, chosen by config. X11 core has
  one pointer and one keyboard, so Xwayland cannot show several people's input to
  one X client. See section 5.

## 3. Options

Each option says what breaks, the Xwayland and toolkit story, the lock and security
story, and the effort in lanes. A lane is one worktree slice with its own tests.

### Option A: real multi-seat in one step

Build N seats in `State`, device-to-seat rules, per-seat focus, pointer, cursor,
clipboard and bindings, all at once.

- **What breaks:** every `State` field listed in section 1.1 must move into a per-seat
  struct. `wm` focus, pointer focus and grabs need a seat argument on every event.
  Every call site of `wm.focused()` changes. The frame cursor path changes.
- **Xwayland:** one seat only (section 2.8). Needs a decision.
- **Toolkits:** the risk is real. Clients that assume one seat (some Electron and
  Qt paths, unverified) could break when a second `wl_seat` appears. The fix would be
  an option to hide extra seats from some clients, which is itself a new policy.
- **Lock and security:** one lock for all seats. A client attached to two seats could
  see focus changes on both, through `focus_changed` events. [INFERENCE]
- **Effort:** largest. Many lanes change the same `State` and `Wm` code at the same
  time, which is hard to review. Few lanes can run in parallel. [JUDGEMENT] 8-12
  lanes, one at a time.
- **Reversibility:** low once clients depend on several seats.

### Option B: device identity and device-scoped bindings on one seat

Pass device identity through the input path. Let bindings and grabs name a device.
Keep one seat.

- **What breaks:** the router's owned-press map must key presses by device and
  keycode, not keycode alone (section 1.4). Binding matchers and config change. Device
  removal must release owned presses of that device.
- **Xwayland:** no change.
- **Toolkits:** no change; there is still one seat.
- **Lock and security:** no change. A binding on the macro keyboard still cannot run
  when locked, unless the binding is marked `allow_when_inhibited` as today.
- **What it does not give:** a second person typing into their own window. The
  macro keyboard types into the one focus, or into nothing if it is binding-only.
- **Effort:** small, 3-4 lanes. Each lane can be tested with synthetic events in the
  router and the mapper.
- **Reversibility:** high. The device matcher is additive. Nothing a client sees changes.

### Option C: visual-only extra cursors through IPC

Keep one real seat. Draw extra cursors for other people as overlays driven by IPC
(for example from a second input source).

- **What breaks:** nothing in the input path. Extra cursors cannot type or click
  into windows, so this is not multi-seat.
- **Xwayland and toolkits:** no change.
- **Lock and security:** overlays must be hidden under a lock, the same as the
  pointer (`state.rs` lines 3194-3200 already do this for the one cursor).
- **Effort:** 2 lanes. A fast demo.
- **Reversibility:** high. It is also a dead end for real multi-seat.

### Option D: option B, then option A (recommended)

Ship option B in phase 1 (device identity and device bindings). Use it to test the
per-device paths. Then build option A in phases 2 to 5, one seat-level change at a time.

- **What breaks:** the same as A, but spread over more lanes. Each phase leaves the
  compositor working with one seat, so a stopped phase is still shippable.
- **Xwayland and toolkits:** measured in phase 5, before any default changes.
- **Lock and security:** decided in phase 4 (section 2.5), with tests that each seat's
  events are blocked while locked.
- **Effort:** phase 1 is 3-4 lanes; phases 2-5 are 8-12 lanes. Total is larger than
  A alone, because phase 1 is extra work, but each lane is reviewable.
- **Reversibility:** high at each phase boundary. Phase 1 has no seat concept, so it
  is independent of the choice in section 5.

**[JUDGEMENT]** Option D. Kieran asked for full multi-seat and device bindings, and
device identity is needed for either. Option C is not multi-seat. Option A in one step
would make a large, hard-to-review change to `State` and `Wm` before the device
identity exists.

## 4. Phased plan

Each lane has: tests first, the files it touches, and its acceptance. Builds run on
the agent machine, not in the lane (per the lane rules). Every lane updates
`design/input-status.md` if its row changes.

### Phase 1: device identity and device-scoped bindings (option B)

**Lane 1.1: device identity on input events.**

- Tests first: mapper unit tests in `gnoblin-backend-drm` (no hardware). Each
  keyboard, pointer and touch event carries the device it came from. Device removal
  retires that device's state.
- Files: `crates/gnoblin-backend/src/lib.rs` (add a `DeviceRef` with `id`, `name`,
  `vendor`, `product`, `kind`, `seat` to `Key`, `PointerButton`, `PointerMotion*`,
  `PointerAxis` and `Touch`); `crates/gnoblin-backend-drm/src/libinput/mod.rs` and
  `map.rs` (pass the device through `translate()` and the mapper).
- Acceptance: every input event has a device; gesture and input events agree on the id.

**Lane 1.2: device-keyed router state.**

- Tests first: router tests (`crates/gnoblin-input/src/router.rs` test module, line 564)
  with two keyboards pressing the same keycode. Each release goes to its own press.
  No duplicate or orphan counts. Device removal releases only that device's presses.
- Files: `crates/gnoblin-input/src/router.rs` (owned presses keyed by device and
  keycode, `Pressed` at line 214); `Event` (line 105) gains the device.
- Acceptance: the stats `duplicate_presses` (line 193) and `orphan_releases`
  (line 195) stay at 0 with two keyboards.

**Lane 1.3: per-device xkb state for bindings.**

- Tests first: a chord with the macro keyboard's layout matches only when that
  keyboard's modifiers are held.
- Files: `crates/gnoblin-wayland/src/keyboard.rs` (`dispatch_key`, line 562,
  `route_key_edge`, line 602): a per-device xkb state, created from the device's
  keymap or the seat's default.
- Acceptance: a Shift held on keyboard A does not change keyboard B's chord.

**Lane 1.4: device selector in bindings and config.**

- Tests first: config validation tests (`crates/gnoblin-config/tests/shortcuts.rs`)
  for a `device` field on `Shortcut`; a binding test in `gnoblin-input` that a
  device-scoped binding does not fire on another keyboard.
- Files: `crates/gnoblin-input/src/binding.rs` (`Binding`, line 65, gains an optional
  device matcher); `crates/gnoblin-input/src/chord.rs` (matchers, around lines 396 and
  404); `crates/gnoblin-config/schema/config.toml` (`Shortcut`, line 1462); the matching
  `crates/gnoblin-config/src/document.rs`; `design/lua-input-policy.md` (device selector
  names, shared with Lua).
- Acceptance: a shortcut with a device selector fires only from that device, and its
  press and release are consumed. A key from that device with no matching binding
  either reaches the seat or is dropped by the binding-only setting (section 5).

### Phase 2: seats (option A starts)

**Lane 2.1: seat object and seat table in `State`.**

- Tests first: a test that creates two seats, each with a `wl_seat` global, each
  with its own keyboard handle; the seat names are distinct.
- Files: `crates/gnoblin-wayland/src/state.rs` (replace the single `seat`,
  `keyboard`, `pointer`, `touch`, `seat_name`, `pointer_position`, `held_buttons`,
  `touch_points` and `cursor_image` fields, lines 425-537, with a per-seat struct);
  `crates/gnoblin-wayland/src/server.rs` (seat names from config).
- Acceptance: one seat behaves exactly as today (no change in the existing tests).

**Lane 2.2: device-to-seat rules.**

- Tests first: a device with `ID_SEAT=seat1` goes to `seat1`; a `seats` rule matching
  name or vendor/product wins over the default; a hot-plugged device goes to the
  default seat; a removed device leaves no seat state.
- Files: `crates/gnoblin-config/schema/config.toml` (a `seats` section); the
  `gnoblin-config` document; `crates/gnoblin-backend-drm/src/libinput/mod.rs`
  (seat tag from udev, and the second libinput context for a second seat, if needed).
- Acceptance: device-to-seat rules are a pure function over the device record and are
  unit-tested without hardware.
- Open: a second libinput context needs a second libseat session (section 1.2).
  [UNVERIFIED] Whether this works without root on the target machines. Phase 2
  starts with the rules and the pure function; the second context is a separate lane
  gated on that check.

**Lane 2.3: per-seat keyboard focus.**

- Tests first: two seats focus two windows; a key on seat A reaches window A and not
  window B. Focus changes on one seat do not change the other.
- Files: `crates/gnoblin-wm/src/wm.rs` (focus per seat, `focus: Option<WindowId>` at
  line 304 becomes a map keyed by seat); `crates/gnoblin-wayland/src/keyboard.rs`
  (`set_keyboard_focus` per seat); `remember_layout` (line 516) and
  `open_focused_window_menu` (line 655) read their own seat's focus.
- Acceptance: a key on each seat reaches that seat's focus only.

### Phase 3: per-seat pointer and cursor

**Lane 3.1: per-seat pointer position, focus and button state.**

- Tests first: two pointers over two windows; each pointer's focus follows only its
  own motion. Button state is per seat, so `held_buttons` (line 534) is per seat.
- Files: `crates/gnoblin-wm/src/pointer_focus.rs` (`PointerFocus`, line 47, per seat);
  `crates/gnoblin-wayland/src/state.rs` (`pointer_position`, line 532; `held_buttons`,
  line 534; `button_grab`, line 2921; `activate_at`, line 2768; `report_pointer`,
  line 2782; `move_pointer`, line 1562).
- Acceptance: focus follows the pointer of the seat that moved.

**Lane 3.2: per-seat move and resize grabs.**

- Tests first: seat A moves window 1 while seat B resizes window 2 at the same time.
- Files: `crates/gnoblin-wm/src/drag.rs`; the grab state in `gnoblin-wm` and
  `crates/gnoblin-input/src/router.rs` (grabs, `begin_grab`, line 272), which are
  global today.
- Acceptance: two grabs run at once, one per seat.

**Lane 3.3: per-seat cursor image and drawing.**

- Tests first: each seat's cursor shape is reported to the frame with that seat's
  position; a lock hides every seat's cursor (as `state.rs` lines 3194-3200 do for one).
- Files: `crates/gnoblin-wayland/src/state.rs` (`cursor_image`, line 497; the cursor
  choice at lines 3194-3200); `crates/gnoblin-wayland/src/protocols/wp_cursor_shape_v1.rs`
  (`update_cursor`, line 83; `pointer_cursor`, line 105); the frame overlay
  (`crates/gnoblin-render-frame/src/frame.rs` line 528) and the DRM cursor plane
  (`design/input-status.md` section 8). Each cursor needs its own plane or a composite.
- Acceptance: two cursors are drawn at the two positions. Optional colour or label is
  a separate lane, if Kieran asks for it.

### Phase 4: per-seat policy

**Lane 4.1: clipboard, primary selection and drag-and-drop per seat.**

- Tests first: a copy on seat A does not change seat B's clipboard. A paste on seat B
  reads B's selection.
- Files: `crates/gnoblin-wayland/src/state.rs` (clipboard, line 1527); the data-device
  handling.
- Acceptance: per-seat selections; a copy on one seat does not reach the other seat.

**Lane 4.2: input methods per seat.**

- Tests first: a text input on seat A and one on seat B each get their own commits.
- Files: `crates/gnoblin-wayland/src/protocols/zwp_input_method_v2.rs`,
  `zwp_text_input_v3.rs`.
- Acceptance: each seat's text input commits to its own client. Decision on one or
  several input methods (section 5).

**Lane 4.3: keyboard accessibility per seat.**

- Tests first: sticky keys on seat A do not stick keys on seat B.
- Files: the accessibility keyboard path (schema `Accessibility` record, line 740).
- Acceptance: per-seat keyboard accessibility state.

**Lane 4.4: lock across seats, EIS seat targeting, Xwayland.**

- Tests first: a locked session blocks input on every seat (section 2.5); an EIS
  client granted seat A cannot inject into seat B; the Xwayland seat is the config
  value.
- Files: `crates/gnoblin-lock/src/lock.rs` (no change expected: the lock already covers
  the session); `crates/gnoblin-input-eis/src/lib.rs` (seat name in the grant);
  `crates/gnoblin-xwayland/` (seat choice).
- Acceptance: the three tests pass; the lock decision in section 2.5 is recorded.

### Phase 5: toolkit and real-client matrix, docs

**Lane 5.1: multi-seat matrix.**

- Clients to test with two seats: GTK 4, Qt 6, Chromium or Electron, Firefox, an
  Xwayland client (`xeyes`), and a terminal. For each: does it see two seats, does
  each seat type into its own window, and does a second seat's cursor render.
- Method: an e2e run in a nested session with EIS-injected input on two seats
  (if the EIS grant supports seat names), or two virtual devices. [UNVERIFIED]
  Whether the existing e2e harness can create a second seat.
- Acceptance: a written matrix in `design/multi-seat.md` with each cell tested, not
  inferred.

**Lane 5.2: docs and decision.**

- Files: `design/input-status.md` (rows for multi-seat and device bindings);
  `design/rust-compositor.md` (D31 accepted or changed); `design/lua-input-policy.md`
  (device and seat selectors); `design/docs-parity/config.md` (new `seats` and
  `device` keys); changelog.
- Acceptance: docs match the code; no row says "absent" for a feature that shipped.

## 5. Needs Kieran

Each item is a real decision. The author's recommendation is given where there is
one.

1. **Shared screens or separate screens?** This document assumes one shared screen
   for all seats (section 2.1). The other model (each user has their own screens and
   their own compositor) is logind multi-seat and is a different design. Confirm the
   shared-screen model is what Kieran means.
2. **Xwayland.** Does an X11 client see only one seat (seat0, or a configured one)?
   X11 core cannot show several people's input to one client. **[JUDGEMENT]** Yes,
   one configured seat.
3. **Default seat for new devices.** Hot-plugged devices with no rule: go to `seat0`
   (the author's suggestion), or stay unassigned until a rule claims them?
4. **Can one window be focused by two seats at once?** Keyboard focus per seat says
   yes (section 2.2). Should the window manager's "active" highlight follow one seat,
   the last seat that focused, or stay independent? **[JUDGEMENT]** The last seat
   that focused sets the active highlight; each seat keeps its own keyboard focus.
5. **Cursor appearance per seat.** Plain cursors, or a colour or label per seat so
   people can tell them apart? Optional (section 2.3).
6. **Lock.** One lock for all seats (section 2.5, the author's recommendation), or a
   per-seat lock that needs a non-standard protocol? **[JUDGEMENT]** One lock for all
   seats.
7. **Binding-only devices.** A macro keyboard that only fires bindings and never types
   into windows: should Gnoblin support that as a device setting, or is "no bindings
   match, so keys follow the seat" enough?
8. **Input methods.** One input method shared by all seats, or one per seat?
9. **Accessibility per seat.** Per-seat sticky and slow keys (section 2.6), or one
   setting for the session?
10. **Second libseat session.** Is it acceptable that a second seat needs its own
    libseat session and possibly root on the target machines? Until this is checked
    on hardware (section 1.2), phase 2 cannot be promised.
11. **Phase 1 first.** Is it acceptable to ship device identity and device-scoped
    bindings before any seat work (option D)?
