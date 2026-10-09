# Lua input policy

## Requested behavior

Lua owns normal-session keyboard and pointer shortcut policy and can handle
other input devices. Shortcuts are convenience declarations over a common
input handler. Mutter owns device access, physical input state, event delivery,
window grabs, and lock isolation. None of this requires systemd.

## Implementation

A dedicated input handler registry must receive immutable event records and
return a delivery decision. It must not use ordinary notification dispatch:
that discards callback results and serializes the complete config after each
event. Handler declarations publish native match metadata; callbacks stay in
the isolated Lua runtime.

The compositor matches event class and device selectors before sending any
request. Events without a matching handler follow normal delivery immediately.
Selected events are copied into a bounded FIFO, with their physical
input identity retained. Pointer targets are picked from the original coordinates
before subsequent queued events update focus. Later related events cannot overtake pending events.
The compositor never blocks waiting for Lua. A correlated decision resumes the
original event once or consumes it. Input callbacks use a dedicated private
request and response, not a full configuration transaction. A deadline, worker failure, invalid reply,
or reload retires pending requests and restores delivery. Late replies cannot
act on retired events.

The initial core filter was too late: Clutter had already updated sprite
focus before reaching it. Interception must run before stage device updates.
The replay path bypasses that one interception hook, then performs the normal
physical updates and routing exactly once. A private replay decision lets core
routing advance seat state while withholding consumed events. Using
`clutter_event_put` at the original core filter would repeat updates and could
retarget an earlier event after a later pointer movement.

## State and security requirements

- Session lock and exclusive input capture run before normal Lua policy.
- Queued events must recheck lock and capture state before resuming.
- Events captured while unlocked cannot leak to a newly active lock surface.
- Consumed key/button presses own their matching release. Touch streams keep
  begin/update/end/cancel decisions coherent.
- Modifier, button, pointer focus, and Wayland state must update exactly once.
- Handler failure cannot leave an application key or compositor grab held.
- Device removal and config reload retire related state.
- Shortcut inhibitors apply to shortcut helpers; low-level handlers need an
  explicit documented scope rather than silently overriding inhibitors.
- Physical event authority is represented by one-use native context tokens.
  Lua cannot create focus authority by constructing an event table.
- Remapping must validate class-specific fields and update delivered state;
  unsupported replacements must fail explicitly instead of being acknowledged.
  The initial implementation accepts consume and forward decisions; remapping
  remains unfinished until each delivered-state transition is implemented.

## Completion evidence

Source changes, successful builds, installed artifacts, and a running desktop
are different stages. Before calling the full input model complete, confirm
normal shortcuts, conditional forwarding, consumed press/release pairs,
reload while held, Lua timeout/crash, lock transitions, device removal, native
Wayland and Xwayland delivery, and cursor response with real input. Published
reference pages must describe only the implemented subset.


## Delivery status

The shared registry, private request protocol, native decision queue, keyboard
and pointer callback helpers, and asynchronous command worker are implemented.
The compositor and CLI build successfully. Config-owned input code does not
use systemd. Legacy declarative command shortcuts and Mutter action groups
remain supported through their existing path; migrating every native action to
Lua and synthetic event remapping are still separate unfinished work.

The build is installed, using Meson staging to rewrite installed ELF paths
before selected artifacts are atomically replaced. Backups are kept under
`~/.local/state/gnoblin/build-backups/lua-input-20261006-170610`.
Press/release and touch/gesture decisions remain sticky until their physical
end. Unmatched queued events replay locally instead of round-tripping Lua.

A separate old-install loader failure prevented login after reboot: manually
copied build binaries retained build-tree RPATH. The old binary was repaired,
and registration now checks an environment-independent version launch before
writing a login entry. This failure occurs before config recovery can start.

The new input build has not been exercised with physical input. Fresh login,
locked-session transitions, device removal, timeouts, cursor response, and
Wayland/Xwayland delivery remain runtime acceptance gaps. Bingux is installed
with corrected helper targets, absolute autostart wrapper paths, and bundled
symbolic icon resolution. Its wallpaper was mapped before the reboot; the new
login and updated icons still require visible confirmation.
