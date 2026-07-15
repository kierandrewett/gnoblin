# Arch Linux packaging (scaffold)

Not implemented yet. Intended approach (see `packaging/rpm/gnome-shell.spec`'s
`gnoblin-session` subpackage for the working reference):

1. `just tarball PROJ` produces a reproducible patched-source tarball and
   publishes it only after sidecar staging succeeds. For `gnome-shell`,
   `scripts/make-tarball.sh` also stages the session mode, gnome-session file,
   login `.desktop`, Gnoblin systemd user units, schema override, shared
   environment helper, and control/wrapper tools beside the tarball.
2. Add a `PKGBUILD` per subproject with `source=(...the tarball...)`, building
   the upstream meson project; wire `just arch PROJ` to run `makepkg`. Split a
   `gnoblin-session` package out of the gnome-shell `PKGBUILD` (a
   `split_gnoblin-session` package function, `depends=('gnome-shell'
   'gnome-session')`) for the payload above, so updating it doesn't force a
   gnome-shell rebuild.
   Install a mode-0644 `gnoblin-libdir` beside `gnoblin-env.sh` containing
   `lib`, the library path relative to `/usr`; installed wrappers read this
   instead of assuming Fedora's `lib64`.

As with RPM/deb, patches live in `patches/` and are applied before the tarball
is made, so the `PKGBUILD` carries no `prepare()` patch step.
