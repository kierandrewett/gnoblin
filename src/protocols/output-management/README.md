# wlr-output-management-unstable-v1 — implementation plan (DEFERRED)

**Status:** vendored protocol XML only (`wlr-output-management-unstable-v1.xml`,
manager v4). Not wired into the build. Deferred because the apply path
(re-configuring CRTCs/modes/positions/scales) cannot be runtime-tested headless and
mis-applying a display config is disruptive; it warrants validation against real
tools on real outputs.

Enables `wlr-randr`, `kanshi` (auto display profiles on hotplug), `wdisplays`, and
`shikane`. **Lower priority:** GNOME's `org.gnome.Mutter.DisplayConfig` D-Bus API
already covers display configuration for GNOME tools; this protocol is purely for
wlr-tool compatibility.

## Protocol shape (v4)

- `zwlr_output_manager_v1`: events `head(zwlr_output_head_v1)`, `done(serial)`,
  `finished`; requests `create_configuration(serial)`, `stop`
- `zwlr_output_head_v1`: events `name`, `description`, `physical_size`,
  `mode(zwlr_output_mode_v1)`, `enabled`, `current_mode`, `position`, `transform`,
  `scale`, `finished`, `make`, `model`, `serial_number`, `adaptive_sync`
- `zwlr_output_mode_v1`: events `size`, `refresh`, `preferred`, `finished`
- `zwlr_output_configuration_v1`: requests `enable_head`, `disable_head`, `apply`,
  `test`, `destroy`; events `succeeded`, `failed`, `cancelled`
- `zwlr_output_configuration_head_v1`: `set_mode`, `set_custom_mode`,
  `set_position`, `set_transform`, `set_scale`, `set_adaptive_sync`

## Implementation approach

### READ side (tractable — advertise heads/modes)
This is what `wlr-randr`/`kanshi` need to *enumerate* outputs. Enumerate from
`MetaMonitorManager`:
- For each `MetaMonitor` / `MetaLogicalMonitor`: emit a `head` with name
  (connector), make/model/serial (`MetaMonitor` info), physical size, enabled,
  current logical position/scale/transform.
- For each `MetaMonitorMode` of the monitor: emit a `mode` (size, refresh,
  preferred).
- Track a `serial` per `done`; bump and re-advertise on `monitors-changed`.

### APPLY side (the risky part — needs runtime validation)
- Accumulate `zwlr_output_configuration_head_v1` requests into a target config,
  translate to a `MetaMonitorsConfig`, and apply via
  `meta_monitor_manager_apply_monitors_config(...)` (the same machinery behind the
  DisplayConfig D-Bus ApplyMonitorsConfig). `test` = build + verify without
  persisting; `apply` = persist. Honour the `create_configuration` serial: send
  `cancelled` if the layout changed since that serial.
- Map transforms/scales between the protocol enums and mutter's
  `MetaMonitorTransform` / fractional scale.

## mutter integration points

- `meta_backend_get_monitor_manager`, `MetaMonitorManager`, `MetaLogicalMonitor`,
  `MetaMonitor`, `MetaMonitorMode`, `meta_monitor_get_main_output`.
- `MetaMonitorsConfig` + `meta_monitor_manager_apply_monitors_config` (private
  headers) for apply/test.
- `monitors-changed` signal for re-advertise + serial bump.

## Wiring (once implemented)

Overlay `.c/.h` + `manifest`; aggregator init call in
`src/gnoblin-protocols/meta-gnoblin-protocols.c`; add to
`scripts/gen-gnoblin-protocols-patch.sh`; `wlr-output-management-enabled` gschema
key; tier-1 block in `scripts/test-build.sh`.

## Runtime validation checklist

- [ ] `wlr-randr` lists all heads/modes correctly (matches GNOME Settings).
- [ ] `wlr-randr --output X --mode/--pos/--scale/--transform` applies correctly.
- [ ] `test` rejects invalid configs without changing the live layout.
- [ ] `kanshi` auto-applies a profile on hotplug; `cancelled` fires on a stale serial.
- [ ] Hotplug re-advertises heads with a new `done` serial.
