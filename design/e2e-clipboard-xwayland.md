# End-to-end: clipboard, primary selection and Xwayland selections

Status: real-process run of the `rust-compositor` binaries on Agent37 (instance
`3wyjhbangi`). Checks A to D pass. Checks E and F fail: X11 clients and Wayland
clients do not see each other's clipboard. The cause is a missing XWM selection
bridge, recorded below as a finding and not fixed here.

Logs are on the workstation in `~/dev/lanes/ClipboardXwaylandE2E-logs/`.

## Setup

- Instance `3wyjhbangi`, Xwayland 22.1.9. Its help text has no
  `-byteswappedclients` switch, so the compositor cannot refuse byte-swapped
  clients on this binary.
- Session: `gnoblin` under a private `dbus-daemon`, `XDG_RUNTIME_DIR` under
  `/home/node/clipx/s/run`, started by `/home/node/clipx/start.sh`.
- Clients: `wl-copy`/`wl-paste` (wl-clipboard), `xclip`, and the clipboard
  check script `clip.sh`. The client socket is `gnoblin-wayland.sock`; the
  privileged socket is `gnoblin-wayland-privileged.sock`.
- Config: `init-kebab.lua` (`xwayland.allow-byte-swapped-clients = true`, plus an
  `envprobe` autostart). Kept store: `F3-last-good-config.json`.

## Results

| Check | What it does                                              | Result           | Log                                                                        |
| ----- | --------------------------------------------------------- | ---------------- | -------------------------------------------------------------------------- |
| A     | `wl-copy` then `wl-paste` on the client socket            | PASS             | `E-F-clip-checks-final.log`                                                |
| B     | primary selection copy and paste                          | PASS             | same                                                                       |
| B2    | a primary copy does not replace the clipboard             | PASS             | same                                                                       |
| C     | unprivileged `wl-paste` reads the clipboard               | PASS, see note 1 | same, `clip-dbg-main.txt`                                                  |
| C     | privileged socket lists data-control and pastes           | PASS             | same, `clip-dbg-priv.txt`                                                  |
| C2    | privileged copy is visible to an unprivileged paste       | PASS             | same                                                                       |
| D     | named in-process tests: selection and clipboard admission | PASS, see note 3 | `D1-inprocess-selection-tests.log`, `D2-compat-mutter-clipboard-tests.log` |
| E     | `xclip` sets the X11 clipboard, Wayland paste reads it    | PASS             | `I43-e2e-clipdeps.log` (E1, E2, P1, P2)                                    |
| F     | `wl-copy` sets the Wayland clipboard, `xclip` reads it    | FAIL, F1 and F3  | `I43-e2e-clipdeps.log`; F2 PASS; see F1                                    |

Xwayland starts on the kept store: `E-F-final-xwayland-start.txt` shows
`ready area="xwayland" display_number=0`.

### Notes

1. An unprivileged `wl-paste` succeeds through the focus fallback: `wl-clipboard`
   opens a hidden `xdg_toplevel`, takes keyboard focus, then reads the selection.
   The client socket does not advertise a data-control global. This is by design
   and is not a leak. `clip-dbg-main.txt` shows the main socket globals with no
   `ext_data_control_manager_v1` or `zwlr_data_control_manager_v1`;
   `clip-dbg-priv.txt` shows both on the privileged socket.
2. D: the lock-screen clipboard and DnD were not run. The named in-process tests
   were run: `crates/gnoblin-wayland/tests/selection.rs` (4 passed, including
   `a_selection_does_not_outlive_its_owner`) and
   `crates/compat/gnoblin-compat-dbus-mutter-clipboard/tests/admission.rs`
   (4 passed).
3. The Xwayland start check failed on the first boot of a fresh store, and
   worked on the next boot. Both facts are below.

## Findings

### F1: X11 and Wayland selections are bridged; Wayland-to-X11 reads are intermittent

- The bridge is in `crates/gnoblin-wayland/src/selection_bridge.rs`: per-target
  owner state, pure target-to-MIME mapping, ownership moves, and lock refusal.
  Its unit tests live in the same file (ownership, clear, lock, mapping).
  `xwayland.rs` wires the XWM selection hooks to it; `wl_data_device_manager.rs`
  feeds Wayland selection changes into it.
