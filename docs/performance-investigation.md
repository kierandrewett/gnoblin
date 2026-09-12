# Desktop performance investigation

Measured on 2026-09-10. The immediate finding is unnecessary client drawing
behind fully opaque windows when the covered actor has effects. The fix is
generic Mutter code, with no application or shell namespace checks.

## Reproduced and fixed

The benchmark creates eight GTK windows and animates one. It measures compositor
CPU and memory while that window is visible, partly covered, fully covered and
revealed. CPU percentages are percentages of one core. Each phase samples four
seconds after settling. Tests use a private 1280x800, 60 Hz headless session.

| Effect-enabled workload | Before | After |
| --- | ---: | ---: |
| Fully covered client draws in four seconds | 241 | 0 |
| Compositor CPU during full coverage | 14.49% | 0.00% |
| Client draws after reveal | 239 | 240 |

A second run with a partial-coverage phase produced 241 draws while partly
covered, zero while fully covered, and 240 after reveal. Its covered CPU sample
was 1.5%. The draw-count assertion is the regression signal; timing varies with
other machine activity. Visible-animation CPU did not improve in these samples.

`meta-cullable.c` previously discarded visibility information for every actor
with active effects. That is necessary for partial redraw clips because an
offscreen effect needs a complete texture. It is unnecessary when the visibility
region is already empty. The patch preserves that empty region for the
unobscured pass only. It leaves redraw clips, partial visibility and mapped-clone
handling unchanged. This allows Wayland frame callbacks to be throttled again.

The change is built and installed. The running desktop retains its old library
until the next compositor session. Do not interpret current live CPU as a test
of the new build.

## What remains expensive

### Follow-up implementation

The next pass moved geometry uniforms and premultiplied blending into an opt-in
native `Shell.GLSLEffect` path. Corner, border and shadow paints no longer enter
a JavaScript paint override. Actor-sized shadows and explicit client dimensions
both work, including changes in scale. The native pixel suite passed corners,
shadows, borders, translucent content, maximised/fullscreen policy and removal.

The same four-second effect-enabled benchmark then measured 11.50% CPU with 241
visible draws, 11.75% with partial coverage, and 0.00% with zero covered draws.
The previous effect-enabled run measured 17.99% visible CPU. This is a local
sample improvement, not a hardware-independent guarantee. No extra render pass
was removed in this change; that remains a separate optimisation.

WindowRules now updates the old and new focused actors only when a rule depends
on focus. Title events do no rule work unless a title matcher exists. Duplicate
width, height, corner and border updates coalesce by actor. Regression tests
cover focus loss, focus-independent rules, repeated events and destroyed actors.

Fully opaque buffers now skip edge-detection GPU readback using Mutter's existing
opacity metadata. Transparent or ambiguous buffers retain the image-analysis
path. Correction to the initial investigation: ordinary resizes with unchanged
frame/buffer offsets already reuse the edge cache. The remaining synchronous
readback is primarily first-map and geometry-state work for ambiguous clients;
this pass does not make that path asynchronous.

Bingux now bounds unused icon records to 256 entries and 4 MiB of estimated
UTF-16 string data. Active images are reference-counted and are not evicted.
Late responses for evicted requests are ignored. The native QML lifecycle test
verified shared retention, last-user eviction and successful reloading. The
`shell status` IPC response reports active/unused counts and unused string bytes.
This does not bound decoded GPU textures or all shell memory; a total footprint
reduction is not implied by the cache limit.

A fresh-process comparison showed that reload accumulation was not the main
memory cost: the process still used 589.3 MiB PSS after restart. Its 149 saved
notifications all had card delegates despite history being closed. The history
surface now supplies those cards only while preparing, showing or closing
history; closed history retains data rather than UI objects. Fresh-process PSS
then measured 387.6 MiB with the same 149 saved notifications (201.7 MiB lower).
This is a startup comparison, not a claim that the allocator immediately returns
all pages after each close. A 200-notification QML test verifies disposal only
after the closing slide, preserved data and reopening. Native sidebar motion,
interrupted closing and reopening tests also pass. A live open/close preserved
all 149 notifications and the viewport geometry. The first open IPC took 344 ms
including client startup and card creation; cold-opening latency remains a target
for card virtualisation, rather than retaining the entire closed history again.

Both native APIs are packaged as patches. The compositor changes are installed
but need a new compositor session; the Bingux cache changes can reload live.

### Remaining targets

1. **Visible window effects.** A live native stack profile repeatedly crossed
   GJS/FFI while painting offscreen window effects. The native geometry migration
   above addresses that repeated call path. Continue measuring CPU per frame.
   A later single-pass corner/border implementation should preserve accurate
   opaque regions and avoid an extra full-window texture where possible. Do not
   remove Mutter's effect-culling guard globally: that can corrupt cached images.
