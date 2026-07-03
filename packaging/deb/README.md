# Debian / Ubuntu packaging (scaffold)

Not implemented yet. The intended approach mirrors the RPM path:

1. `just tarball mutter` / `just tarball gnome-shell` produce release tarballs with
   gnoblin's patches already applied (so `debian/patches/` is **not** used).
2. Add a `debian/` dir per package (control, rules, changelog) for **patched
   mutter** and **patched gnome-shell** — the two things gnoblin ships. The session
   data (`src/data/session/*`, the `gnoblin` session mode + `.desktop`) and
   `gnoblinctl` go in the gnome-shell package (or a small `gnoblin-session` one).
3. Wire `just deb PROJ` to run `debuild` / `dpkg-buildpackage`.

Because patches are pre-applied in the tarball, deb packaging stays a thin wrapper
around the upstream mutter / gnome-shell builds.
