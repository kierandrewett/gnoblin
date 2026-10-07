# Gnoblin RPMs

Gnoblin installs alongside Fedora's GNOME packages:

| Package                     | Contents                                                    |
| --------------------------- | ----------------------------------------------------------- |
| `gnoblin`                   | Single compositor, Lua supervisor, runtime tools and session |
| `gnoblin-portal`            | Portal backend selected by the Gnoblin session              |
| `gnoblin-gnome-integration` | Optional GVfs, Keyring and user-directory services          |

Binaries, libraries, schemas and upstream service definitions stay under
`/usr/lib/gnoblin`. Private libraries do not provide dependencies for Fedora's
GNOME packages. No package replaces, conflicts with or obsoletes GNOME.
The integration package is a subpackage of the `gnoblin` source RPM and is not
required by `gnoblin`. The `gnoblin` package provides and replaces the former
`gnoblin-session` payload package.

[Build and install](../../docs/installation.md#fedora). The `gnoblin` RPM
builds and contains the patched Mutter runtime and session supervisor. The
optional portal backend remains a separate package. See
[publication](../../docs/distribution.md).

Before distributing binary packages, run:

```sh
python3 scripts/check-rpm-isolation.py PATH_TO_RPM...
```

The isolation check inspects package names, file paths and dependency metadata.
It does not test package installation, scriptlets, DNF resolution or GDM login.
Also test installation alongside stock GNOME on a clean Fedora host, log in
to each session, and remove Gnoblin. Stock GNOME files must remain unchanged.
The local installer runs the isolation check before invoking DNF.

The package build does not produce Mutter or GNOME Shell replacement RPMs.
Gnoblin's private Mutter runtime ships inside the `gnoblin` package; the host's
GNOME packages remain separate.
