# Arch Linux packaging plan

Not implemented yet. Packages must install alongside Arch's GNOME packages.

- Use `gnoblin-mutter`, `gnoblin-shell` and `gnoblin-session` package names.
- Install the runtime under `/usr/lib/gnoblin`; set its library directory to
  `lib` and record that in `libexec/gnoblin-libdir`.
- Depend on the matching `gnoblin-*` runtime. Do not replace, conflict with,
  or provide Arch's `mutter` or `gnome-shell` packages.
- Export only Gnoblin's login entry, control command, user units and separately
  named backlight policy. Follow the [RPM layout](../rpm/README.md).

Use `just tarball mutter` and `just tarball gnome-shell` for patched sources.
Wire `just arch` to `makepkg` only after install, coexistence and removal tests
pass with stock GNOME installed.
