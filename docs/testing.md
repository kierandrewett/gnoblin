# Testing Gnoblin

Choose a check based on the code you changed. The source build, native tests,
nested preview, and real login each verify a different part of the session.

## Available checks

| Command or test                           | What it checks                                                                                         | Requirement                                                        |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------ |
| `make check`                              | Source manifests, patch metadata, scripts, configuration, and packaging checks                         | Python and repository tooling                                      |
| `make test`                               | CTest runtime, Lua configuration, protocol, and CLI tests                                              | A configured build in `build/make`                                 |
| `tests/test-gnoblin-devkit.sh`            | Config, native controls, and visible workspace animation frames in a fresh nested Gnoblin session      | A working Wayland desktop and `grim`                               |
| `tests/test-privacy-pipewire.py`          | Live microphone and camera activity through `gnoblin.privacy.state()`                                  | Source build, Wayland desktop, PipeWire tools                      |
| `tests/test-window-csd-reconstruction.py` | Lua `remove_csd` pixel behavior in a fresh nested Gnoblin session                                      | Source-build prefix, Wayland desktop, Quickshell, grim, and Pillow |
| `tests/test-window-native-borders.py`     | Lua border rule pixels in a fresh nested Gnoblin session                                               | Source-build prefix, Wayland desktop, Quickshell, grim, and Pillow |
| `tests/test-window-rule-lifecycle.py`     | Repeated Lua window-rule reloads preserve a live window in a fresh nested session                      | Source-build prefix, Wayland desktop, and Quickshell               |
| `tests/test-window-native-shadows.py`     | Lua replacement shadow pixels, rule removal, and translucent content in a fresh nested Gnoblin session | Source-build prefix, Wayland desktop, Quickshell, grim, and Pillow |
| `tests/test-window-manager.sh`            | Mutter unit, Wayland, backend, and focus tests                                                         | A working seat and file-monitoring support                         |

`make` builds the session and `make test` runs the CTest suites after it. The nested pixel tests
(`test-window-csd-reconstruction`, `test-window-native-borders`, `test-window-rule-lifecycle` and
`test-window-native-shadows`) run through the nested viewer. For example:

```sh
GNOBLIN_DEVKIT_EXEC='python3 tests/test-window-csd-reconstruction.py' make preview
```

Use the same form for the other three tests. Replace the script name in `GNOBLIN_DEVKIT_EXEC`.
`test-window-rule-lifecycle` also needs `GNOBLIN_DEVKIT_CONFIG_SOURCE=tests/configs/window-rule-lifecycle`.

## Test the Lua runtime in a preview

Run `tests/test-gnoblin-devkit.sh` to build or reuse the private nested viewer and check
that a Lua configuration loads and that `gnoblinctl` can query and mutate
compositor state. It uses temporary home and XDG directories, a private D-Bus
session, and the host Wayland display. It is not a login or a sandbox.

The test also resizes a Wayland client through `Window:resize`, checks the
animation lifecycle events, and captures intermediate rendered frames.

For an interactive preview, run:

```sh
make preview
```

See the [devkit guide](/devkit) for launching shell clients and choosing a
configuration snapshot.

Run `python3 tests/test-privacy-pipewire.py` to check live microphone and camera
transitions. It starts a private PipeWire graph for the nested session, so the
test does not connect to or change the host audio graph. A virtual audio source
and an active stream with the PipeWire media role set to `Camera` let the test
run without capture hardware.

The test requires `pipewire`, `wireplumber`, `pipewire-pulse`, `pactl`,
`pw-cat`, and `pw-cli`. Its source build prefix defaults to `install`, the
`./build.sh` default. Set
`GNOBLIN_TEST_PREFIX` to use a different prefix.

## Test compositor changes

Run `tests/test-window-manager.sh` for Mutter and Wayland changes. These tests need
access to a real seat; a successful CMake build or CTest run alone does not
verify input, output hotplug, or a graphical login.

Use [hardware verification](real-hardware-verification.md) to check a fresh
login, display configuration, input, lock, and logout on a supported machine.

## CI

The release workflows build the source archive without Git metadata, run the
source build as an unprivileged user, and check package installation and
removal. Separate workflows assess build dependencies and run application
compatibility checks. A passing CI job does not prove behavior on a real seat.
