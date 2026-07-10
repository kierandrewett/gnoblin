# Quick Settings Plugin Commands

This folder contains executable commands `gnoblin-qs-*` — legacy quick-settings
backend scripts from the retired Slint top bar's plugin protocol. gnoblin itself
no longer consumes them; they are kept as reference for a bring-your-own chrome
that wants ready-made backends for volume/wifi/bluetooth/etc. tiles.

## Contract

- Print one JSON object to stdout describing `tile` and optional `menu`.
- In oneshot mode, handle the current interaction from `GNOBLIN_QS_EVENT`.
- Keep side effects local to the backing system tool or runtime flag the plugin
  owns.
- Do not add shared helper scripts here without also updating this README:
  gnoblin does not ship an installer for these scripts, they are reference only.

Run `bash -n src/data/plugins/gnoblin-qs-*` after shell edits.
