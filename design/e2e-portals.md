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

### F-P2: the second request from an unsandboxed caller is refused with no dialog (open)

Run 8 on a fresh instance. `wtype` returned 0 for every key.

- Pass: request 1 draws the consent dialog. Escape answers it with `response=1` (`a consent answer arrived outcome=Refused`), and the dialog closes. A caller that vanishes tears its dialog down. The backend stays up with no restart.
- Fail: request 2, from a second app, gets `response=2` at once with `the user refused this request`. No `asked portal.ui for consent` line follows, so no dialog is offered.

Cause, confirmed in code. `ConsentState::ask` (`consent.rs`) keys the authority requester by the pair:

- `bus_name` and `app_id`, which is the key `ask()` passes to `gate()`.
- `Refusal::Refused` comes only from `gate()` at consent.rs:711, when that requester's grant is `Consent::Refused`.
- A new caller gets `NoGrant` and prompts. So request 2 maps to request 1's requester.

- `bus_name` is `sender(header)` (`iface/mod.rs`). [INFERENCE] From the requester mapping, the name is the same for both calls. The likely sender is the frontend's connection, but this is not logged; confirm by logging the sender before fixing.
- `app_id` is `""` for both. The frontend passes it as empty for unsandboxed apps, and the log shows `app=`.
- So the backend has no per-app identity for an unsandboxed caller. The 120 s refusal cache (`CONSENT_LIFETIME`, `main.rs`) is meant to hold for one caller. Here it holds for every unsandboxed caller at once.

The fix is a design decision, not a harness change: how an unsandboxed caller is identified. The design says identity comes from compositor attribution by PID (design/rust-compositor.md, about line 2538). Not implemented here, because it needs a choice of the attribution source. Escalated to Main.

Still open: two queued requests answered in order, a Return/Tab+Return Allow with PNG, and the vanished-caller/teardown check after a grant. The last is partly proven (vanished caller tears down). Allow with PNG is blocked by this identity bug.

FileChooser and the fallback sysroot are unchanged.

## Next steps

1. Run the consent path once in a session with a person or harness answering the dialog, to prove
   the picture is returned.
2. Install a `gtk` or `kde` fallback in the test sysroot, to prove FileChooser fall-through end to end.
