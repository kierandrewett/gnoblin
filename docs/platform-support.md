# Platform support

This page lists the releases that have Gnoblin packages and how far each has been tested.

The Rust compositor has no release packages yet. The packages the install guides name come from the earlier
Mutter-based releases, so every release is Unsupported until packages of the new compositor pass the gates below.

| Status      | What it means                                                                        |
| ----------- | ------------------------------------------------------------------------------------ |
| Supported   | Passes package build, clean installation, graphical-session login and removal        |
| Candidate   | Passes everything except graphical-session login. Use it for testing                 |
| Unsupported | No claim. Use [Build from source](install-source.md) if the libraries are new enough |

## Status by release

| Distribution        | Releases         | Status      | Available path                                                           |
| ------------------- | ---------------- | ----------- | ------------------------------------------------------------------------ |
| Fedora              | 43, 44           | Unsupported | No release package claim                                                 |
| Fedora              | 45               | Unsupported | [Repository or RPMs](install-fedora.md)                                  |
| Enterprise Linux    | 8, 9, 10         | Unsupported | No release package claim                                                 |
| Debian              | 11, 12           | Unsupported | No release package claim                                                 |
| Debian              | 13               | Unsupported | [Source build](install-source.md) if library versions meet requirements  |
| Debian              | Testing (forky)  | Unsupported | [Packages](install-debian.md), install checks pass, no session login yet |
| Ubuntu              | 22.04            | Unsupported | No release package claim                                                 |
| Ubuntu              | 24.04, 26.04     | Unsupported | [Source build](install-source.md) if library versions meet requirements  |
| Arch                | Rolling          | Unsupported | [Repository or packages](install-arch.md)                                |
| openSUSE Leap       | 15.5, 15.6, 16.0 | Unsupported | No release package claim                                                 |
| Alpine              | Edge             | Unsupported | [Packages](install-alpine.md), install checks pass, no session login yet |
| openSUSE Tumbleweed | Rolling          | Unsupported | [Repository or RPMs](install-opensuse.md)                                |

The status comes from the release target matrix in the source tree. It changes
only when the target's required gates pass. A successful build or package
installation does not establish graphical-session support.

## Test a candidate

Use the package instructions linked above for your release, then select **Gnoblin** from your display manager's
session menu. If login fails, report the release, display manager, GPU and the result.

For development builds, use [Build from source](install-source.md). A source
build is a development workflow and does not change the distribution status.
