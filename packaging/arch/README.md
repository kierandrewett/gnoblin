# Arch Linux packaging (scaffold)

Not implemented yet. Intended approach:

1. `just tarball PROJ` produces a patched-source tarball (gnoblin patches
   pre-applied).
2. Add a `PKGBUILD` per subproject with `source=(...the tarball...)`, building
   the upstream meson project; wire `just arch PROJ` to run `makepkg`.

As with RPM/deb, patches live in `patches/` and are applied before the tarball
is made, so the `PKGBUILD` carries no `prepare()` patch step.
