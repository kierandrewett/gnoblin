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
| `just fuzz-lifecycle`       | Seeded window and frame lifecycle stress   |
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
Ubuntu and openSUSE. It does not prove a complete desktop installation. Run `python3 tests/build-deps.test.py` and `python3 tests/private-deps.test.py`
to check command planning, checksums and private library links.

Graphical login and interactive checks still need a host.

## Window lifecycle fuzzing

`just fuzz-lifecycle` runs a seeded state-machine fuzzer in a private headless
Gnoblin session. It opens real GTK Wayland windows, enables native Gnoblin
frames, then mixes pointer crossing/click/resize, window state changes, and
graceful, compositor-requested, and abrupt client closes.

Every run finishes by
exiting Gnoblin while a native frame is under the pointer. The run fails if the
shell stops responding unexpectedly or logs a fatal compositor diagnostic.

Use `just fuzz-lifecycle SEED=1738 STEPS=1000` to control a run. Every run keeps
its seed, action plan, executed action prefix, runner output, and shell log under
`$XDG_STATE_HOME/gnoblin/lifecycle-fuzz/` (normally
`~/.local/state/gnoblin/lifecycle-fuzz/`). Replay a failure with the command
printed by the runner, or run:

```sh
python3 tests/window-lifecycle-fuzz.py --replay /path/to/repro.json
```

The harness reports failures and prepares a repair request alongside the exact
replay. A generated patch still needs to pass that replay and the relevant
compositor checks before it is accepted.

## Broad application E2E

`.github/workflows/application-e2e.yml` is the app-compatibility workflow;
`.github/workflows/compositor-fuzz.yml` is the separate seeded state-machine
workflow. The fuzz workflow runs for pull requests, pushes to `main`, nightly,
and manual dispatch. The app sweep runs one shard on a pull request, all shards
weekly, or a selected/full set on manual dispatch.

At workflow start, `tests/e2e/app-catalog.py` refreshes two independent sources:

- The first 500 unique desktop apps in Flathub's Popular collection.
- 300 launchable Fedora RPM applications from `appstream-data`, balanced over
  the metadata categories and excluding duplicate desktop IDs from Flathub.

That makes an 800-application catalog without committing a stale popularity
snapshot. The workflow divides it into 40 shards. Each app is installed or its
installation failure is recorded, then launched in a disposable user session
on the built Gnoblin Mutter compositor.

A real `zwlr-layer-shell` panel stays mapped for the duration. The driver records
whether each app maps a window, then captures a screenshot and checks activation,
native frames, move/resize, titlebar dragging, resize handles, maximize,
minimize, fullscreen and close. It also saves the app's stdout and stderr.

A shard fails if an app cannot be installed or mapped, a supported window
operation is rejected, a window cannot close, or the compositor fails. The
artifacts distinguish those outcomes.

Flathub apps keep their Flatpak sandbox and run without network access, so this
suite measures desktop-window behavior rather than online service behavior.

The Actions runner uses Fedora 44 and Gnoblin's actual Mutter/Wayland code with
virtual 1280x800 monitors and software rendering. It exercises real Flatpak
and RPM clients against real Gnoblin windows without a physical GPU or logged-in
desktop.

GPU drivers, physical input devices and a hardware login need separate coverage
using the [hardware verification](real-hardware-verification.md) checklist.

Each shard artifact contains its exact catalog slice, installation report,
per-app logs and screenshots, JSONL operation trace, summary, shell log and a
reproduction/repair request when it fails. Re-run a shard locally after
installing its recorded apps with:

```sh
GNOBLIN_PREFIX="$PWD/install" \
GNOBLIN_E2E_CATALOG=/path/to/app-catalog.json \
GNOBLIN_E2E_SHARD_INDEX=0 GNOBLIN_E2E_SHARD_COUNT=40 \
python3 tests/e2e/app-e2e.py
```

The test workflows produce machine-readable repair packets and exact replays.
Source patching still needs a configured repair worker that can run the replay,
check the fix and open a reviewable change.

## Environment failures

Mutter backend tests need a seat and working local file monitoring.
If they fail with `Unable to find default local file monitor type` (exit 251),
fix that environment before treating the run as compositor evidence.

## Add a test

Use an existing `tests/test-*.sh` as a starting point.
Assert observable behavior, keep session state private, and preserve literal
errors. Add an appropriate Just recipe and include it in the relevant suite.
