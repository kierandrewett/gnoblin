# Gnoblin RPMs

Gnoblin ships two Fedora source packages: patched Mutter and patched GNOME
Shell. Build them in that order because GNOME Shell uses Mutter's headers.

Prepare a clean source tree and run the repository checks before building:

```sh
just init
just verify-release
just rpm-all
```

The COPR project is `kierandrewett/gnoblin`. The `just copr` recipe submits the
prepared source RPMs in dependency order and waits for Mutter before sending
GNOME Shell. Keep the Fedora chroot aligned with the GNOME major version in the
spec files. A COPR project existing does not mean that a package is ready for
installation; verify a clean host login and rollback first.

The regular GNOME session remains installed alongside Gnoblin. The package
changes the patched Mutter and GNOME Shell builds used by both sessions, while
the Gnoblin session data keeps its own login entry and shell policy.
