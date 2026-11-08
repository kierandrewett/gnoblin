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

## Next steps

1. Run the consent path once in a session with a person or harness answering the dialog, to prove
   the picture is returned.
2. Install a `gtk` or `kde` fallback in the test sysroot, to prove FileChooser fall-through end to end.
