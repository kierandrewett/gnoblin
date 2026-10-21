# Dismissed layer activation regression

Run this fixture before integrating the issue 207 candidate:

```sh
CC=clang python3 tests/layer-activation-context/run.py --sanitize \
  --compile-commands build/make/mutter/compile_commands.json
```

It needs GLib/GObject/GIO and Wayland server development files, Wayland
protocols, `wayland-scanner`, Git, Python, and a C compiler. The optional
`--compile-commands` check needs an already configured Mutter build. It runs
syntax checks only and writes no objects into that build.

The runner reconstructs seven files from the Mutter revision in
`versions/gnome-versions.json`, then applies the patch series to a temporary directory.
It compiles the whole production activation translation unit and extracts the
production keyboard, pointer, touch, and tablet delivery functions verbatim.
Keyboard and pointer grab validation also use the production functions.
It does not modify the shared submodule or start a desktop.

To reproduce the original failure, omit the candidate patch:

```sh
python3 tests/layer-activation-context/run.py --baseline --test /lifecycle/dismiss
```

That command should fail with zero activations where one was expected. For
iteration on disposable prepared source, use `--source-tree DIRECTORY`.

## Candidate behavior

The new patch is
`patches/mutter/99-typed-window-api/0124-preserve-layer-activation-context.patch`.
It follows the existing typed API patches, including 0050 and 0076. It changes
activation handling and adds small input-delivery hooks; it does not retain a
surface reference, keyboard grab, or invisible popup.

For Gnoblin layer sources, commitment checks the requesting client's surface
and seat ownership, the compositor's actual event recipient and serial, and
Mutter's current grab validation. Focus-enter serials alone do not authorize
this path. Events flagged synthetic, input-method, or repeated do not create
saved authorization. A returned token string does not imply authorization.

An authorized token keeps the input event's timestamp. Dismissing, unmapping,
or destroying the source afterward does not remove that authorization. It
expires 15 seconds after event delivery, even if the startup timeout callback
has not run.

A later press, touch start, scroll, or lock transition revokes authorization.
Releasing the initiating button or key preserves the press serial, timestamp,
and expiry. Only a button press, key press, or touch begin can record new
authorization. Consumption also checks Mutter's last user time with wraparound
handling and the current lock state.
Unlocking does not revive a revoked token.

Standard XDG activation can transfer the token to another client. The typed
focus API still checks the requesting PID and consumes the same token. Valid
standard activation completes the sequence before focus side effects, so
callbacks cannot replay it. Committed request arguments are immutable; token
resource destruction and startup completion detach their references safely.

These authorization checks govern strict focus policy
(`focus_new_windows = "prevent"`). Explicit `"allow"` policy keeps its existing
permissive behavior, including activation without a token. Non-Gnoblin
activation and Gnoblin non-layer validation retain their existing behavior.

## Why these changes belong together

The legacy keyboard grab helper accepts focus-enter serials as well as key
serials. Input-delivery hooks establish which surface actually received the
initiating event before its authorization can survive dismissal. Saving only
the result of the legacy helper would also preserve focus-only context.

The snapshot depends on request arguments staying fixed after commitment.
Allowing setters afterward could change the source or seat, including moving
a request committed without a layer source into the legacy validation path.
The setter checks prevent that change of validation path.

Completing the startup sequence frees the token before focus callbacks run.
Its Wayland request resource may still exist, so it must stop pointing at the
freed token. The resource destructor also clears the reverse link. Otherwise,
checking an invalid later setter or destroying objects in the opposite order
could access freed memory.

These changes could be split into a prerequisite patch, but the snapshot
candidate would depend on it. They belong to the same token lifecycle; an
independent overlay would neither enforce that dependency nor replace the
upstream protocol handlers involved here.

The C fixture is [`regression.c`](regression.c). [`run.py`](run.py) reconstructs
patched source and generates temporary `activation.inc` and
`input-delivery.inc` files for that fixture. The C fixture is not embedded in
the runner, and it is not a native desktop or GTK integration test.

## Press and release ordering

