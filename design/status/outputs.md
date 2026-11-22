# Outputs status

Checked against the code on `rust-compositor` (base `bffed8c45`). Each row cites
the code or test that backs it. Status words:

- `done`: implemented and covered by a test that runs without hardware.
- `partial`: some of the path exists; the rest is named below.
- `absent`: no code path.
- `needs hardware`: the remaining behaviour can only be checked on real DRM
  hardware. The row names the test to run.

## Summary

| #   | Claim                                 | Status                                 | What exists                                                                                                                        |
| --- | ------------------------------------- | -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| 1   | VRR is absent                         | partial                                | Model, plan and DRM op exist. Nothing sets VRR outside tests; no config name.                                                      |
| 2   | Transactional apply with rollback     | done / partial                         | Live rollback is done. Confirm timeout is model-only, not wired live.                                                              |
| 3   | DRM/KMS hotplug is only headless      | partial                                | Pure hotplug planning and its tests, plus the live loop that applies it. Live unplug and replug are not exercised on real DRM.     |
| 4   | Output power and gamma are unverified | done (protocol), needs hardware (live) | Gamma and power protocol behaviour is tested. Gamma reaches the CRTC through the backend. A live ramp on real DRM is not verified. |
| 5   | Multi-GPU is unknown                  | partial                                | Primary GPU selection is tested. Outputs on secondary GPUs are never driven: `additional()` has no caller outside its own test.    |

## 1. VRR

What exists:

- `LogicalMonitor.vrr: bool`, `crates/gnoblin-output/src/layout.rs:91`.
- Plan: `vrr: monitor.vrr && connector.vrr == VrrSupport::Supported`,
  `crates/gnoblin-backend-drm/src/plan.rs:385`. VRR change is emitted as
  `Op::Vrr`, `plan.rs:447-448`.
- DRM op: `Op::Vrr(enabled)` calls `surface.use_vrr(enabled)`,
  `crates/gnoblin-backend-drm/src/gpu.rs:349`.
- Capability probe: `gpu.rs:950-958`. `RequiresModeset` and `NotSupported` map to
  `VrrSupport::Unsupported`, with the comment "Enabling VRR on this panel needs a
  modeset the apply path does not do yet."
- Tests: `plan.rs:638` `a_vrr_request_on_an_incapable_panel_is_downgraded`,
  `plan.rs:650` `a_capable_panel_gets_the_vrr_it_asked_for`.
- Wayland: `zwlr_output_management` `adaptive_sync = true` is refused with
  `"<output>: adaptive sync is not supported"`, `crates/gnoblin-wayland/src/output_config.rs:178-179`.

What is missing:

- No config key. `gnoblin-config/schema/config.toml` and `docs/` define no `vrr`,
  `adaptive_sync` or `variable refresh` name, so none was invented.
- Nothing outside tests sets `LogicalMonitor.vrr = true`. The Lua and config paths
  cannot reach VRR today.
- A capable-by-kernel panel that needs a modeset is downgraded silently. That is
  the documented design at `plan.rs:50`, not a bug. Changing it to a named refusal
  is a design decision.

Run: `cargo test -p gnoblin-backend-drm vrr`.
Needs hardware: enable VRR on a panel that reports `VRR_CAPABLE` and read back the
connector's `VRR_ENABLED` property after apply.

## 2. Transactional apply with rollback

What exists:

- Live Wayland apply: `BackendOutputConfig::apply`, `output_config.rs:159`, calls
  `apply_in_order` at `output_config.rs:213`.
- `apply_in_order` rolls back the earlier outputs in reverse order, and
  `ApplyFailure` names the failing output, what was restored, and what is stuck
  (commit `7935159d0`).
- Tests (`crates/gnoblin-wayland/src/output_config.rs`, using a fake backend):
  `a_refusal_on_the_second_output_names_it_and_restores_the_first` (l351),
  `rollback_undoes_the_changes_in_reverse_order` (l363),
  `a_restore_that_is_refused_is_reported_as_stuck` (l372),
  `the_message_names_the_output_by_connector_and_says_what_was_restored` (l384),
  `the_message_names_an_output_left_at_its_new_state` (l403).
