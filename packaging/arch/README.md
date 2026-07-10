# Arch Linux packaging (scaffold)

Not implemented yet. Intended approach (see `packaging/rpm/gnome-shell.spec`'s
`gnoblin-session` subpackage for the working reference):

1. `just tarball PROJ` produces a patched-source tarball (gnoblin patches
   pre-applied). For `gnome-shell`, `scripts/make-tarball.sh` also stages the
   gnoblin-session payload (session mode, gnome-session file, login
   `.desktop`, the `org.gnoblin.Shell` systemd --user units under
   `src/data/session/systemd-user/`, and the control/wrapper tools under
   `src/tools/`) into the sources dir alongside the tarball.
2. Add a `PKGBUILD` per subproject with `source=(...the tarball...)`, building
   the upstream meson project; wire `just arch PROJ` to run `makepkg`. Split a
   `gnoblin-session` package out of the gnome-shell `PKGBUILD` (a
   `split_gnoblin-session` package function, `depends=('gnome-shell'
   'gnome-session')`) for the payload above, so updating it doesn't force a
   gnome-shell rebuild.

As with RPM/deb, patches live in `patches/` and are applied before the tarball
is made, so the `PKGBUILD` carries no `prepare()` patch step.
