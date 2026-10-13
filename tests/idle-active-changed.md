# ScreenSaver active-state checks

Build only the idle service and CLI in an isolated checkout:

```sh
cmake -S . -B build/activechanged -G Ninja
cmake --build build/activechanged --target gnoblin-idle gnoblinctl
python3 tests/idle-active-changed.py \
  --idle build/activechanged/gnoblin-idle --ctl build/activechanged/gnoblinctl
```

The regression starts its own D-Bus daemon and redirects both bus addresses
to it. It runs the compiled service and CLI against a private compositor socket
fixture. It needs Python with PyGObject and `dbus-daemon`; no host service,
Wayland compositor, or logind policy is changed.

It checks:

- The initial `hello`, `subscribed`, and snapshot records, on every reconnect.
- Lock/unlock and requests without a resulting lock.
- Boolean deduplication across covering, locked, and failsafe states.
- Reconnect snapshots and restart while locked.
- GNOME delegation and ownership changes.
- Fragmented and invalid records.
- Shutdown with an active connection or a pending retry.

The pre-change service fails after `GetActive` returns `true`: no
`ActiveChanged(true)` arrives on the private bus. The fixture supplies native
state; it does not prove an actual Wayland lock transition.

## Observe a real lock cycle in the VM

Main owns installation and service management in the disposable VM. After
installing the candidate idle service and restarting it there, run the passive
observer from a terminal inside the unlocked Gnoblin session:

```sh
python3 tests/idle-active-changed-live.py \
  --output /var/tmp/active-changed-live.json --cycles 2 --timeout 120
```

Wait for `READY`, then lock and authenticate twice using the installed locker.
The observer never requests a lock or unlock and never starts or stops a
service. Use a new output path each time.

`--socket PATH` selects a specific
compositor socket; otherwise it uses `GNOBLIN_COMPOSITOR_SOCKET` or the normal
socket under `XDG_RUNTIME_DIR`.

The JSON artifact records the native events, D-Bus booleans, service owner,
and initial/final `GetActive` values. Success requires:

- A compositor-confirmed `locked` event in each cycle.
- Exactly `[true, false, true, false]` on D-Bus.
- An unchanged service owner and a final inactive value.

The default two cycles check repeated transitions. The private regression
covers reconnects and duplicate events. Neither check establishes input,
capture, or lock security.

## Source contract

The service follows `session.status` and
`gnoblin.session.lock-state-changed` from the native compositor. It subscribes
and reads the snapshot on one ordered socket, using the same boolean mapping
as `GetActive`. Disconnecting cannot establish an unlocked state.

GNOME's [ScreenSaver interface](https://github.com/GNOME/gnome-shell/blob/main/data/dbus-interfaces/org.gnome.ScreenSaver.xml)
defines `ActiveChanged(b)`;
[ScreenSaverDBus](https://github.com/GNOME/gnome-shell/blob/main/js/ui/shellDBus.js)
reports the same `screenShield.active` value as `GetActive`.
[ScreenShield](https://github.com/GNOME/gnome-shell/blob/main/js/ui/screenShield.js)
emits only when its active boolean changes. This change adds reporting only;
issue #141's lock-request failure policy remains a separate owner decision.
