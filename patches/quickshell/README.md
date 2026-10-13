# Prepare Quickshell desktop actions

Apply this patch before building the managed Quickshell runtime. It preserves
the complete `DesktopAction.id` from each `[Desktop Action ID]` group. The
affected parser drops the first character, so `proof` becomes `roof` and two
actions with the same remaining suffix collapse into one model entry.

The patch targets Quickshell v0.2.1 at
`a1a150fab00a93ea983aaca5df55304bc837f51b`. It changes only
`src/core/desktopentry.cpp`. Bingux passes the model's action ID to its GIO
launcher; display labels are not action identifiers.

## Managed build path

Gnoblin's `make` and `build.sh` do not build Quickshell. The existing Fedora
runtime is an external CMake build, recorded in
`~/.local/share/gnoblin/quickshell-fedora/BUILD.md`.

Its source and build cache are under
`~/.local/share/gnoblin/quickshell-fedora-src`. The cache selects
`Release`, disables crash reporting and installs into
`~/.local/share/gnoblin/quickshell-fedora`.

`~/.local/bin/gnoblin-quickshell` launches that runtime with the matching
native Qt environment. Bingux's `scripts/install-shell.py` selects an existing
runtime; it does not compile Quickshell. Preserve the external source tree's
other patches and its configured build options.

From the Gnoblin repository, run the preparation step before the existing
CMake build:

```sh
qs_source="$HOME/.local/share/gnoblin/quickshell-fedora-src"
python3 scripts/prepare-quickshell.py "$qs_source"
cmake --build "$qs_source/build" --parallel 4
python3 tests/quickshell-desktop-actions.py /var/tmp/quickshell-actions-candidate \
  --quickshell "$qs_source/build/src/quickshell"
```

Use a fresh artifact directory for each test. For another managed source
location, set `qs_source` to that directory. The preparation script needs
Python 3 and GNU `patch`. It accepts the exact pinned parser or its already
patched form, preserves other files, and rejects source drift for review.
It does not verify the rest of the source tree or configure the build.

`python3 scripts/prepare-quickshell.py --check "$qs_source"` changes nothing.
It exits 0 when the patch is present, 1 when it is still required, and 2 for
unrecognized or missing parser source. Run preparation again after refreshing
the upstream source. Rebuild the executable: Quickshell statically links its
QML modules, so replacing QML tooling metadata cannot apply this fix.

After the candidate passes, the integration owner installs it using the
external build's existing `cmake --install "$qs_source/build"` step, then
starts a session using the new executable. Probe that exact runtime too:

```sh
python3 tests/quickshell-desktop-actions.py /var/tmp/quickshell-actions-installed \
  --quickshell "$HOME/.local/bin/gnoblin-quickshell"
```

## Runtime regression and standalone compatibility

The test creates a private desktop-file directory and reads real
`DesktopEntries.byId(...).actions` through Quickshell QML. It checks complete
IDs, a one-character ID, leading hyphens, suffix collisions and duplicate
display labels. It uses the offscreen Qt platform, private runtime paths and
unavailable D-Bus addresses. It creates no window and executes no action.

`result.json` records the expected and observed IDs and labels; `runtime.log`
records the runtime output. Exit 0 means this runtime preserves the fixture's
action identities. Exit 1 means the probe failed; inspect the logs to
distinguish incorrect identities from a runtime startup failure.

Use the same test with `--quickshell /path/to/quickshell` to check a standalone
or distribution runtime. This is a behavioral capability check; the version
number cannot distinguish patched and unpatched builds. An older system
Quickshell remains incompatible with Bingux's GIO desktop-action route until
its parser is fixed. This managed patch does not update system packages.

Do not reconstruct IDs from labels or truncated suffixes. The parser can lose
whole actions through collisions, and an incorrect ID can match a different
valid action. A universal desktop-action claim requires a corrected runtime
or a separate action model that reads authoritative desktop-file IDs.

## Native delivery check

In a disposable Gnoblin session, the integration owner must select the real
dock menu action whose ID differs from its display label. Confirm that GIO
activates that exact action, its expected window opens, and the ordinary
application starts at nice 0 while the shell retains its boost. Record the
runtime executable and scheduling policy.

The headless model test does not
prove menu interaction, GIO activation, installation or native scheduling.

The upstream [desktop-entry source][parser] defines the parsed identity, and
the [build instructions][build] describe the external CMake build.

[parser]: https://git.outfoxxed.me/quickshell/quickshell/src/commit/a1a150fab00a93ea983aaca5df55304bc837f51b/src/core/desktopentry.cpp
[build]: https://git.outfoxxed.me/quickshell/quickshell/src/commit/a1a150fab00a93ea983aaca5df55304bc837f51b/BUILD.md
