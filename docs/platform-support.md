# Platform support

This page lists the releases that have Gnoblin packages and how far each has been tested.

| Status      | What it means                                                                        |
| ----------- | ------------------------------------------------------------------------------------ |
| Supported   | Passes package build, clean installation, graphical-session login and removal        |
| Candidate   | Passes everything except graphical-session login. Use it for testing                 |
| Unsupported | No claim. Use [Build from source](install-source.md) if the libraries are new enough |

A pre-release is a release its distribution has not published. Its packages are built when the build passes and carry no release claim.

## Status by release

| Distribution        | Releases            | Status      | Available path                                                               |
| ------------------- | ------------------- | ----------- | ---------------------------------------------------------------------------- |
| Fedora              | 43, 44              | Unsupported | CI builds and install-checks the packages, no release package claim          |
| Fedora              | 45 (pre-release)    | Candidate   | [Repository or RPMs](install-fedora.md)                                      |
| Enterprise Linux    | 8, 9, 10            | Unsupported | No release package claim                                                     |
| Debian              | 11, 12              | Unsupported | No release package claim                                                     |
| Debian              | 13                  | Unsupported | [Source build](install-source.md) if library versions meet requirements      |
| Debian              | Testing (forky)     | Unsupported | [Packages](install-debian.md), install checks pass, no session login yet     |
| Ubuntu              | 22.04               | Unsupported | No release package claim                                                     |
| Ubuntu              | 24.04, 26.04        | Unsupported | [Source build](install-source.md) if library versions meet requirements      |
| Arch                | Rolling             | Candidate   | [Repository or packages](install-arch.md)                                    |
| openSUSE Leap       | 15.5, 15.6, 16.0    | Unsupported | No release package claim                                                     |
| Alpine              | Edge                | Unsupported | [Packages](install-alpine.md), install checks pass, no session login yet     |
| openSUSE Tumbleweed | Rolling             | Candidate   | [Repository or RPMs](install-opensuse.md)                                    |
| NixOS               | 25.05, 25.11, 26.05 | Unsupported | [Channel requirements](install-nixos.md)                                     |
| NixOS               | unstable            | Unsupported | [Experimental flake package](install-nixos.md), pending session verification |

The status comes from the release target matrix in the source tree. It changes
only when the target's required gates pass. A successful build or package
installation does not establish graphical-session support.

Bingux reports a further Arch result. It builds an Arch package of Gnoblin, installs it into a live ISO and boots the Gnoblin session in
QEMU with UEFI and BIOS. Its boots cover the greeter, user session, lock and
unlock, logout, and an install to disk. This is not a Gnoblin CI gate, so Arch
stays a candidate.

## Test a candidate

Use the package instructions linked above for your release, then select **Gnoblin** from your display manager's
session menu. If login fails, report the release, display manager, GPU and the result.

For development builds, use [Build from source](install-source.md). A source
build is a development workflow and does not change the distribution status.
