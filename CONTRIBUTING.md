# Contributing to Gnoblin

Gnoblin is the compositor and session layer. It keeps Mutter window
management, input, login integration and hardware services, then exposes a
small interface for an external shell such as Bingux. Shell widgets belong in
the Bingux repository.

## Development

First install the dependencies listed in the
[source installation guide](docs/install-source.md).
Build the pinned upstream sources for the install prefix:

```sh
GNOBLIN_DEVKIT=enabled ./build.sh --dry-run   # read what make will change
make                   # build for /usr/local/lib/gnoblin into build/stage, with the nested viewer; installs nothing
make preview           # run the staged build in a nested window, without root
make install           # after make: copy the build to the prefix and add the login entry (asks for sudo)
```

`./build.sh` is the build, and `make` is a short way to run it with
`--prefix`, `--destdir build/stage` and the build directory `build/make`.

Set `PREFIX=DIR` to use another prefix, and use the same value for `make` and
`make install`. A plain `./build.sh` still builds a private prefix in
`./install`. The [scripts map](scripts/README.md)
says what each script does and who runs it.

Run the fast checks before sending a change:

```sh
make check
make test    # build the tests, then run the CTest suites
```

Run `tests/test-window-manager.sh` only when the real-host and RPM gates are required. The testing guide records which
checks need a real seat, hardware or a running private session.

Before you merge a change to `release.yml`, the packages or their generator, run
`gh workflow run release.yml --ref BRANCH -f dry_run=true`. The package jobs and install
checks run only on `main` and on tags, so a pull request does not reach them.

Strict scheduled and manually dispatched app E2E failures create or update a
Beads/GitHub compatibility issue with the failed outcomes and workflow
artifacts. Treat those outcomes as triage evidence until the artifacts show
whether the cause is Gnoblin, an application or the test environment.

For first-frame presentation changes, run the pixel fixture in a disposable
devkit. It needs GTK4 Python bindings, Pillow, `grim`, and a frame renderer.
Use a prefix built for that location; a copied supervisor can still launch its
compiled compositor path. The fixture checks the worker executable and loaded
Mutter library before recording pixels.

Build the renderer fixture that acknowledges configuration without supplying
a buffer:

```sh
mkdir -p /var/tmp/frame-no-buffer
wayland-scanner client-header src/protocols/window-frame/gnoblin-window-frame-v1.xml \
  /var/tmp/frame-no-buffer/gnoblin-window-frame-v1-client-protocol.h
wayland-scanner private-code src/protocols/window-frame/gnoblin-window-frame-v1.xml \
  /var/tmp/frame-no-buffer/protocol.c
cc -Wall -Wextra -Werror -Wno-unused-parameter -I/var/tmp/frame-no-buffer \
  tests/frame-bind-no-buffer.c /var/tmp/frame-no-buffer/protocol.c \
  $(pkg-config --cflags --libs wayland-client) -o /var/tmp/frame-no-buffer/client
```

Then run the pixel fixture:

```sh
GNOBLIN_PREFIX="$PWD/install" \
GNOBLIN_FRAME_RENDERER="$(command -v bingux-frame)" \
GNOBLIN_FRAME_BIND_NO_BUFFER=/var/tmp/frame-no-buffer/client \
GNOBLIN_DEVKIT_EXEC="python3 $PWD/tests/frame-first-presentation.py /var/tmp/frame-presentation" \
  scripts/run-gnoblin-devkit.sh
```

The fixture checks cold and delayed presentation, paused-renderer resize,
renderer failure, timeout, close during startup, and minimize cancellation.
Screenshots, window states, renderer wire logs, and process identity are saved
in the artifact directory. Use a fresh directory for each run. GTK startup
that exceeds the two-second deadline should take fallback; record that timing
separately from a successful first custom presentation.

For GTK corner and border changes, use an installed Bingux session in a
disposable QEMU VM. It needs Nautilus in the guest, Pillow on the host, and one
1280×800 output at scale 1. Close Files before running:

```sh
python3 tests/qemu-e2e/files-csd-corners.py /path/to/qemu-e2e/run
```

