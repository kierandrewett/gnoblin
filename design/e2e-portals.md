# Portal end-to-end results

Live runs on Agent37 with a debug build of `rust-compositor`. The gnoblin session ran on a private
session bus with `xdg-desktop-portal` 1.16 as the frontend.

The guardian started `xdg-desktop-portal-gnoblin` as its `portal` child. That child holds the
privileged pair.

The harness is `e2e-live/live.sh guardian` in the PortalRole lane (not committed). Logs:
`PortalStart-logs/live2.txt`.

## Results

| Check                                                      | Route           | Result                                                                                                                                                      |
| ---------------------------------------------------------- | --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Guardian starts the backend as its child                   | session         | PASS. The portal's parent pid is the guardian. The log has `started portal`.                                                                                |
| Backend owns `org.freedesktop.impl.portal.desktop.gnoblin` | session         | PASS.                                                                                                                                                       |
| Backend killed, guardian restarts it                       | session         | PASS. New pid, same guardian parent, restart after 250 ms (attempt 1 of 5).                                                                                 |
| Backend reaches `portal.ui` with its capability            | backend         | PASS. Two consent requests reached `portal.ui`. No `needs the portal-ui capability` refusal appears.                                                        |
| Frontend exports Screenshot (F-P1a)                        | frontend        | PASS. The frontend routes Screenshot to gnoblin, then provides `org.freedesktop.portal.Screenshot`. The call returns a request handle (rc 0).               |
| Frontend Screenshot returns a result                       | frontend        | PARTIAL (F-P2). Request 1: dialog drawn, Escape gives `response=1`, dialog gone, vanished caller tears it down. Request 2 is refused with no dialog (F-P2). |
| Direct backend `Screenshot`, `AccessDialog`                | backend, direct | Block until the caller's timeout (rc 124). This is consent waiting for an answer, which is correct.                                                         |
| `FileChooser.OpenFile`                                     | frontend        | Not re-run in this round. Routed to `gtk;kde` (F-P1b), so with no fallback installed it returns `UnknownMethod`.                                            |
| `Secret.RetrieveSecret`                                    | frontend        | Not exported. Expected: `portals.conf` refuses Secret and `design/keyring.md` says so.                                                                      |

A first attempt was contaminated by the harness, not the product. Its cleanup matched `gnoblin$`,
which never matches the guardian (its argv is `gnoblin`).

Stale guardians from earlier runs kept the bus name. The new portals then exited with `another
portal serves this name`. The cleanup now kills by exact name and by `--role`. The clean run has
zero such lines.

## Findings

### F-P1a: frontend did not export Screenshot (fixed)

The frontend exports a portal only when its backend lists the `Access` interface. xdp 1.16 gates
`screenshot_create` on it.

`gnoblin.portal` now lists `Access`, backed by `AccessDialog` on the `portal.ui` consent path. Live,
the frontend routes Access and Screenshot to gnoblin and exports Screenshot.

### F-P1b: fall-through interfaces (decided, routes made honest)

`portals.conf` routes Wallpaper, Background, FileChooser, AppChooser, Print, Account, Email,
Notification, Lockdown and DynamicLauncher to `gtk;kde;*`.

Gnoblin does not serve these, and the file says so. The frontend uses the fallback, so a session with
no fallback installed returns `UnknownMethod`. That is the honest result, because Gnoblin does not
claim a service it does not run.

### F-P2: anonymous callers shared one consent answer (fixed in code, live proof open)

Run 8 on a fresh instance. `wtype` returned 0 for every key.

- Pass: request 1 draws the consent dialog. Escape answers it with `response=1` (`a consent answer arrived outcome=Refused`), and the dialog closes. A caller that vanishes tears its dialog down. The backend stays up with no restart.
- Fail: request 2, from a second app, got `response=2` at once with `the user refused this request`, and no dialog was offered.

Cause. `ConsentState::ask` keyed the requester by `(bus_name, app_id)`. An unsandboxed caller
has `app_id == ""`, so every unsandboxed caller shared one key, and the 120 s refusal cache applied
to all of them at once.

