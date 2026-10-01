# Gnoblin RPMs

Gnoblin installs alongside Fedora's GNOME packages:

| Package                     | Contents                                                    |
| --------------------------- | ----------------------------------------------------------- |
| `gnoblin`                   | Lua supervisor, runtime tools, login entry and `gnoblinctl` |
| `gnoblin-mutter`            | Private Mutter runtime and a Gnoblin backlight policy       |
| `gnoblin-mutter-devel`      | Private headers and pkg-config files for Gnoblin            |
| `gnoblin-portal`            | Portal backend selected by the Gnoblin session              |
| `gnoblin-gnome-integration` | Optional GVfs, Keyring and user-directory services          |

Binaries, libraries, schemas and upstream service definitions stay under
`/usr/lib/gnoblin`. Private libraries do not provide dependencies for Fedora's
GNOME packages. No package replaces, conflicts with or obsoletes GNOME.
The integration package is a subpackage of the `gnoblin` source RPM and is not
required by `gnoblin`. The `gnoblin` package provides and replaces the former
`gnoblin-session` payload package.

[Build and install](../../docs/installation.md#fedora). Build Mutter first,
install its private development package, then build the portal backend and
Gnoblin package. COPR uses the same order; see
[publication](../../docs/distribution.md).

Before distributing binary packages, run:

```sh
python3 scripts/check-rpm-isolation.py PATH_TO_RPM...
bash tests/test-rpm-coexistence.sh MUTTER_RPM GNOBLIN_RPM
```

The coexistence test uses disposable copies of installed GNOME files and
requires Fakeroot. It checks payload installation, schema compilation and
removal; it does not run RPM scriptlets or test DNF resolution or GDM login.

Also test installation alongside stock GNOME on a clean Fedora host, log in
to each session, and remove Gnoblin. Stock GNOME files must remain unchanged.
The local installer runs the isolation check before invoking DNF.

The current package build no longer produces the experimental `mutter` or
`gnome-shell` replacement RPMs. Gnoblin uses its own private Mutter runtime;
the host's GNOME packages remain separate.
