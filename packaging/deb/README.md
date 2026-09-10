# Debian / Ubuntu packaging plan

Not implemented yet. Packages must install alongside the distribution's GNOME.

- Use `gnoblin-mutter`, `gnoblin-shell` and `gnoblin-session` package names.
- Install the runtime under `/usr/lib/gnoblin`, including private libraries,
  schemas and upstream service files.
- Depend on the matching `gnoblin-*` runtime. Do not replace, conflict with,
  or provide the distribution's `mutter` or `gnome-shell` packages.
- Export only Gnoblin's login entry, control command, user units and separately
  named backlight policy. Follow the [RPM layout](../rpm/README.md).
- Record the private library directory in `libexec/gnoblin-libdir`, relative
  to `/usr/lib/gnoblin` (for example `lib/x86_64-linux-gnu`).

Use `just tarball mutter` and `just tarball gnome-shell` for patched sources.
Wire `just deb` to the build only after install, coexistence and removal tests
pass with stock GNOME installed.