- Model: `gnoblin-output` `Transaction` with a confirm timeout in caller ticks.
  Test `confirming_after_the_timeout_rolls_back`,
  `crates/gnoblin-output/tests/failure_modes.rs:264`. A failed test rolls back
  before commit: `failure_modes.rs:220`.

What is missing:

- The confirm timeout has no live caller. `OutputModel` is not constructed outside
  `gnoblin-output`.
- `wlr-output-management` has no confirm step. `design/outputs.md` lists the
  display configuration interface as future work. Wiring a timeout needs a protocol
  decision, not just code.

Run: `cargo test -p gnoblin-wayland output_config` and
`cargo test -p gnoblin-output --test failure_modes`.

## 3. Hotplug

What exists:

- Pure planning: `hotplug(known, now)`, `crates/gnoblin-backend-drm/src/plan.rs:553`.
- Tests: `plan.rs:754` `a_replanned_hotplug_has_no_operation_for_the_removed_connector`,
  plus the cases at `plan.rs:877`, `:885` and `:893`.
- Live loop: `gnoblin-backend-drm/src/gpu.rs:586` `for change in hotplug(&known, &seen)`.

What is missing:

- Live unplug and replug on real DRM. Headless tests drive the planner only.

Needs hardware: unplug and replug a connected output with the compositor running,
and check that the output set changes and the layout is replanned. `docs/testing.md:65`
already lists this as a manual check.

## 4. Output power and gamma

What exists:

- Gamma: `tests/gamma_power.rs` (`crates/gnoblin-wayland/tests/gamma_power.rs`)
  covers a second gamma control refused (l174), restore on release (l206), restore
  on holder disconnect (l220), a wrong-size ramp as a protocol error (l239), a short
  ramp loads nothing (l254), an unreadable ramp descriptor (l269), removing an output
  (l290), and an unsupported backend (l299).
- Power: second object refused (l321), mode change reported (l340), invalid mode as
  a protocol error (l360), removal (l382), unsupported backend (l391).
- Privilege: `zwlr_gamma_control_manager_v1` and `zwlr_output_power_manager_v1` are in
  `PRIVILEGED` in `crates/gnoblin-wayland/tests/access.rs` (commit `11d968970`). Clients
  without the privileged socket do not see them.
- Backend path: `BackendHardware::set_gamma` at `crates/gnoblin-backend-drm/src/lib.rs:449`
  calls `gpu.rs:795` and then the CRTC `set_gamma` at `gpu.rs:359`. The composition root
  implements `OutputControl` over it at `crates/gnoblin-wayland/src/output_control.rs:331-341`.

What is missing:

- A real CRTC ramp change and its restore on real DRM.

Run: `cargo test -p gnoblin-wayland --test gamma_power` and
`cargo test -p gnoblin-wayland --test access`.
Needs hardware: run `gammastep -O 4000` on a live output (it uses `wlr_gamma_control`),
check the ramp changes, then kill it and check the original ramp returns.

## 5. Multi-GPU

What exists:

- Primary GPU selection: `udev.rs:197` `primary()`.
- Tests: `udev.rs:457` `only_the_primary_going_away_is_fatal`,
  `udev.rs:522` `removing_the_primary_ends_the_backend_and_no_other_does`,
  `udev.rs:534` `an_added_gpu_never_becomes_the_primary`,
  `udev.rs:504` `an_override_wins_over_the_boot_gpu`.

What is missing:

- `additional()`, `udev.rs:270`, has no caller outside its own test
  (`udev.rs:541`). Outputs on a secondary GPU are never driven.

Needs hardware: a machine with two DRM cards, one output on each, and a check that
both drive their outputs.

## Docs

- `docs/config/configure/protocols.md:21` and `docs/wayland-protocols.md:39-40` list
  `wlr_gamma_control` and `wlr_output_power_management`. Both match what is served.
- No doc describes VRR. Nothing to correct.