2. **Shell memory.** The live compositor used about 196 MiB PSS. The main Bingux
   process used 666 MiB; Search, Capture, Emoji and Switcher used another 491 MiB
   combined. These figures cover Quickshell processes, excluding their Python
   worker subprocesses. The open Settings process added 190 MiB. PSS apportions shared pages,
   so it is more useful than adding RSS. These are session snapshots after use
   and reloads, not minimum footprints or proven leaks. Compare a fresh shell
   with repeated open/close/reload cycles, and inspect cache bytes and retained
   QML objects before changing process boundaries.
3. **Synchronous GPU readback.** `windowGeometry()` still calls `actor.get_image()`
   when detecting client shadow edges. It is cached and outside paint callbacks,
   but a first map or geometry-state change can still stall the compositor on the
   GPU. Measure map/resize latency with readback timing; prefer authoritative
   client geometry or a deferred bounded analysis path without losing correctness.
4. **Rule matching.** The focus and geometry changes above reduce refresh counts.
   Matching still constructs regular expressions while evaluating rules. Consider
   compiling matchers when a new validated configuration is installed, with tests
   for configuration replacement and title-dependent rules.
5. **Inherited shell services.** Gnoblin already uses a dummy Overview and an
   empty GNOME panel. Other objects are still constructed in `main.js`, including
   screenshot UI, notifications and keyboard management. Measure their retained
   objects and subscriptions before making them lazy. Preserve authentication,
   accessibility, lock-screen and portal services.

The blur cache is not the current primary target: the earlier native regression
reused one cached backdrop across 300 frames. Live blur effects reported native
damage tracking, with clipped redraws enabled. Broadly disabling blur would not
address the covered-window bug.

## Comparison limits and next acceptance criteria

This harness compares Gnoblin and GNOME `user` modes of the same patched build.
It is not a comparison against an unpatched distribution GNOME installation.
GNOME starts in Overview; the harness dismisses it before measurements because
Overview's live window clones correctly keep clients drawing.

The verified four-second GNOME-mode run passed the same visibility assertions:
240 visible draws, 240 partly covered, zero covered and 241 after reveal. Its
visible-animation CPU was 11.74%, versus 17.99% in the effect-enabled Gnoblin
repeat. This is an indication of the remaining effect overhead, not a claim
about identical visual workloads or stock GNOME. Both modes were nearly idle
with eight static windows (0.25% and 0.00% respectively).

The native build, patch application check, Python syntax check and both mode
benchmarks passed. The before/after covered screenshots were pixel-identical.
The scoped Python similarity check found no duplicate functions. Initial fixture
runs exposed a stack assertion when fullscreen was requested before mapping;
the fixture now presents and settles the cover before changing its state, and
the final runs contain no fatal diagnostics.

Do not claim an overall win over GNOME from compositor-only numbers. Include
Bingux, its helper processes and ancillary workers in the desktop footprint.
Hardware testing must use the same resolution, refresh rate, applications,
recording state and visual features. Headless tests cannot prove direct scanout,
GPU memory usage, power consumption or input-to-photon latency.

Next acceptance criteria:

- Covered animated clients stop drawing; partly visible and revealed clients
  keep drawing. This is now a repeatable regression test.
- Visible animation reduces CPU per frame while preserving pixel output.
- Track median and 95th/99th percentile frame and input latency on real hardware.
- Bound cache memory in bytes and verify steady memory after repeated UI cycles.
- Verify fullscreen direct scanout without overlays, and its recovery after
  opening and closing overlays.

## Reproduce

From the repository root, after building and installing:

```sh
GNOBLIN_PREFIX="$PWD/install" \
GNOBLIN_CONFIG="$PWD/tests/performance-effects.lua" \
GNOBLIN_BENCH_REQUIRE_OCCLUSION=1 \
GNOBLIN_BENCH_REPORT=/tmp/gnoblin-performance.json \
GNOBLIN_TEST_DBUS_CLIENT="$PWD/tests/benchmark-desktop.py" \
bash scripts/run-gnome-shell.sh
```

For a plain Gnoblin baseline, set `GNOBLIN_CONFIG=''`. For GNOME mode, also set
`GNOBLIN_TEST_MODE=user GNOBLIN_TEST_UNSAFE_MODE=1`. This enables Eval only on
the throwaway test compositor so the benchmark can dismiss and verify Overview.
Set `GNOBLIN_BENCH_SECONDS` to increase the sample
duration. Optional `GNOBLIN_BENCH_SCREENSHOT` saves the covered state for visual
verification. Run comparisons sequentially to avoid competing test compositors.