Decision (Main): an empty `app_id` means no identity. The backend cannot tell unsandboxed callers
apart, and it invents no attribution source.

So an empty-`app_id` request never reuses a cached consent or refusal. Each one prompts on its own
and queues behind open dialogs. The queue is bounded. When it is full the request fails with
`Refusal::Busy` (`AtCapacity`). A non-empty `app_id` refusal is still cached for 120 s.

Fix: `consent.rs` skips the pending and gate lookups for an empty `app_id`. It tracks anonymous
requests in their own count (`anonymous_requester`, `end_anonymous`).

`iface/mod.rs` maps `Busy` to a failed response. Regression tests in `tests/failure_modes.rs`
(7, anonymous and named) were red before the fix and are green on Agent37
(`PortalConsentLive-logs/green2.log`).

Live run with the fixed build (`PortalConsentLive-logs/rd4-run1.txt`, `consent-tests-run2.txt`):
Tab+Return on the dialog arms and presses Allow. The backend logs `a consent answer arrived
outcome=Granted` for request 2, then request 3 (queued behind it) is shown and granted in turn.
Queue order holds: request 3 is not answered until request 2 has been. Anonymous callers each
prompt.

Screenshot is wired to the compositor's `capture.screen` member through `CaptureBackend` in
`crates/xdg-desktop-portal-gnoblin/src/backends.rs` and `capture_relay.rs`, so an Allow returns
`response=0` with a `file://` PNG when the shell is reachable. Not yet proven live: the later run
(`PortalConsentLive-logs/live-run2.log`) got `response=1` on Tab+Return, and the cause is open.
The 500 ms arm delay is not it, since that harness waits 5 s before its keys.

FileChooser and the fallback sysroot are unchanged.

## The consent flows, from the dialog to enforcement (lane 10C)

The portal decides, the compositor enforces. Seven owner-only IPC members (`crates/gnoblin-compositor/src/ipc/consent.rs`) carry the answer; both flows follow the same rule, **no recorded consent, nothing opens**. Remote input and capture used to stop at the dialog: `EisGate::grant` and the PipeWire export had no caller. Decision O12 (a) was auto-selected, "default selected, owner may revisit"; (b), the shell answering a `remote` signal, stays open with O3.

