# Testing

Use a focused test while editing. Run the broader checks before a release.

## Choose a check

| Command                    | Checks                                         | Requires                         |
| -------------------------- | ---------------------------------------------- | -------------------------------- |
| `just check`               | Syntax, logic, config parser and script tests  | Test dependencies                |
| `just test-session`        | All isolated Shell integration tests           | Current local build              |
| `just test-all`            | Fast checks, fresh build, headless integration | Build dependencies               |
| `just test-window-manager` | Mutter native/Wayland/focus suites             | Working seat and file monitoring |
| `just test-release`        | Full checks plus native tests and RPM builds   | Real host and packaging tools    |

A passing headless build does not prove login, visible shell controls or portal
consent. Use [hardware verification](real-hardware-verification.md) for those.

## Focused integration checks

Run against the prefix built from your current source:

| Command                     | Covers                                     |
| --------------------------- | ------------------------------------------ |
| `just test-startup`         | Gnoblin startup and protocol advertisement |
| `just test-stock-gnome`     | Stock GNOME behavior stays separate        |
| `just test-protocols`       | Wayland object and geometry contracts      |
| `just test-control-api`     | Control API                                |
| `just test-preview`         | Devkit environment and connectivity        |
| `just test-native-chrome`   | Removed native UI stays absent             |
| `just test-scripting`       | User script load/reload lifecycle          |
| `just test-notifications`   | Notification service ownership             |
| `just test-protocol-gating` | Startup protocol switches                  |

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
