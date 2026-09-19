# Debian / Ubuntu packaging

`debian/` is the generated `gnoblin` metapackage source. Its version,
private-runtime major bounds, GNOME capability dependencies, and Debian
package-name mappings come from `nix/native-packages.nix`. Refresh it with
`just package-manifest write`.

- Use `gnoblin-mutter`, `gnoblin-shell` and `gnoblin-session` package names.
- Install the runtime under `/usr/lib/gnoblin`, including private libraries,
  schemas and upstream service files.
- Depend on the matching `gnoblin-*` runtime. Do not replace, conflict with,
  or provide the distribution's `mutter` or `gnome-shell` packages.
- Export only Gnoblin's login entry, control command, user units and separately
  named backlight policy. Follow the [RPM layout](../rpm/README.md).
- Record the private library directory in `libexec/gnoblin-libdir`, relative
  to `/usr/lib/gnoblin` (for example `lib/x86_64-linux-gnu`).

The `gnoblin-mutter`, `gnoblin-shell`, and `gnoblin-session` Debian runtime
packages and APT repository are not published yet. Until they are, use the
[source build](../../docs/installation.md#build-from-source). Do not publish
the metapackage by itself: all dependencies must resolve in the same repository.