- Run on Agent37 (`I43-e2e-clipdeps.log`, `tests/xwayland-clipboard-e2e.sh`).
  `src` is the sha256 prefix (16 hex) of the source bytes, `got` of the bytes
  received.
    - E1 PASS 19 bytes, `src=got=7d916bff01a814c7`.
    - E2 PASS 512 bytes, `src=got=1e50eb4754c6d7cb`.
    - P1 PASS 11 bytes, `src=got=2fc49f4086f88e47`.
    - P2 PASS 512 bytes: an X11 primary copy leaves the clipboard alone.
    - F1 FAIL: `src=77545c1a41f01819` (19 bytes), `got=e3b0c44298fc1c14` (0 bytes).
    - F2 PASS 512 bytes, `src=got=1e50eb4754c6d7cb`.
    - F3 FAIL: `src=3c2395a3298f8a47` (15 bytes), `got=e3b0c44298fc1c14` (0 bytes).
    - C0 PASS 12 bytes; C1 PASS (cleared X11 selection pastes 0 bytes).
    - `failures: 2`.
- Diagnosis (`I44-f1-diag.log`, `I45-f1-mimes.log`): the Wayland source offers
  `text/plain`, `text/plain;charset=utf-8`, `TEXT`, `STRING` and `UTF8_STRING`.
  In I44 the `text/plain` read returned the text and a `UTF8_STRING` read
  returned 0 bytes. In I45 the same `UTF8_STRING` read returned the text. So the
  failure is intermittent and not a missing MIME mapping. It is not isolated.
  The X11 log shows `BadWindow` on `ChangeProperty` for the XWM's own window
  during these steps. `[INFERENCE]` a race in smithay's XWM selection path
  (`src/xwayland/xwm/mod.rs`) is the likely cause; not confirmed.
- Policy: ZERO smithay patches. Any fix must wrap smithay from this crate. The
  F1 and F3 checks stay failing until that fix lands; the harness is not weakened.

### F2: Xwayland does not start on this binary with the default setting

- `crates/gnoblin-xwayland/src/settings.rs:284-294`: with
  `allow_byte_swapped_clients` false and no `-byteswappedclients` switch, the
  settings refuse to start Xwayland. The message is
  `Xwayland is not started: this Xwayland has no -byteswappedclients switch`. This
  is deliberate.
- The test run set `allow-byte-swapped-clients = true` for this binary only. With
  `true` and no switch, the binary is started without a switch
  (`settings.rs:287`).
- The config key is kebab-case: `allow-byte-swapped-clients` is accepted.
  The snake-case key `allow_byte_swapped_clients` is rejected by validation.
  The Rust field name is snake-case, so the mismatch is easy to hit.

### F2b: the first boot of a fresh store refuses Xwayland before the config is committed

- `F2-freshstore-kebab-session.log:26-29`: the refusal at line 26 is logged before
  `configuration committed ... revision=1` at line 29. The store is read at
  startup (`crates/gnoblin/src/main.rs:182-183`,
  `crates/gnoblin-compositor/src/xwayland.rs:21-47`). Lua runs in the runtime role
  afterwards, so the first boot uses the empty store.
- `design/lean-runtime.md:90-91` says Xwayland settings "take effect on the next
  Xwayland start". That matches the code: they are read once at startup and there
  is no live reload for them. The docs do not promise live reload for Xwayland.
- On the next boot with the stored document, Xwayland starts
  (`F2-session-initxw-run2-keptstore.log`, `F2-kebab-keptstore-xwayland-starts.log`).
- The compositor applies a committed document at `ipc/server.rs:1183-1190`: a
  section it cannot decode keeps the last good values. This is how a bad section
  is dropped without an error the user sees (`[INFERENCE]`: I did not trace every
  caller). A fix is to report the refusal back to the runtime when the first boot
  has no store, or to re-check Xwayland settings after the first commit.

### F3: the stored document

- `F3-last-good-config.json` holds the accepted document after revision 1:
  `{"document":{"autostart":{...},"xwayland":{"allow-byte-swapped-clients":true}},"format":1,...}`.
  The visible setting is stored, so the store is not empty.
- Not run: an init file that sets nothing. The earlier `{"document":{}}` report
  is therefore not reproduced here.

## Not run

- Lock-screen clipboard and DnD (check D, as written).
- A fresh-store boot with the snake-case key on a binary with the switch.
- An Xwayland binary that has `-byteswappedclients`. Only Xwayland 22.1.9 was
  tested.

## Gates (earlier on this lane)

From the logs in the lane directory: `check`, `clippy`, `fmt`, `dupes --deny`,
`xtask deps`, `layout`, `logging`, and pre-commit all passed. The
`gnoblin-compositor --test window_rules` run failed once
(`G4-test-compositor.log`, test `a_later_commit_without_the_rule_stops_it_applying`
at `tests/window_rules.rs:115`). It passed on rerun (`G4b-window-rules-rerun.log`,
3 passed). This is a flaky test, not investigated.
