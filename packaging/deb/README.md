# Debian / Ubuntu packaging (scaffold)

Not implemented yet. The intended approach mirrors the RPM path (see
`packaging/rpm/gnome-shell.spec`'s `gnoblin-session` subpackage for the
working reference):

1. `just tarball mutter` / `just tarball gnome-shell` produce release tarballs with
   gnoblin's patches already applied (so `debian/patches/` is **not** used).
   `scripts/make-tarball.sh` also stages the gnoblin-session payload — session
   mode (`src/data/session/modes/gnoblin.json`), gnome-session file, login
   `.desktop`, the `org.gnoblin.Shell` systemd --user units
   (`src/data/session/systemd-user/`), and the control/wrapper tools
   (`src/tools/gnoblin-env.sh`, `gnoblin-session`, `gnoblin-shell-service`,
   `gnoblinctl`) — into the sources dir alongside the gnome-shell tarball.
2. Add a `debian/` dir per package (control, rules, changelog) for **patched
   mutter** and **patched gnome-shell**. Split a `gnoblin-session` binary
   package out of the gnome-shell source package for the payload above (a
   `Depends: gnome-shell (= ${binary:Version}), gnome-session` binary
   package, matching the RPM subpackage) — do not fold it into the main
   gnome-shell package, since the login-manager `.desktop` and systemd units
   are gnoblin-specific and shouldn't force a rebuild of gnome-shell itself
   to update.
3. Wire `just deb PROJ` to run `debuild` / `dpkg-buildpackage`.

Because patches are pre-applied in the tarball, deb packaging stays a thin wrapper
around the upstream mutter / gnome-shell builds.
