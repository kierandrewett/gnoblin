# Gnoblin RPMs

Gnoblin installs alongside Fedora's GNOME packages:

| Package                | Contents                                              |
| ---------------------- | ----------------------------------------------------- |
| `gnoblin-mutter`       | Private Mutter runtime and a Gnoblin backlight policy |
| `gnoblin-mutter-devel` | Private headers for building Gnoblin Shell            |
| `gnoblin-shell`        | Private GNOME Shell runtime and session tools         |
| `gnoblin-session`      | Login entry, user units and `gnoblinctl` command      |

Binaries, libraries, schemas and upstream service definitions stay under
`/usr/lib/gnoblin`. Private libraries do not provide dependencies for Fedora's
GNOME packages. No package replaces, conflicts with or obsoletes GNOME.

[Build and install](../../docs/installation.md#fedora). Build Mutter first,
install its private development package, then build Shell. COPR uses the same
order; see [publication](../../docs/distribution.md).

Before distributing binary packages, run:

```sh
python3 scripts/check-rpm-isolation.py PATH_TO_RPM...
bash tests/test-rpm-coexistence.sh MUTTER_RPM SHELL_RPM SESSION_RPM
```

The coexistence test uses disposable copies of installed GNOME files and
requires Fakeroot. It checks payload installation, schema compilation and
removal; it does not run RPM scriptlets or test DNF resolution or GDM login.

Also test installation alongside stock GNOME on a clean Fedora host, log in
to each session, and remove Gnoblin. Stock GNOME files must remain unchanged.
The local installer runs the isolation check before invoking DNF.

Old experimental RPMs named `mutter` and `gnome-shell` are replacement builds.
Do not distribute or install them. The new installer refuses those artifacts
and does not automatically undo an earlier replacement installation.
