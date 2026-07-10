# ext-session-lock-v1 — implementation plan (DEFERRED, needs runtime validation)

**Status:** vendored protocol XML only (`ext-session-lock-v1.xml`). Not wired into
the build. Deferred because the security-critical behaviour cannot be verified by
inspection and cannot be runtime-tested headless — a session lock that compiles but
subtly leaks input or shows content underneath is *worse than none*.

Enables `swaylock`, `hyprlock`, `gtklock`, `waylock`, and any other
ext-session-lock-v1 client. Protocol XML is the canonical wayland-protocols
staging copy.

## Protocol shape (v1)

- `ext_session_lock_manager_v1`: `lock() -> ext_session_lock_v1`, `destroy`
- `ext_session_lock_v1`: `get_lock_surface(surface, output) -> ext_session_lock_surface_v1`,
  `unlock_and_destroy`, `destroy`; events `locked`, `finished`
- `ext_session_lock_surface_v1`: `ack_configure(serial)`, `destroy`; event
  `configure(serial, width, height)`

## Implementation approach

Model the lock surface on the existing layer-shell shell-surface subclass —
`src/protocols/layer-shell/meta-wayland-layer-shell.c` is the template (a
`MetaWaylandShellSurface` subclass that creates a `MetaWindow`, sizes/stacks it,
and drives the configure/ack handshake).

- `MetaWaylandSessionLockSurface : MetaWaylandShellSurface`, one per output, sized
  to the output's logical-monitor geometry, stacked **above everything** (reuse the
  stacking-layer mechanism from `patches/mutter/30-layer-shell/0002-*honor-wlr-stacking*`;
  use the topmost layer / `META_LAYER_OVERRIDE_REDIRECT`). Send `configure(serial,
  w, h)` on map and on output resize; require `ack_configure` before the buffer is
  shown (same gate as layer-shell).
- Manager `lock()`: refuse (send `finished`) if a lock is already active; otherwise
  begin locking.
- `locked` event must be sent **only after** all normal content is hidden and the
  compositor is ready to route input solely to lock surfaces.

## The security-critical parts (MUST be runtime-validated before enabling)

1. **Content hiding** — on `lock()`, blank/hide all normal windows immediately;
   show black until lock surfaces commit on every output. Do not reveal the desktop
   at any point while locked. Needs a compositor-level "locked" state that suppresses
   normal `MetaWindow` visibility (investigate `meta_window_update_visibility` /
   stage paint suppression) or a full-output opaque actor above all windows.
2. **Input isolation** — route **all** keyboard, pointer and touch only to lock
   surfaces; no normal surface may receive input. layer-shell EXCLUSIVE keyboard
   (`META_WAYLAND_LAYER_SHELL_KEYBOARD_FOCUSABLE_KEY`) is a partial precedent for
   keyboard; pointer + touch focus forcing needs `MetaWaylandSeat`/pointer/keyboard
   focus to be constrained to the lock client.
3. **Crash safety** — if the lock client dies while locked, the session must STAY
   locked with a blank screen (never auto-unlock). Handle wl_client destroy while a
   lock is active.
4. **Multi-output / hotplug** — a new output appearing while locked must also be
   covered (client gets a new lock surface; until then that output stays blank).
5. **unlock_and_destroy** restores the normal session (un-hide windows, restore
   input routing).

## mutter integration points

- Shell-surface role + window: `src/wayland/meta-wayland-shell-surface.h`,
  `meta-wayland-actor-surface`, the layer-shell template.
- Stacking above all: the `30-layer-shell/0002` stacking patch + `MetaStackLayer`.
- Input focus forcing: `MetaWaylandSeat`, `MetaWaylandKeyboard`/`Pointer` focus.
- Output geometry: `meta_wayland_output_get_monitor` → `MetaLogicalMonitor` layout
  (see `get_monitor_layout()` in the layer-shell overlay).

Same pattern as the other gnoblin protocols: overlay `.c/.h` + `manifest`, add the
init call to `src/protocols/aggregator/meta-gnoblin-protocols.c`, add sources +
`ext-session-lock-v1` to `scripts/gen-gnoblin-protocols-patch.sh`, and gate it via
an `ext-session-lock` key in the `[protocols]` section of `gnoblin.conf` (see
"Authoring a protocol overlay" in the root README).

## Runtime validation checklist (the actual "validated")

Run in a real Gnoblin/mutter session with a lock client:
- [ ] `swaylock` (or gtklock) locks; input cannot reach apps underneath.
- [ ] No desktop content visible at any point while locked (incl. the lock→map gap).
- [ ] Killing the lock client (`kill -9`) leaves the screen locked and blank.
- [ ] Multi-monitor: every output is covered; hotplugging an output stays covered.
- [ ] `unlock_and_destroy` fully restores the session and input.
