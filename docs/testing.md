# Testing Gnoblin

Choose a check based on the code you changed. The source build, native tests,
nested preview, and real login each verify a different part of the session.

## Available checks

| Command                    | What it checks                                                                 | Requirement                                                        |
| -------------------------- | ------------------------------------------------------------------------------ | ------------------------------------------------------------------ |
| `just check`               | Source manifests, patch metadata, scripts, configuration, and packaging checks | Python and repository tooling                                      |
| `just test-runtime`        | CTest runtime, Lua configuration, protocol, and CLI tests                      | A configured build in `build/ninja`                                |
| `just test-all`            | Builds the standalone session, then runs the native runtime checks             | Installed source-build dependencies                                |
| `just test-preview`        | Config and native control behavior in a fresh nested Gnoblin session           | A working Wayland desktop                                          |
| `just test-window-csd`     | Lua `remove_csd` pixel behavior in a fresh nested Gnoblin session              | Source-build prefix, Wayland desktop, Quickshell, grim, and Pillow |
| `just test-window-manager` | Mutter unit, Wayland, backend, and focus tests                                 | A working seat and file-monitoring support                         |
| `just test-release`        | Build and run the native release verification checks                           | Source dependencies and test tools                                 |

## Test the Lua runtime in a preview

Run `just test-preview` to build or reuse the private nested viewer and check
that a Lua configuration loads and that `gnoblinctl` can query and mutate
compositor state. It uses temporary home and XDG directories, a private D-Bus
session, and the host Wayland display. It is not a login or a sandbox.

For an interactive preview, run:

```sh
./build.sh --preview
```

See the [devkit guide](/devkit) for launching shell clients and choosing a
configuration snapshot.

## Test compositor changes

Run `just test-window-manager` for Mutter and Wayland changes. These tests need
access to a real seat; a successful CMake build or CTest run alone does not
verify input, output hotplug, or a graphical login.

Use [hardware verification](real-hardware-verification.md) to check a fresh
login, display configuration, input, lock, and logout on a supported machine.

## CI

The release workflows build the source archive without Git metadata, run the
source build as an unprivileged user, and check package installation and
removal. Separate workflows assess build dependencies and run application
compatibility checks. A passing CI job does not prove behavior on a real seat.
