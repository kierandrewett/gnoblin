# Testing

Use a focused test while editing. Run the broader checks before a release.

## Choose a check

| Command                          | Checks                                         | Requires                                   |
| -------------------------------- | ---------------------------------------------- | ------------------------------------------ |
| `just verify-fast`               | Syntax, logic, config parser and script tests  | Test dependencies                          |
| `just test-config`               | Native Lua/config loader                       | C compiler, GLib and Lua development files |
| `just verify-installed-headless` | All isolated Shell integration tests           | Current local build                        |
| `just verify`                    | Fast checks, fresh build, headless integration | Build dependencies                         |
| `just test-mutter`               | Mutter native/Wayland/focus suites             | Working seat and file monitoring           |
| `just verify-release`            | Full checks plus native tests and RPM builds   | Real host and packaging tools              |

A passing headless build does not prove login, visible shell controls or portal
consent. Use [hardware verification](real-hardware-verification.md) for those.

## Focused integration checks

Run against the prefix built from your current source:

| Command                                      | Covers                                     |
| -------------------------------------------- | ------------------------------------------ |
| `just gnome-verify`                          | Gnoblin startup and protocol advertisement |
| `just gnome-stock-protocol-isolation-verify` | Stock GNOME behavior stays separate        |
| `just gnome-protocol-boundaries-verify`      | Wayland object and geometry contracts      |
| `just gnome-dbus-verify`                     | Control API                                |
| `just gnome-devkit-verify`                   | Devkit environment and connectivity        |
| `just gnome-native-chrome-verify`            | Removed native UI stays absent             |
| `just gnome-scripting-verify`                | User script load/reload lifecycle          |
| `just gnome-notifications-verify`            | Notification service ownership             |
| `just gnome-protocol-gating-verify`          | Startup protocol switches                  |

## Run a private test

```sh
GNOBLIN_PREFIX="$PWD/install" \
GNOBLIN_TEST_DBUS_CLIENT="$PWD/tests/test-permissions-live.py" \
bash scripts/run-gnome-shell.sh
```

Choose the test matching your change. Tests live in `tests/`; build and launch
helpers live in `scripts/`.

The private harness uses temporary HOME and XDG directories. The
[devkit](devkit.md#isolation) keeps the real HOME; do not confuse the two.

## CI

The verification workflow provisions disposable Fedora and Arch images, then
runs `./build.sh` as an unprivileged user. Package provisioning is confined to
the CI images; the user-facing build script never runs it.
A separate matrix checks dependency provisioning and build tests on Debian,
Ubuntu and openSUSE. It does not prove a complete desktop installation.
Run `python3 tests/build-deps.test.py` and `python3 tests/private-deps.test.py`
to check command planning, checksums and private library links.
Graphical login and interactive checks still need a host.

## Environment failures

Mutter backend tests need a seat and working local file monitoring.
If they fail with `Unable to find default local file monitor type` (exit 251),
fix that environment before treating the run as compositor evidence.

## Add a test

Use an existing `tests/test-*.sh` as a starting point.
Assert observable behavior, keep session state private, and preserve literal
errors. Add an appropriate Just recipe and include it in the relevant suite.