**RemoteDesktop.** `Start` asks capture then input injection through `portal.ui` (today the compositor's `Modal` dialog). A yes calls `remote.grant {session, devices}`, which records the approval and lets the EIS endpoint advertise exactly those devices to that session. The user's own no calls `remote.refuse`; an expired prompt or a caller that left decides nothing. `ConnectToEIS` calls `remote.connect`, and the compositor opens a one-shot `0600` socket for that session (`eis-<n>.sock` beside `eis.sock`, removed when the first client connects or the session ends). The portal connects to it and returns the connected stream as the app's descriptor. The IPC transport carries no file descriptors, which is why the path is returned rather than a descriptor. The compositor refuses `remote.connect` with `no-consent` or `consent-refused` for a session it was not told about, whatever the portal believes. `Close`, or the caller leaving the bus, calls `remote.end`: the connection ends after the seat releases every held key, button and contact. The answer for one session never reaches another session or `eis.sock`, which stays default deny (the test harness grants it).

**ScreenCast.** `Start` asks for capture. A yes calls `cast.grant {session, output}` (`output` is `OUTPUT_FIRST` when the user chose none) and `cast.start`, which opens the only `CaptureStream::Granted` session in the capture table and starts its PipeWire stream (`State::start_cast`); the reply carries the node the portal returns. `cast.start` for a session with no recorded consent is refused `no-consent`. A locked session, a missing output and a missing PipeWire daemon are refused by name (`locked`, `no-output`, `unavailable`). The stream ends with `cast.end`, `Close`, or the connection that held the consent.

**ScreenCast of a window.** `cast.grant {session, window}` (source tag 2) is refused `no-consent` unless a `portal.ui` request of the same connection carried the `window` option, named the window in the question (`PortalService::name_window`, which also refuses `no-window` for a window the compositor does not show) and was answered yes; that answer is spent by the one grant (`take_window_consent`), and a lock or the connection leaving withdraws it. `cast.start` then streams the window's rectangle (`no-window` if it is gone) and answers its layout place and extent beside the stream's pixel size (`CastStream.x`, `y`, `layout_width`, `layout_height`, `width`, `height`). The portal puts these in `position` and `size`.

**What is proven where.** End to end over real sockets: `crates/gnoblin-compositor/tests/eis.rs` (IPC, a real libei client, a real Wayland client: seat capabilities, absolute pointer motion, keys, refusal, end, holder gone) and `tests/cast.rs` (a private PipeWire daemon, `pw-dump`: no node without consent, node after, gone after end or holder loss). And, from the D-Bus call down: `crates/gnoblin-compositor/tests/portal_e2e.rs` serves the portal's real service body (`xdg_desktop_portal_gnoblin::serve::serve`, the body of `main`) on a private `dbus-daemon` bus, against a live headless compositor over its owner IPC socket. The caller is a second bus connection calling `CreateSession`, `SelectDevices`/`SelectSources`, `Start`, `ConnectToEIS` and `Session.Close`; the consent dialog is the compositor's own, answered by keys sent over the `input` role (Tab then Enter allows once armed, Escape denies; the test knows a dialog is open because a counting transport under the portal's IPC connection sees its `portal.ui` request, as the compositor accepts a key with no dialog open and does nothing with it). Proven there: an approved `RemoteDesktop.Start` (two dialogs) answers 0 and `ConnectToEIS` hands a descriptor on which a real libei client binds the devices, moves a Wayland client's pointer and types; a denied one answers 1 (the user cancelled), asks no second dialog, opens no descriptor and leaves a recorded refusal in the compositor; `Session.Close` and the caller leaving the bus end the compositor session and disconnect the EI client; an approved `ScreenCast.Start` answers a PipeWire node a private daemon holds, and `Close` or the caller leaving removes it; a denied one answers 1 and makes no node; the same for a View and for a window the app named (an approved window share answers the node with `position` and `size`; a window that is not there is cancelled and makes no node); `InputCapture.Start` asks, then answers `NotSupported` and the compositor holds nothing. Not run end to end: the real `xdg-desktop-portal` frontend in front of the backend (see F-P1), the packaged binary's own `main` (its body is the shared `serve`), and the frames a stream carries (`gnoblin-pipewire-export` tests and `gnoblin-wayland`'s `view_capture_e2e`, which read a window's frames on a real daemon). The lock deadlock below has no deterministic test: the approved `RemoteDesktop` test failed or hung in about four runs in ten before the fix and passed 15 of 15 after it; a scripted-transport test that holds the IPC thread in a read while a handler offers is the missing deterministic one. Not served: `InputCapture` (needs EIS receiver contexts) and a live update of a stream's `position` (the portal API has none).

**Bugs the end to end test found.** (1) The backend exported `ConnectToEis`, not the spec's `ConnectToEIS`, on `RemoteDesktop` and `InputCapture`, so a frontend's call was `UnknownMethod`. (2) Every `version` property was exported as `Version`; the `org.freedesktop.impl.portal.*` interfaces name it `version`. (3) The IPC thread waited for the portal's state lock while a D-Bus handler held that lock across a consent offer only that thread can send: when a graph event followed a dialog's answer, the second question of `RemoteDesktop.Start` was never offered and `Start` never returned (about four runs in ten). The thread now only tries the lock and queues what it read (`IpcLoop::apply_deferred`).

## Next steps

1. Run the consent path once in a session with a person or harness answering the dialog, to prove
   the picture is returned.
2. Install a `gtk` or `kde` fallback in the test sysroot, to prove FileChooser fall-through end to end.
