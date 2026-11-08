# End-to-end keyring unlock: run record

Status: the store, lookup, search and clear flow runs end to end on the built-in
keyring. Two results are open, see Findings: a first boot with
`keyring.enabled = false` still serves the keyring, and the Secret Service
prompt wrong-password path has only been shown through the existing-keyring
variant.

The services stall recorded in the earlier blocked run is fixed; see
`design/e2e-smoke.md`, "Services stall". Every run below reached `session ready`.

Build: `rust-compositor` at `93152886e` (lane HEAD), built on Agent37 with
`a37-gate.sh`. Agent37 instance `7sgzvn3ki4`, deleted at the end of the run.
Scenario logs are in `~/dev/lanes/KeyringE2E-logs/rerun/`.

## Method

- Session: the guardian `gnoblin` with the headless backend, as in
  `design/e2e-smoke.md`. Runtime dir `/home/node/e2e/run`.
- Per-scenario config under `/home/node/e2e/<scenario>/config/gnoblin/init.lua`.
  `main` has no keyring setting. `off` sets `keyring = { enabled = false }`.
- A claimant process answers `prompt.secret` with the login password. The
  wrong-password case answers with a different password.
- Client: `secret-tool` built from `/home/node/e2e-src/secret-tool` (DH
  algorithm, the same one libsecret uses by default).
- Leak check: the secret value and both passwords are grepped in session logs,
  claimant log, the tool outputs and the data directory. All counts were 0.
- Harness: `.e2e/run.sh SCENARIO`, with two switches. `REUSE_DATA_FROM=<s>`
  starts from another scenario's data and state dirs. The owner-pid parse was
  fixed in this run. The harness is not committed.

## Results

| Scenario                                 | Check                                                                                                                                                | Result                                                              | Evidence                                         |
| ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- | ------------------------------------------------ |
| main                                     | session ready, name owned, `store` rc=0, `lookup` rc=0, value matches, `search` rc=0, `clear` rc=0, lookup after clear rc=1                          | PASS                                                                | `rerun/main/steps.txt`, `rerun/main/session.log` |
| main                                     | owner pid is the services role                                                                                                                       | PASS (after harness fix)                                            | `rerun/main/owner.txt`                           |
| main                                     | leak check                                                                                                                                           | PASS, 0 matches                                                     | `rerun/main/leak-check.txt`                      |
| wrongpw, fresh data dir                  | `store` rc=0                                                                                                                                         | NOT A WRONG-PASSWORD TEST: the wrong password created a new keyring | `rerun/wrongpw` (first attempt)                  |
| wrongpw, `REUSE_DATA_FROM=main`          | session log: `password from the prompt did not unlock the keyring reason=WrongKey`, `store` rc=1 with `Error: "prompt was dismissed"`, `lookup` rc=1 | PASS (fails closed)                                                 | `rerun/wrongpw/session.log`, `store.err`         |
| wrongpw                                  | leak check                                                                                                                                           | PASS, 0 matches                                                     | `rerun/wrongpw/leak-check.txt`                   |
| nohandler                                | no prompt claimant: `store` rc=1 `Error: "prompt was dismissed"`, `lookup` rc=1 `no matching item`                                                   | PASS (fails closed)                                                 | `rerun/nohandler/store.err`, `lookup.err`        |
| off, first boot                          | `keyring.enabled=false` ignored: keyring served, file created, store rc=0                                                                            | FAIL, see Finding F1                                                | `rerun/off-first/session.log`                    |
| off, second boot (state from first boot) | no keyring log lines, `store` rc=1 `ServiceUnknown`, bus tree has no `org.freedesktop.secrets`                                                       | PASS                                                                | `rerun/off/session.log`, `rerun/off/store.err`   |

The `off` evidence in `rerun/off` is the second boot. `rerun/off-first` is the
first boot. `wrongpw` loads the wrong password file; `nohandler` starts no
claimant at all.

## Findings

- **F1 (open): `keyring.enabled = false` is ignored on first boot.**
    - The services role reads the flag from `last-good-config.json` at start.
    - The compositor saves the commit only after services start and report ready.
    - On a first boot the file does not exist, so the default (on) applies.
    - Evidence, `rerun/off-first/session.log`: services serve `org.freedesktop.secrets` and create a keyring file before `configuration revision 1 accepted`.
    - On a second boot from the saved state the setting is honoured (`rerun/off`).
    - Fix options, not chosen here: make services wait for the first commit, or save the commit before services start. This is a design question, so it was not changed here.
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
- Real DRM, PAM, a real login and gnome-keyring interop. Headless backend only.
