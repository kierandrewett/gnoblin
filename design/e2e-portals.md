# Portal end-to-end results

Live runs on Agent37 with a debug build of `rust-compositor`. The gnoblin session ran on a private
session bus with `xdg-desktop-portal` 1.16 as the frontend.

The guardian started `xdg-desktop-portal-gnoblin` as its `portal` child. That child holds the
privileged pair.

The harness is `e2e-live/live.sh guardian` in the PortalRole lane (not committed). Logs:
`PortalStart-logs/live2.txt`.

## Results

| Check                                                      | Route           | Result                                                                                                                                        |
| ---------------------------------------------------------- | --------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| Guardian starts the backend as its child                   | session         | PASS. The portal's parent pid is the guardian. The log has `started portal`.                                                                  |
| Backend owns `org.freedesktop.impl.portal.desktop.gnoblin` | session         | PASS.                                                                                                                                         |
| Backend killed, guardian restarts it                       | session         | PASS. New pid, same guardian parent, restart after 250 ms (attempt 1 of 5).                                                                   |
| Backend reaches `portal.ui` with its capability            | backend         | PASS. Two consent requests reached `portal.ui`. No `needs the portal-ui capability` refusal appears.                                          |
| Frontend exports Screenshot (F-P1a)                        | frontend        | PASS. The frontend routes Screenshot to gnoblin, then provides `org.freedesktop.portal.Screenshot`. The call returns a request handle (rc 0). |
| Frontend Screenshot returns a result                       | frontend        | NOT PROVEN (F-P2). Consent waits for a UI answer. Headless nobody answers, so the request ends with `response=2` when the caller leaves.      |
| Direct backend `Screenshot`, `AccessDialog`                | backend, direct | Block until the caller's timeout (rc 124). This is consent waiting for an answer, which is correct.                                           |
| `FileChooser.OpenFile`                                     | frontend        | Not re-run in this round. Routed to `gtk;kde` (F-P1b), so with no fallback installed it returns `UnknownMethod`.                              |
| `Secret.RetrieveSecret`                                    | frontend        | Not exported. Expected: `portals.conf` refuses Secret and `design/keyring.md` says so.                                                        |

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

### F-P2: consent cannot finish headless (open, expected)

Before this change, the backend was refused: `portal.ui needs the portal-ui capability`. The guardian
now starts the backend as a privileged child, so it reaches `portal.ui`.

The request then waits for a person to answer the compositor's consent dialog. A headless run has
nobody to answer, so no picture is taken. Consent is working, and the run is not a success. A real
success needs one answer from a UI session, which this harness cannot give.

## Next steps

1. Run the consent path once in a session with a person or harness answering the dialog, to prove
   the picture is returned.
2. Install a `gtk` or `kde` fallback in the test sysroot, to prove FileChooser fall-through end to end.
