# Wayland Protocol Overlays

`src/protocols/` contains gnoblin-owned Mutter overlay code. These sources are
not built directly from this directory; the dev scripts copy them into the
Mutter submodule and the generated patch stack persists them under
`patches/mutter/`.

## Layout

- `aggregator/` wires all gnoblin protocol globals into Mutter startup.
- One directory per protocol contains:
  - protocol XML, when the XML is not already supplied by Mutter;
  - `meta-wayland-*.c` and `.h` implementation files;
  - `manifest`, used by the overlay copy/generation scripts.

Current overlays include layer-shell, screencopy, session-lock, idle-notify,
data-control, gamma-control, output power/management, and foreign toplevel
list and management.

## Change Flow

1. Edit or add the protocol source under this directory.
2. Wire new globals in `aggregator/meta-gnoblin-protocols.c`.
3. Update `scripts/gen-gnoblin-protocols-patch.sh` if the protocol adds files.
4. Regenerate and reapply the Mutter patches before testing.
5. Gate the protocol on a `[protocols]` key in `gnoblin.conf`.

Smoke-test with `just gnome-verify` (boots gnome-shell in the gnoblin session
mode and confirms `zwlr_layer_shell_v1` is advertised) and the Mutter in-tree
headless suite via `just test-mutter`.
