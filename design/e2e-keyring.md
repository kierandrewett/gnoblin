# End-to-end keyring unlock: run record

Status: the store, lookup, search and clear flow runs end to end on the built-in
keyring. A first boot with `keyring.enabled = false` now serves nothing; this was
open as F1 and is fixed in the F1 commit (see Findings). The F1 fix covers the
services role only. The Secret portal still reads the last accepted file; see
F1 residual.

The services stall recorded in the earlier blocked run is fixed; see
`design/e2e-smoke.md`, "Services stall". Every run below reached `session ready`.

Build for the F1 runs: `rust-compositor` at the F1 fix commit, pushed as
`ae2e4fde8` and rebased on `b12a2472b`.

Built on Agent37 with `a37-gate.sh`. The instance was `we2xo5p4gy`, deleted at the
end of the run.
Scenario logs for the F1 runs: `~/dev/lanes/KeyringE2E-logs/f1/`,
named `<scenario>.<file>`.

Earlier runs (before the F1 fix, build `93152886e`, instance `7sgzvn3ki4`) are in
`~/dev/lanes/KeyringE2E-logs/rerun/`.

## Method

- Session: the guardian `gnoblin` with the headless backend, as in
  `design/e2e-smoke.md`. Runtime dir `/home/node/e2e/run`.
- Per-scenario config under `/home/node/e2e/<scenario>/config/gnoblin/init.lua`.
  `off` sets `keyring = { enabled = false }`. The other scenarios set
  `enabled = true`.
- A claimant process answers `prompt.secret` with the login password. The
  wrong-password case answers with a different password.
- Client: `secret-tool` built from `/home/node/e2e-src/secret-tool` (DH
  algorithm, the same one libsecret uses by default).
- Leak check: the secret value and both passwords are grepped in session logs,
  claimant log, the tool outputs and the data directory. All counts were 0.
- Harness: `/home/node/e2e-src/run.sh SCENARIO`, with `REUSE_DATA_FROM=<s>`,
  which starts from another scenario's data and state dirs. The harness is not
  committed.

## Results

F1 runs (build with the F1 fix):

| Scenario                               | Check                                                                                                                                                                                   | Result                            | Evidence                                                                        |
| -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------- | ------------------------------------------------------------------------------- |
| off, first boot (empty state)          | log: `keyring.enabled decided enabled=false source=accepted revision 1`; no owner of `org.freedesktop.secrets`; `store` rc=1 `ServiceUnknown`; keyring db count 0                       | PASS (F1 fixed)                   | `off.session.log`, `off.owner.txt`, `off.store.err`, `off.keyring-db-count.txt` |
| main, first boot, `enabled = true`     | `enabled=true source=accepted revision 1`; `serving the Secret Service`; file created; `store` rc=0, `lookup` rc=0, value matches, `search` rc=0, `clear` rc=0, lookup after clear rc=1 | PASS                              | `main.session.log`, `main.steps.txt`, `main.keyring-db-count.txt`               |
| main, leak check                       | secret value and both passwords: 0 matches                                                                                                                                              | PASS                              | `main.leak-check.txt`                                                           |
| wrongpw, `REUSE_DATA_FROM=main`        | `password from the prompt did not unlock the keyring reason=WrongKey`; `store` rc=1; `lookup` rc=1                                                                                      | PASS (fails closed)               | `wrongpw.session.log`, `wrongpw.store.err`                                      |
| nostore, `enabled = true`, no claimant | `serving the Secret Service`; owner present; no password asked; 20 s hold                                                                                                               | PASS (serves, waits for a prompt) | `nostore.session.log`, `nostore.owner.txt`                                      |

Runs from before the F1 fix (build `93152886e`):

| Scenario                                 | Check                                                                                          | Result                                                              | Evidence                                         |
| ---------------------------------------- | ---------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- | ------------------------------------------------ |
| main                                     | session ready, name owned, `store`/`lookup`/`search`/`clear` as above                          | PASS                                                                | `rerun/main/steps.txt`, `rerun/main/session.log` |
| main                                     | owner pid is the services role                                                                 | PASS (after harness fix)                                            | `rerun/main/owner.txt`                           |
| wrongpw, fresh data dir                  | `store` rc=0                                                                                   | NOT A WRONG-PASSWORD TEST: the wrong password created a new keyring | `rerun/wrongpw` (first attempt)                  |
| wrongpw, `REUSE_DATA_FROM=main`          | `reason=WrongKey`, `store` rc=1, `lookup` rc=1                                                 | PASS (fails closed)                                                 | `rerun/wrongpw/session.log`, `store.err`         |
| nohandler                                | no prompt claimant: `store` rc=1 `prompt was dismissed`, `lookup` rc=1 `no matching item`      | PASS (fails closed)                                                 | `rerun/nohandler/store.err`, `lookup.err`        |
| off, first boot                          | `keyring.enabled=false` ignored: keyring served, file created, `store` rc=0                    | FAIL (F1, before the fix)                                           | `rerun/off-first/session.log`                    |
| off, second boot (state from first boot) | no keyring log lines, `store` rc=1 `ServiceUnknown`, bus tree has no `org.freedesktop.secrets` | PASS                                                                | `rerun/off/session.log`, `rerun/off/store.err`   |

## Findings

- **F1 (fixed in the F1 commit, residual noted): `keyring.enabled = false` was
  ignored on first boot.**
    - Before the fix, the services role read the flag from `last-good-config.json`
      at start. The compositor saved the commit only after services started and
      reported ready. On a first boot the file did not exist, so the default (on)
      applied.
    - The fix: the services role waits for the compositor to accept this session's
      configuration (`CONFIGURATION_WAIT` = 20 s, poll `CONFIGURATION_POLL` = 100 ms
      in `crates/gnoblin-services/src/keyring.rs`) and decides from the accepted
      document. If nothing is accepted in time it uses the last accepted file, then
      the schema default.
    - Proof: `off.session.log` shows `enabled=false source=accepted revision 1`, no
      owner of `org.freedesktop.secrets`, no keyring db file, and `store` fails with
      `ServiceUnknown`. `main.session.log` shows the enabled first boot serving
      after revision 1 is accepted.
    - Residual, not fixed: the Secret portal (`xdg-desktop-portal-gnoblin`,
      `main.rs:115`) reads the last accepted file on start, not the accepted
      document. On a first boot with no saved file it assumes the default (on) and
      does not follow an unsaved `enabled = false`. This is a separate change.
    - Not run: the 20-second timeout path (no configuration ever accepted). It needs
      a runtime that never accepts a document, which this harness does not provide.
- **F2 (harness, fixed): owner pid parse.** The owner pid was parsed from
  `GetConnectionUnixProcessID`'s reply. The reply is `uint32 N`, so the digits
  in the type name were kept. The harness now takes the number after
  `uint32`. Earlier runs showed a wrong pid and are not used.
- **F3 (fixed, 93152886e): keyring directory missing on first create.** A
  first `store` failed with `ENOENT` because the keyring directory did not
  exist. The fix creates the directory before the first file. The `main` run
  above is the proof.

## Not verified

- Clean-profile `SearchItems` on a locked collection (the limit from the
  brief). The `main` run unlocks before search, so this was not exercised.
- `prompt.secret` with a claimant that never answers. The `nohandler` run
  covers the no-claimant case only.
- The 20-second configuration timeout (see F1).
- The Secret portal's behaviour on first boot with `enabled = false` (see F1
  residual).
- Real DRM, PAM, a real login and gnome-keyring interop. Headless backend only.
