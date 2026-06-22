# Debian / Ubuntu packaging (scaffold)

Not implemented yet. The intended approach mirrors the RPM path:

1. `just tarball mutter` produces `mutter-49.5.tar.xz` with gnoblin's mutter
   patches already applied (so `debian/patches/` is **not** used for mutter).
2. Package gnoblin's own compositor (`src/compositor`) and Rust/Slint clients
   (`src/clients`) from this repository, matching what `just dev-shell` and
   `just dev-userspace` install into the local prefix.
3. Add a `debian/` dir per package (control, rules, changelog), and wire
   `just deb PROJ` to run `debuild`/`dpkg-buildpackage`.

Because patches are pre-applied in the tarball, the deb packaging stays a thin
wrapper around the upstream mutter build. gnome-shell is retired and should not
be packaged as part of the current gnoblin stack.
