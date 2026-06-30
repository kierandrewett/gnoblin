# Quick Settings Plugin Commands

This folder contains executable commands installed on `PATH` as
`gnoblin-qs-*`. Their filenames are part of the config contract: the default
`[qs-plugin.NAME] command = ...` entries in `../gnoblin.defaults.conf` refer to
these names directly.

## Contract

- Print one JSON object to stdout describing `tile` and optional `menu`.
- In oneshot mode, handle the current interaction from `GNOBLIN_QS_EVENT`.
- Keep side effects local to the backing system tool or runtime flag the plugin
  owns.
- Do not add shared helper scripts here unless the installer is changed to ship
  them too; `scripts/install-userspace.sh` installs only `gnoblin-qs-*`.

Run `bash -n src/data/plugins/gnoblin-qs-*` after shell edits.
