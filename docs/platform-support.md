# Platform support

This page lists the releases that have Gnoblin packages and how far each has been tested.

| Status      | What it means                                                                        |
| ----------- | ------------------------------------------------------------------------------------ |
| Supported   | Passes package build, clean installation, graphical-session login and removal        |
| Candidate   | Passes everything except graphical-session login. Use it for testing                 |
| Unsupported | No claim. Use [Build from source](install-source.md) if the libraries are new enough |

A pre-release is a release its distribution has not published. Its packages are built when the build passes and carry no release claim.

## Which releases are listed

Gnoblin aims at older systems as well as new ones. For each family, the list has:

- the newest three releases
- every release first published in the last three years

Both use the release date from [endoflife.date](https://endoflife.date). End of life does not remove a release.

Ubuntu lists long-term support releases only, because an interim release lives nine months and is not a
compatibility target.

Each listed release has a CI state:

| CI state     | What it means                                                                            |
| ------------ | ---------------------------------------------------------------------------------------- |
| Built        | A CI job builds or install-checks it. A failure fails the release run                    |
| Experimental | A CI job runs. A failure does not fail the run. No full run has verified the release yet |
| Planned      | No CI job yet. The reason is in the table                                                |

A CI state is not a support status. Run `python3 scripts/supported-targets.py --table` for the
current list with release dates, end-of-life dates, states and reasons. The rule and the fields are in
the "Choosing the targets" section of `design/distribution.md` in the source tree.

## Status by release

| Distribution        | Releases            | Status      | Available path                                                                                                                                                       |
| ------------------- | ------------------- | ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Fedora              | 39, 40              | Unsupported | No CI job yet. Its libraries are older than the build floors                                                                                                         |
| Fedora              | 41, 42              | Unsupported | Experimental CI job builds and install-checks the packages. No release package claim                                                                                 |
| Fedora              | 43, 44              | Unsupported | CI builds and install-checks the packages, no release package claim                                                                                                  |
| Fedora              | 45 (pre-release)    | Candidate   | [Repository or RPMs](install-fedora.md)                                                                                                                              |
| Enterprise Linux    | 8, 9, 10            | Unsupported | No release package claim                                                                                                                                             |
| Debian              | 10, 11, 12          | Unsupported | No CI job yet. Its libraries are older than the build floors                                                                                                         |
| Debian              | 13                  | Unsupported | Experimental CI job builds and install-checks the packages. No release package claim. [Source build](install-source.md) if library versions meet requirements        |
| Debian              | Testing (forky)     | Unsupported | [Packages](install-debian.md), install checks pass, no session login yet                                                                                             |
| Ubuntu              | 20.04, 22.04        | Unsupported | No CI job yet. Its libraries are older than the build floors                                                                                                         |
| Ubuntu              | 24.04               | Unsupported | Experimental CI job builds and install-checks the packages. No release package claim. Loses the most features: see [older system libraries](#older-system-libraries) |
| Ubuntu              | 26.04               | Unsupported | Experimental CI job builds and install-checks the packages. No release package claim. [Source build](install-source.md) if library versions meet requirements        |
| Arch                | Rolling             | Candidate   | [Repository or packages](install-arch.md)                                                                                                                            |
| openSUSE Leap       | 15.4, 15.5, 15.6    | Unsupported | No CI job yet. Its libraries are older than the build floors                                                                                                         |
| openSUSE Leap       | 16.0                | Unsupported | Experimental CI job builds and install-checks an rpm built on Leap 16.0. No release package claim                                                                    |
| Alpine              | Edge                | Unsupported | [Packages](install-alpine.md), install checks pass, no session login yet                                                                                             |
| Alpine              | 3.21 to 3.24        | Unsupported | Experimental CI job for each branch builds and install-checks apk packages built on that branch. No release package claim                                            |
| Alpine              | 3.15 to 3.20        | Unsupported | No CI job yet. Its libraries are older than the build floors                                                                                                         |
| openSUSE Tumbleweed | Rolling             | Candidate   | [Repository or RPMs](install-opensuse.md)                                                                                                                            |
| NixOS               | 25.05, 25.11, 26.05 | Unsupported | [Channel requirements](install-nixos.md)                                                                                                                             |
| NixOS               | unstable            | Unsupported | [Experimental flake package](install-nixos.md), pending session verification                                                                                         |

The status changes only when a release passes the gates in the first table. A
successful build or package installation does not establish graphical-session
support.

Bingux reports a further Arch result. It builds an Arch package of Gnoblin, installs it into a live ISO and boots the Gnoblin session in
QEMU with UEFI and BIOS. Its boots cover the greeter, user session, lock and
unlock, logout, and an install to disk. This is not a Gnoblin CI gate, so Arch
stays a candidate.

## X11 apps and GTK 4

The split of GTK 4 out of `gnoblin` is not complete. It is not a promise that an install with no GTK 4 is safe for
X11 apps. The title bars and borders that Mutter shows on X11 apps come from a program that needs GTK 4. It is in
its own package, `gnoblin-x11-frames`.

- Without that package, X11 apps that use the system title bar get no title bar and no borders, and some X11
  windows may fail to open. No one has tested that case in a graphical session. Wayland apps and apps that draw
  their own title bar do not change.
- Fedora, openSUSE and Debian install the package with `gnoblin` as a recommended package. You can refuse it, at
  your own risk for X11 apps.
- Arch and Alpine cannot recommend a package, so `gnoblin` depends on it there. A plain install brings GTK 4 with it.
- The `.tar.xz` packages have no dependencies. The frames tarball is needed for X11 apps.

See [optional packages](optional-packages.md#x11-window-frames).

## Older system libraries

Gnoblin builds against the libraries of the oldest release that it supports. Where the system has an older library
than the one Gnoblin was developed against, these features are not available:

| Library                   | Older than    | Effect                                                                                                                                                |
| ------------------------- | ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| libinput                  | 1.26          | The dials of a tablet pad do nothing. The pressure range of a pen is not applied                                                                      |
| libinput                  | 1.27          | Tap-and-drag lock ends a short time after you lift the finger, not at the next tap                                                                    |
| libinput                  | 1.29          | Pens report no eraser button, and the eraser button setting has no effect                                                                             |
| libinput                  | 1.30          | Gnoblin loads no libinput Lua plugins                                                                                                                 |
| libwayland                | 1.23          | Each app connection has a 4 KB message buffer, not 1 MB. An app that floods messages or reads slowly is disconnected sooner                           |
| libwayland                | 1.24          | Clients get no `wl_fixes` global, and they repeat keys themselves                                                                                     |
| PipeWire                  | 1.2           | Screen sharing works without explicit sync. With a GPU driver that needs explicit sync, the shared screen can show torn or partly drawn frames        |
| libadwaita                | 1.6           | Apps get no accent colour from the Settings portal. The wallpaper preview has the older spinner. App chooser text is not centred                      |
| libadwaita                | 1.7           | The screen sharing dialog shows the older view switcher, in the header bar style                                                                      |
| xdg-desktop-portal        | 1.19          | Sandboxed apps cannot ask for USB devices. The other portal features of releases 1.19 and 1.20 are not available                                      |
| libei                     | 1.5           | Gnoblin does not wait for queued input before it answers a sync request from a remote desktop client                                                  |
| IBus                      | 1.5.33        | The IBus input method shows its text without the preedit hints                                                                                        |
| gsettings-desktop-schemas | 47            | Switch monitor and keybinding pen button actions work from the Lua configuration only. Pens have no pressure range setting. Apps get no accent colour |
| gsettings-desktop-schemas | 49            | The newer input settings can be set in the Gnoblin Lua configuration only                                                                             |
| Glycin                    | not installed | The portal backend decodes wallpaper previews with gdk-pixbuf in its own process, not in the Glycin sandbox                                           |

Ubuntu 24.04 has every library in the table at an older version, so it has every effect in the table.

On Ubuntu 24.04, Gnoblin also does two things with its own code that the system libraries do on newer releases:

- It reads the colour primaries, gamma and HDR data of each display from the display's EDID. The code is a copy of
  the libdisplay-info 0.2 code.
- It reads the bus type of a tablet from the kernel.

These have no known effect. No one has started a Gnoblin session on Ubuntu 24.04, so no one has seen any of these
effects on a real desktop.

## Test a candidate

Use the package instructions linked above for your release, then select **Gnoblin** from your display manager's
session menu. If login fails, report the release, display manager, GPU and the result.

For development builds, use [Build from source](install-source.md). A source
build is a development workflow and does not change the distribution status.
