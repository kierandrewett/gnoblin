# Portal end-to-end results

Live run on Agent37 (HEAD `d31a8ddd7`, debug build). The gnoblin session ran on a private
session bus with `xdg-desktop-portal` (1.16) as the frontend and `xdg-desktop-portal-gnoblin`
as the backend. Harness: `run.sh` in the PortalE2E lane logs, plus `backend_call.py` to call
the backend directly.

## Results

| Call                                               | Route           | Result                                                                                                                                                 |
| -------------------------------------------------- | --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `Settings.ReadAll(["org.freedesktop.appearance"])` | frontend        | PASS with a note: `{}`. The session sets no appearance value, so the backend returns no key. Its direct `Read color-scheme` answers `UnknownProperty`. |
| `Screenshot.Screenshot`                            | frontend        | FAIL: the frontend never exports `org.freedesktop.portal.Screenshot`.                                                                                  |
| `Screenshot.Screenshot`                            | backend, direct | Answers synchronously `(1, {})`. Response 1 is cancelled, because consent is refused (F-P2).                                                           |
| `Secret.RetrieveSecret`                            | frontend        | Not exported. Expected by design: `portals.conf` refuses Secret and `design/keyring.md` says so.                                                       |
| `FileChooser.OpenFile`                             | frontend        | FAIL: `UnknownMethod`. The frontend does not export FileChooser.                                                                                       |

## Findings

### F-P1: frontend exports fewer portals than `portals.conf` routes

- **F-P1a (confirmed live and in source).** Screenshot is routed to gnoblin in `portals.conf:49`,
  but the frontend does not export it. Upstream xdp 1.16 `src/xdg-desktop-portal.c:288` gates
  `screenshot_create` on `find_portal_implementation("org.freedesktop.impl.portal.Access") != NULL`.
  `gnoblin.portal` lists 8 interfaces and no `Access`, so the frontend never exports Screenshot.
  The frontend log confirms `Using gnoblin.portal for ...Screenshot` and then no export.
  Fix candidate: add `org.freedesktop.impl.portal.Access` to `gnoblin.portal`, backed by a real
  implementation or a deliberate `Access` stub, then re-run this scenario.
- **F-P1b (confirmed live, by design per the `portals.conf` comment).** `gnoblin.portal` omits
  FileChooser, AppChooser, Print, Wallpaper, Background, Account, Email, Notification, Lockdown
  and DynamicLauncher, though `portals.conf` routes them to gnoblin. Those fall through, so
  FileChooser returns `UnknownMethod`. Decide whether each should be listed, or whether the
  routes should be removed.

### F-P2: the portal backend never gets a privileged connection, so consent can never succeed (confirmed live)

- Live: the backend asks `portal.ui` for consent and the compositor refuses:
  `portal.ui needs the portal-ui capability, which this connection does not hold`. The answer
  is `Refused`, and the Screenshot request ends with response 1.
- Source: `crates/gnoblin-session/src/supervisor.rs:504` grants a privileged socket only to
  `runtime` and `services`. `crates/gnoblin-compositor/src/ipc/privileged.rs:90` grants
  `portal` PortalUi and RetrieveSecret, but no `portal` role is ever started with a socket.
  `crates/xdg-desktop-portal-gnoblin/src/host.rs:3-5` states that a bus activation has no
  privileged connection and `NoShell` takes its place.
- Lua cannot repair this. The runtime role (`privileged.rs`) has no `PortalUi`. The wire
  `SLOT_CLAIM` is ungated in `server.rs`, but no Lua, CLI or `gnoblinctl` binding calls it. So a
  Lua `portal.ui` handler cannot claim the slot.
- **Design call, not fixed here.** Granting `PortalUi` to the backend would let the process that
  asks for consent also answer it. The consent UI must be a separate process holding `PortalUi`,
  started by the services role. That changes the supervisor plan and the portal-UI wiring, so it
  is recorded as a finding and not patched.

### F-P3: Screenshot and FileChooser are unreachable, not wrong

With F-P1 and F-P2 in place, Screenshot cannot succeed on this session, and FileChooser is not
routed. Reachability is the blocker. Behaviour inside the backend is not yet proved past the
consent refusal.

## Observations (not verified as bugs)

- Appearance: the backend publishes `color-scheme`, `contrast` and `reduced-motion`, but a
  session with no appearance config returns no key. The spec's default ("no preference") is not
  synthesised. Confirm whether the compositor should always publish a default.
- The harness `call.py` screenshot path raised a `dbus` `TypeError` on a non-string argument.
  The direct `backend_call.py` path is the working one.

## Next steps

1. Decide F-P2 (separate consent UI process started by services, or another owner of `PortalUi`).
2. Decide F-P1a (add `Access` to `gnoblin.portal`) and F-P1b (list or unroute the fall-through interfaces).
3. Re-run the scenario after each decision.
