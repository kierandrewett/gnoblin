# Platform support

No distribution has completed Gnoblin's full support gate yet. A fully
supported target must pass package build, clean installation, stock GNOME
coexistence, graphical-session login and removal on that target.

Package candidates have passed dependency probing, package build, clean
installation, coexistence with stock GNOME and removal. They still need a
graphical-session login result, so use them for testing and keep another desktop
session available for recovery.

## Current status

| Distribution        | Releases               | Status            | Available path                                                                                |
| ------------------- | ---------------------- | ----------------- | --------------------------------------------------------------------------------------------- |
| Fedora              | 43, 45                 | Unsupported       | No release package claim                                                                      |
| Fedora              | 44                     | Package candidate | [COPR](install-fedora.md)                                                                     |
| Enterprise Linux    | 8, 9, 10               | Unsupported       | No release package claim                                                                      |
| Debian              | 11, 12                 | Unsupported       | No release package claim                                                                      |
| Debian              | 13                     | Package candidate | [APT package](install-debian.md)                                                              |
| Ubuntu              | 22.04                  | Unsupported       | No release package claim                                                                      |
| Ubuntu              | 24.04, 26.04           | Package candidate | [APT package](install-debian.md)                                                              |
| Arch                | Current                | Package candidate | [PKGBUILD](https://github.com/kierandrewett/gnoblin/blob/main/packaging/arch/PKGBUILD)        |
| openSUSE Leap       | 15.5, 15.6, 16.0       | Unsupported       | No release package claim                                                                      |
| openSUSE Tumbleweed | Current                | Package candidate | [RPM build files](https://github.com/kierandrewett/gnoblin/blob/main/packaging/rpm/README.md) |
| NixOS               | 25.05, 25.11, unstable | Unsupported       | No installable package                                                                        |
| NixOS               | 26.05                  | Unsupported       | [Experimental package and module](install-nixos.md)                                           |

The status comes from the release target matrix in the source tree. It changes
only when the target's required gates pass. A successful build or package
installation does not establish graphical-session support.

## Test a candidate

Use the package instructions linked above for the exact release. Before testing
a candidate, keep stock GNOME or another working desktop session installed.
After installation, verify that the GNOME session remains selectable, then
select **Gnoblin** from your display manager's session menu. Report the target,
display manager, GPU and the result if login fails.

For development builds, use [Build from source](install-source.md). A source
build is a development workflow and does not change the distribution status.
