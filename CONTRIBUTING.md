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

A pull request that changes `release.yml`, the package recipes or the release scripts runs the whole
release pipeline as a dry run. It builds and tests every package and publishes nothing. To run the
same check on a branch without a pull request, use
`gh workflow run release.yml --ref BRANCH -f dry_run=true`. To cut a release, see
[Packaging and releases](design/distribution.md).

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

For command-priority changes, run `python3 tests/command-child-priority.py`.
The probe links the real command and capture sources without a compositor.
It checks repeated launches, the caller's priority, and a shared GTask worker.
Use `--build-only /var/tmp/command-priority` to keep the executable for a
separate run at nice `-5`. A run with nonnegative nice does not prove that
commands drop an inherited boost; the probe reports that missing check.

In a disposable session, also launch an unclassified helper through
`gnoblin.commands.run` and `gnoblin.commands.capture`. Record its actual nice
and scheduler policy, the compositor's per-thread priorities, and the loaded
runtime identity before and after the change. External priority rules can
override both the observer and the helper. This check proves command priority
isolation, not responsiveness under load.

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

Packages are not built by distribution recipes. `scripts/build-prefix.sh` builds Gnoblin into a
prefix, and [pkgsmith](https://github.com/kierandrewett/pkgsmith) writes the rpm, deb, Arch and
tar packages from that prefix with the definitions in `pkgsmith/`. `scripts/build-packages.sh`
runs it, and `scripts/check-package-install.sh` installs the result on a clean system. The
commands, the package list and the release stages are in
[Packaging and releases](design/distribution.md).

When you add a file to the build, check which package ships it: `pkgsmith/runtime.star` names
the portal backend's files, and everything else in the prefix belongs to `gnoblin`. When you add
a library to the build, add its Debian and Arch package to `pkgsmith/dependencies.star`.
`python3 tests/pkgsmith-packages.test.py` checks both with `pkgsmith` on `PATH`.

Use small conventional commits. Keep unrelated working-tree changes out of a
commit, and explain the user-visible result in the commit body.

## Lint and format

See [Code quality](docs/code-quality.md) for tool setup, language coverage,
whole-repository checks and formatting selected files.

For documentation prose, use `design/markdown-style.md` as the writing guide. Jev
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

## Lua API annotations

The signatures the console and editors show for `gnoblin.*` functions come from LuaCATS
annotations in `src/config/lua/api/*.h`. Each block is a C comment that opens with `/*** lua`
and closes with `*/`, holding plain LuaCATS: a `---@class` with its `---@field` lines for each
table argument, and for each function its summary, `---@param` and `---@return` lines and a
`function gnoblin.example(name) end` stub. Describe what the code accepts and enforces, not what
the docs say; read the handler, not just the Lua binding.

After adding or changing a Lua function or the keys it accepts, update its annotation and run:

```sh
python3 scripts/gen-lua-api.py
```

This rewrites `src/config/lua/gnoblin-lua-signatures.inc`, which the console compiles in, and
`docs/public/gnoblin.d.lua`, the file published for Lua Language Server. A pre-commit hook runs
the script with `--check` and fails when either file is stale. Use `--validate` to check the
annotations without writing anything. A C function without an annotation shows as plain `fn` in the
console, so annotate every public function.

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

## CSD pixel ownership checks

Run the CSD sampling fixture against a built Cogl 51 prefix with Clang's
sanitizer runtime available:

```sh
CC=clang PKG_CONFIG_PATH="$PWD/build/make/pkgconfig" \
LD_LIBRARY_PATH="$PWD/build/stage/usr/local/lib/gnoblin/lib64/mutter-51" \
  python3 tests/csd-pixel-lifetime.py --sanitize
```

The fixture applies the CSD patches to disposable pinned source and compiles
the actual `meta_shaped_texture_get_pixels` function. It uses real Cogl host
bitmaps with bounded readback substitutes. It checks early dimension rejection,
premultiplied RGBA byte identity, tight and padded strides, direct and fallback
paths, map failure, and delayed final release.

Renderer and transform functions must remain unchanged. No compositor or
display is started.

Add `--baseline --test /csd/reject-before-readback` to reproduce the old late
dimension check, or `--baseline --test /csd/direct-tight` to reproduce the extra
copy. Both should fail. GPU rendering still needs the existing real-session
`tests/test-window-csd-reconstruction.py` fixture inside an isolated devkit.

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

For rounded window and border shader changes, use `--kind rounded` with
`--reference-source`. This checks pixels and counts executed powers in both
the outer boundary and inset border calculations. See
[`tests/window-rounded-gpu.md`](tests/window-rounded-gpu.md) for the covered
cases and timing limits. Verify GTK corner fill in a native session separately.

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

## Guest pressure check

Run this only in a disposable QEMU guest with a live Gnoblin session. It starts
bounded CPU and memory systemd units, drives the paced client through Mutter's
RemoteDesktop input path, and writes a new artifact directory.

Build the paced client with the command above. Build the bounded guest load
helper from the checked-in source:

```sh
cc -O2 -Wall -Wextra tests/qemu-e2e/pressure-load.c \
  -o /var/tmp/shadow-client/gnoblin-pressure-load
sudo install -Dm755 /var/tmp/shadow-client/gnoblin-pressure-load \
  /usr/local/libexec/gnoblin-pressure-load
sudo restorecon /usr/local/libexec/gnoblin-pressure-load
```

Use passwordless `sudo` for `chrt`, `renice`, `systemd-run`, and
`systemctl`:

```sh
python3 tests/qemu-e2e/guest-pressure.py /var/tmp/gnoblin-pressure \
  --client /var/tmp/shadow-client/client \
  --load-probe /usr/local/libexec/gnoblin-pressure-load
```

The defaults are 8 seconds baseline, 40 seconds combined load, and 5 seconds
recovery. The memory unit is capped at 1900 MiB. Each load unit has a 75-second
cap. The CPU helper has four workers: its parent and three children. Change a
duration only when their sum remains within that runtime cap.

Use one monitor at origin 0,0. The fixture reads its dimensions from Mutter's
DisplayConfig interface and resets the pointer through the lower-right corner
to avoid opening GNOME's overview through its upper-left hot corner.

The client is 320 by 240 pixels. `--focus-x` and `--focus-y` default to the
existing 220,220 target. Supply its actual on-screen position when the
compositor places it elsewhere. The check sends a key after clicking and fails
if the client does not receive it.

`observer-policy.json` records the observer scheduling policy. The paced client
is launched through `nice` and `chrt` without Python `preexec_fn`; its PID,
process start time, nice value, and scheduler are checked after `START` and on
every sample. The check fails if the client is not nice 0 with `SCHED_OTHER`,
including when another service changes its policy during the run. A passing
artifact therefore reports observed policy at those checkpoints, not a promise
that another service cannot change it between samples. `phases.json` contains
guest-monotonic and client-`START` boundaries. `samples.json` records client
policy alongside D-Bus round trips, PSI, memory, and unit liveness.
`inputs.json` pairs each injected key with the received `KEY` event.

Read `result.json` before interpreting `summary.json`. `lifecycle_passed`
replaces the former unqualified `passed` field. It means all three phases
completed, the existing scheduling and liveness checks held at their
checkpoints, and each phase had usable frame, input, and D-Bus observations.
Malformed records or inconsistent frame/input timestamps fail this check.
Exit status zero means this lifecycle check passed.

`performance.status` is always `"not_evaluated"`. There is no performance
acceptance budget in this harness: even a 450 ms frame p95 can accompany
`lifecycle_passed: true`. Compare timing evidence against a workload-specific
budget or reference run with the same VM resources, renderer, display, and
client pacing. A lifecycle pass cannot establish acceptable rendering speed.

Interpret the per-phase metrics as follows:

- `frame_p95_ms` and `frame_max_ms` measure intervals between client dispatches
  of Wayland frame callbacks using `CLOCK_MONOTONIC`. They include the client's
  deliberate 16.667 ms wait before its next commit, client scheduling, and
  compositor delay. They measure neither GPU execution nor presentation.
- `frame_callbacks` counts callback records; `frame_intervals` counts usable
  deltas. The first callback's zero sentinel is excluded from interval metrics.
  Later deltas must equal the difference between adjacent callback timestamps.
- Frame intervals belong to the phase containing their ending callback, using
  `start <= timestamp < end`. `frame_cross_boundary_intervals` counts included
  intervals that started before that phase. Such intervals retain their full
  duration. A stall without a subsequent callback has no completed interval;
  callbacks outside all phases remain only in `frames.log`.
- `frame_over_100ms` counts intervals strictly longer than 100 ms. This is a
  diagnostic count, not an acceptance threshold. All p95 values use nearest
  rank (`ceil(0.95 * sample_count)`); absent observations produce JSON `null`.
- Input latency runs from just before the D-Bus key injection call to the
  client's key handler, in the same guest monotonic clock. It belongs to the
  sending phase even when receipt occurs later. The focus handshake is kept
  in `inputs.json` but excluded from phase summaries. `dbus_p95_ms` separately
  measures the RemoteDesktop properties call, not input delivery.

Input presses are paired in stream order. Missing, extra, non-monotonic, or
early receipts fail lifecycle validation and suppress paired latency metrics;
raw sends and receipts remain in `inputs.json`. Use an otherwise idle input
seat: key events carry no injection ID, so a lost press and an unrelated
replacement press cannot always be distinguished. The probe does not measure
physical input latency or input-to-presentation latency.

Run the local log-reader and aggregation regression with Python and PyGObject:

```sh
python3 tests/qemu-e2e/guest-pressure.test.py -v
```

It starts no VM, compositor, input session, or load process. Cleanup in the
guest harness stops the RemoteDesktop session, load units, and client after a
failed assertion or interruption.

## Native backdrop blur cache checks

Run the allocation and pixel fixture in an empty, disposable devkit.
It requires Quickshell, Pillow, `grim`, GDB with Python support, and permission
to attach GDB to the test compositor. Set `--prefix` and `--quickshell` if needed.

### Compare rendering and allocations

Capture the reference build, then compare the candidate build against it:

```sh
GNOBLIN_PREFIX=/usr/local/lib/gnoblin \
GNOBLIN_DEVKIT_EXEC="python3 $PWD/tests/native-blur-cache.py /var/tmp/blur-cache-reference --reference" \
  scripts/run-staged.sh /usr/local/lib/gnoblin /path/to/reference-stage \
  scripts/run-clean-devkit.sh
GNOBLIN_PREFIX=/usr/local/lib/gnoblin \
GNOBLIN_DEVKIT_EXEC="python3 $PWD/tests/native-blur-cache.py /var/tmp/blur-cache-check --compare /var/tmp/blur-cache-reference" \
  scripts/run-staged.sh /usr/local/lib/gnoblin "$PWD/build/stage" \
  scripts/run-clean-devkit.sh
```

Use fresh artifact directories and leave `GNOBLIN_DEVKIT_CONFIG_SOURCE` unset.
The fixture verifies the worker executable and records loaded library hashes.

The fixture checks:

- Changing backdrop pixels and alpha fades.
- Resize capture bounds, target replacement, and radius changes.
- Resource release on disable, hide, and close, followed by re-enable or show.

At fixed geometry, one panel should allocate four blur textures once.
Alpha changes must reuse them. Disable, hide, and close must release them.
The comparison allows at most one RGB level of difference per channel.

### Check capture failure and recovery

Compile the test-only preload shim and run it inside the disposable devkit:

```sh
cc -shared -fPIC -Wall -Wextra -Werror tests/native-blur-capture-failure.c \
  $(pkg-config --cflags --libs glib-2.0) -ldl -o /var/tmp/blur-capture-failure.so
LD_PRELOAD=/var/tmp/blur-capture-failure.so \
GNOBLIN_TEST_BLUR_BLIT_FAIL=/var/tmp/blur-capture-failure.marker \
GNOBLIN_PREFIX=/usr/local/lib/gnoblin \
GNOBLIN_DEVKIT_EXEC="python3 $PWD/tests/native-blur-cache.py /var/tmp/blur-capture-check --capture-failure-marker /var/tmp/blur-capture-failure.marker" \
  scripts/run-staged.sh /usr/local/lib/gnoblin "$PWD/build/stage" \
  scripts/run-clean-devkit.sh
```

The marker must be absent before the run. The fixture warms the blur before
creating the marker. A failed capture must leave the current backdrop visible.
Removing the marker must restore blur.

The preload library changes only the test processes and returns a Cogl failure.
It does not reproduce driver exhaustion.

### Measure responsiveness separately

GDB changes timing, so measure responsiveness separately in a native VM
without the debugger. This fixture does not prove clone rendering,
allocation-failure recovery, or long-run driver memory behavior.

### Check cache construction and allocation failure

Run the constructor preload probe separately, without GDB:

```sh
cc -shared -fPIC -Wall -Wextra -Werror tests/native-blur-state.c \
  $(pkg-config --cflags --libs gobject-2.0) -ldl -o /var/tmp/blur-state.so
LD_PRELOAD=/var/tmp/blur-state.so \
GNOBLIN_TEST_BLUR_STATE_REPORT=/var/tmp/blur-state.json \
GNOBLIN_PREFIX=/usr/local/lib/gnoblin \
GNOBLIN_DEVKIT_EXEC="python3 $PWD/tests/native-blur-cache.py /var/tmp/blur-state-check --state-report /var/tmp/blur-state.json" \
  scripts/run-staged.sh /usr/local/lib/gnoblin "$PWD/build/stage" \
  scripts/run-clean-devkit.sh
```

Use fresh report and artifact paths. The probe runs inside the test compositor
with its real framebuffer and Cogl context. It checks:

- Fixed geometry reuses targets; simultaneous live nodes receive separate targets.
- Different capture widths that share a downscaled size replace the capture target.
- Failure at each constructor framebuffer allocation preserves the warmed cache.
- Recovery reuses the original geometry, and final release destroys every tracked texture.

The probe constructs nodes without painting them. It does not check painted
clones, lazy blur-pass allocations during painting, or physical driver exhaustion.

## Heap allocation attribution

Analyze heaptrack traces without merging backtraces:

```sh
heaptrack_print -f /path/to/complete-trace.zst --merge-backtraces 0 \
  --print-allocators 0 --print-temporary 0 --print-leaks 1 -n 20
```

Heaptrack 1.5 can merge unrelated unresolved addresses into one unknown
allocation site. That row does not identify a common owner, and merged peaks
can be inaccurate. Keep separate traces when attributing memory.

Save the profiled process's `/proc/PID/maps` and `/proc/PID/smaps` while it is
alive, including before and after opening panels. Use the containing mapping
and its file offset to resolve an unknown address. A mapping alone does not
prove a function name or which object owns an allocation.

Let the owned process exit and the trace writer finish before analysis.
Allocations still live when recording ends are not proof of a leak. Heaptrack
also changes timing and memory use; measure responsiveness and process-tree
PSS plus SwapPss separately without instrumentation. Do not add individual
allocation-site peaks to estimate live memory.