The fixture checks sixteen reconstructed corners against their local GTK
background, unchanged opaque content, four continuous border edges, and
maximize/restore. It saves screenshots, pixel crops, window state and assertions
in the run directory. Its temporary rule and owned Files process are removed
on exit. These software-VM checks do not replace hardware or scaled-output
verification.

## Boundaries

Keep upstream submodules at their pinned commits. Put Gnoblin overlays in
`src/`, protocol changes in `src/protocols/`, and small upstream fixes in
`patches/`. Do not commit build prefixes, generated tarballs or local session
configuration.

Export a committed subproject change through `scripts/manage-patches.py` so its
mail header uses the Gnoblin patch identity:

```sh
scripts/manage-patches.py export mutter HEAD \
  patches/mutter/90-example/0001-example.patch
```

The source build patches Mutter and `xdg-desktop-portal-gnome`; it does not
build or patch GNOME Shell. For uncommitted tracked changes, use
`scripts/manage-patches.py export-worktree PROJECT OUTPUT --subject "..."`.
For a patch that follows existing patches on the same lines, use a temporary
subproject worktree. Apply the earlier patches, stage that state as the baseline,
make the new edit, then export with `--source-tree WORKTREE --against-index`.
The `apply-patches` step (`cmake/source-step.cmake`) checks every patch header before use.

## Packaging

Fedora packages use the specs in `packaging/rpm/`. The COPR project is
`kierandrewett/gnoblin`; publish only after the local release gate and a clean
install test pass. The Arch recipe uses the same CMake/Ninja source build;
its package still needs clean installation and removal verification.

Use small conventional commits. Keep unrelated working-tree changes out of a
commit, and explain the user-visible result in the commit body.

## Lint and format

See [Code quality](docs/code-quality.md) for tool setup, language coverage,
whole-repository checks and formatting selected files.

For documentation prose, use `MARKDOWN_STYLE.md` as the writing guide. Jev
review is required for staged Markdown and agent documentation changes. Create
`.env` in the repository root:

```dotenv
GNOBLIN_DOCS_JEV_PROVIDER=typesafe
GNOBLIN_TYPESAFE_API_KEY=replace-with-your-TypeSafe-key
# Or use OpenRouter instead:
# GNOBLIN_DOCS_JEV_PROVIDER=openrouter
# GNOBLIN_OPENROUTER_API_KEY=replace-with-your-OpenRouter-key
# Optional model override:
# GNOBLIN_DOCS_JEV_MODEL=jev-latest
```

`.env` is ignored by Git. Keep the key in that file and do not commit it.
With the provider configured, run:

```sh
python3 scripts/markdown-style.py review --fail-on-flags docs/page.md
```

The defaults are `jev-latest` on TypeSafe and `~typesafe/jev-latest` on
OpenRouter. Reviews use five concurrent requests by default; set
`--concurrency` to change that. Override the model with `--model` or
`GNOBLIN_DOCS_JEV_MODEL`.

The script includes the full Markdown guide in Jev's state and asks it to
classify each guide section. Jev returns a probability for each rule;
`--threshold` (default `0.7`) controls which results are marked for review.
`--fail-on-flags` exits nonzero if any rule is flagged. The report includes the
exact rule text and requires each flag to be fixed before completing the change.

The pre-commit hook sends staged Markdown source and the style guide to the
chosen provider; referenced image files are not sent. Review output uses color
in interactive terminals and plain text when captured. Use `--color never` or
`--color always` to override automatic color. Process environment variables
override values in `.env`.

