# Arch Linux packaging

`PKGBUILD` is the generated `gnoblin` metapackage. Its version, private-runtime
major bounds, GNOME capability dependencies, and Arch package-name mappings
come from `nix/native-packages.nix`. Refresh it with
`just package-manifest write`.

- Use `gnoblin-mutter`, `gnoblin-shell` and `gnoblin-session` package names.
- Install the runtime under `/usr/lib/gnoblin`; set its library directory to
  `lib` and record that in `libexec/gnoblin-libdir`.
- Depend on the matching `gnoblin-*` runtime. Do not replace, conflict with,
  or provide Arch's `mutter` or `gnome-shell` packages.
- Export only Gnoblin's login entry, control command, user units and separately
  named backlight policy. Follow the [RPM layout](../rpm/README.md).

The `gnoblin-mutter`, `gnoblin-shell`, and `gnoblin-session` Arch runtime
packages and pacman repository are not published yet. Until they are, use the
[source build](../../docs/installation.md#build-from-source). Do not publish
the metapackage by itself: all dependencies must resolve in the same repository.