Qt's `lastInputSerial()` retains the press serial when a pointer or keyboard
release arrives. QML `onClicked` can therefore request a token for press
serial 35 after the compositor has delivered release serial 36. Recording only
the latest delivered serial loses the initiating event before commitment.

The recorder now ignores release events before changing its saved context.
This preserves the initiating press and its original expiry. The regression
replays serials 35 then 36 before committing 35, with the timestamps from the
native denial. It covers pointer and keyboard input, later-press rejection,
and release without an expiry refresh.

In [Qt 6.10.3 input handling](https://github.com/qt/qtbase/blob/v6.10.3/src/plugins/platforms/wayland/qwaylandinputdevice.cpp),
`setLastInputDevice` runs for pointer/key presses and touch-down, not their
releases. [Tablet tip-down](https://github.com/qt/qtbase/blob/v6.10.3/src/plugins/platforms/wayland/qwaylandtabletv2.cpp)
sets the same context; tip-up carries no serial. Tablet side-button events
update the seat serial but do not update `lastInputSerial()` in that Qt code.
The tablet delivery fixture does not prove Qt side-button launch support.

## Evidence and limits

The 46 cases cover:

- Source lifecycle: held, dismissed, destroyed, and deferred mapping.
- Input proof: four device types, wrong recipients, missing or foreign context,
  focus-only serials, and timestamp wraparound.
- Revocation: later input, expiry, lock/unlock, and replay during focus.
- Protocol boundaries: token-object lifetime, cross-client transfer, typed peer
  checks, and immutable request arguments.
- Gesture ordering: press/release/commit using the press serial, without an
  expiry refresh or acceptance after another press.

Wayland resources, client credentials, weak references, GLib objects, signals,
and token tables are real. Display time, touch/tablet grab lookup, target
window activation, and lock transitions are controlled substitutes. Pointer
focus/press state is supplied by the fixture; pointer-focus synchronization
is a no-op. The keyboard and pointer grab checks are production code.

The fixture proves policy and lifetime behavior, not real device routing,
GTK active-window state, or desktop integration.

Main must validate the installed candidate in the disposable VM with strict
focus policy: held source, dismissed source, tokenless launch, later input,
and replay. Capture both the compositor's focused window and GTK active state
after releasing the source. Also check lock and expiry behavior there.

Main owns integration, public documentation, Git, and issue updates. This work
changes only the new patch and this regression folder.

## Source basis

The fixture starts from Mutter
`138a14fbeef09d49ebf5be8a0cb83b042dd5c841` plus Gnoblin's patch series. Relevant
source is `src/wayland/meta-wayland-activation.c`, the seat and input-device
implementations beside it, and `src/core/startup-notification.c` for the
15-second startup lifetime.

The [XDG activation protocol](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/xdg-activation/xdg-activation-v1.xml)
specifies that a committed token may be transferred between clients, that an
invalid request still receives a token, and that destroying the request object
does not invalidate the issued token. The local protocol used by the fixture
is `/usr/share/wayland-protocols/staging/xdg-activation/xdg-activation-v1.xml`.

## Native VM evidence

On 2026-10-13, the installed candidate was checked with a real Quickshell
layer surface, standard XDG tokens and GTK windows under strict focus policy.
The mapped Mutter library SHA256 was
`cacc48b8a8dd2f22ffc14cb3b416364dc0ad87b3024cd57ff2946d32553e62b9`.
Held and dismissed launches focused the target; the held surface required
release before GTK received active state. Tokenless launches remained unfocused.

Separate native cases rejected exact-token replay, later input before dispatch,
a 16-second delay and a lock/unlock transition before dispatch. Replay launched
two GTK windows: only the first became active, and the second received an
activation denial. The compositor stayed alive throughout. These checks used
no debugger. Evidence is in the VM run's `astra-activation-native-matrix.json`
and per-case logs. Actual installed dock and user-manager launch delivery are
separate integration checks.

An earlier candidate failed real clicks because a release serial replaced the
initiating press serial; Qt retained the press serial. The fixture now covers
the actual press, release, commit sequence. That earlier VM also crashed after
a debugger inferior call. Its core is retained; no production cause is assigned
from that instrumented run.