See the [TypeSafe API reference](https://api.typesafe.ai/docs) and
[OpenRouter Jev guide](https://openrouter.ai/blog/tutorials/how-to-use-jev/)
for provider details. Agents only run Jev automatically when `.env` opts
in with `GNOBLIN_DOCS_JEV_PROVIDER`.

## Issues and commit hooks

Gnoblin uses Beads for its work queue and GitHub Issues as the shared record.
Install `bd`, authenticate `gh` for the repository, then install the hook once
per clone:

```sh
uv tool run --from pre-commit==4.6.2 pre-commit install
```

The hook syncs Beads and GitHub before each otherwise-valid commit. Commits
require a working GitHub connection and issue-write access. To commit offline,
explicitly skip this hook once with
`SKIP=beads-github-sync git commit ...`, then sync before the next commit. See
`.beads/README.md` for filing and importing issues.

## Shadow shader checks

For shadow rendering changes, run the GPU fixture in a Python environment
with `moderngl` and `numpy` installed:

```sh
python3 tests/window-shadow-gpu.py --report /var/tmp/shadow-gpu.json
```

The fixture compiles the compositor's generated GLSL in an EGL context.
Atomic counters check that settled shadows skip the unused animation state
and that covered pixels skip shadow layers. It does not launch a compositor.
The shader also skips corner-distance math in the fully covered rectangular
interior. Compare hardware and software timing when changing these branches.

Use `--reference-source /path/to/previous-window-effects.c` to compare pixels
through transition endpoints, interpolation, overshoot, and corner shapes.
GPU and wall timings describe this shader alone.

Use `--device-index N` to select another EGL device. Check the reported
renderer before comparing hardware and software results. Keep native-session
pressure checks separate: record actual observer scheduling, frame callbacks,
input receipt, memory pressure, and whether each load process remained alive.
A frame callback does not prove that a frame reached the display.

## Native shadow cache checks

Run these checks in a disposable native VM with a live Gnoblin session,
`grim`, Pillow, and passwordless `sudo perf`. The compositor library must
retain the `window_shadow_paint_target` symbol.

### Build the paced client

Build the client against your installed Wayland development packages:

```sh
mkdir -p /var/tmp/shadow-client
wayland-scanner client-header \
  /usr/share/wayland-protocols/stable/xdg-shell/xdg-shell.xml \
  /var/tmp/shadow-client/xdg-shell-client-protocol.h
wayland-scanner private-code \
  /usr/share/wayland-protocols/stable/xdg-shell/xdg-shell.xml \
  /var/tmp/shadow-client/xdg-shell-protocol.c
cc -I/var/tmp/shadow-client tests/window-shadow-client.c \
  /var/tmp/shadow-client/xdg-shell-protocol.c \
  $(pkg-config --cflags --libs wayland-client glib-2.0) \
  -o /var/tmp/shadow-client/client
```

### Count settled shadow renders

```sh
python3 tests/window-shadow-cache.py /var/tmp/shadow-cache-check \
  --client /var/tmp/shadow-client/client
```

Use a new artifact directory for each run. Set `XDG_RUNTIME_DIR` and
`WAYLAND_DISPLAY` for the test seat if they differ from the defaults. Use
`--prefix` when Gnoblin is installed outside `/usr/local/lib/gnoblin`.

The fixture checks frame callbacks and counts actual shadow renders for
eight seconds after the window settles. An unchanged shadow should need
at most two renders. Use `--reference` to record the pre-cache failure.
The fixture removes its perf probe and closes its client on exit.

### Check invalidation and pixels

The fixture requires one monitor at the origin. It scales screenshot
coordinates with that monitor and injects focus clicks through Mutter's
RemoteDesktop test session.

The VM config must load
`conf.d/**/*.lua` from `~/.config/gnoblin/init.lua`. Run:

```sh
python3 tests/window-shadow-cache-state.py /var/tmp/shadow-cache-states \
  --client /var/tmp/shadow-client/client
```

The fixture temporarily adds a rule for its own client and removes that rule
on exit. It checks:

- Corner shape and shadow geometry, colour, padding, and multiple layers.
- Disable/re-enable and animated shadow changes.
- Minimize/restore, opacity changes, and focus cycles.

Compare settled PNGs with the same fixture on the pre-cache build. The
transition midpoint is timing-dependent; compare its endpoints and confirm
intermediate pixels change.

Run the same checks at each display scale being changed. Keep
transformed-window and pressure evidence separate from these single-seat
checks. Both fixtures take a seat lock and refuse to overlap each other.
