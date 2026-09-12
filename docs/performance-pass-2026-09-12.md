# Compositor performance pass, 12 September 2026

This pass starts from the existing working tree. Earlier uncommitted fixes are
preserved. The scope is Gnoblin's compositor and session code. Bingux remains a
separate part of the complete desktop footprint.

## Work plan

- [x] Measure the existing private compositor and a passive live sample.
- [x] Remove repeated window-rule compilation and dependency scans.
- [x] Reduce bridge snapshots and blur-region updates that do no useful work.
- [x] Reduce duplicate foreign-toplevel protocol events and list removal cost.
- [x] Inspect native effect allocation and per-frame work, then test safe changes.
- [x] Inspect disabled GNOME UI services for avoidable retained objects.
- [x] Build without resetting the dirty submodules, then run native integration.
- [x] Record results, limitations and remaining hardware checks.

## Measurement rules

Use operation counts for deterministic work reductions. CPU and memory samples
need the same workload and renderer, and must run sequentially. An installed
binary baseline can differ from earlier uncommitted source changes, so do not
attribute the full difference to this pass without an exact build comparison.

Do not restart the active desktop. Private compositor tests can prove protocol,
pixel and lifetime behaviour. They cannot prove direct scanout, GPU power use or
input-to-photon latency on the active display. A comparison between Gnoblin and
GNOME modes of the patched build is not an unpatched GNOME comparison.

## Verified changes

| Workload | Before | After | Evidence |
| --- | ---: | ---: | --- |
| Static blur mask, 300 moving-backdrop frames | 300 mask captures during sample | 0 after initial capture | Same optimised native build, cache disabled/enabled; pixel and fade regressions pass |
| Mask uniform-location setup, same sample | 300 setups (2,400 lookups) | 0 after initial setup | Native effect counters |
| Clutter stage-view redraw-region copies | 420 copies | 0 copies | Actual MtkRegion callsites over whole fixture, including startup |
| Five-second animated exclusive-zone panel | 301 work-area invalidations, 301 frames | 0 invalidations, 300 frames | Native private compositor; only the three protocol files rebuilt |
| Same panel compositor CPU, one core | 6.20% | 5.80% | Short sample; operation count is the acceptance signal |
| 500 unchanged blur-region registrations | 500 window scans, 1,000 effect writes | 0 scans, 0 writes | Production module with counted calls; real blur pixel suite also passed |
| Six rules, 10,000 evaluations | 60,000 regex constructions during evaluation | 6 at config installation, 0 during evaluation | Production matcher test |
| Native blur redraw fallback | Two signal handlers per registered actor | No fallback handlers or retained actor | GJS lifetime test |
| Window-state broadcast | Encode once per subscriber | Encode once for all subscribers | Production transport test |
| Maximise protocol transition | Duplicate state/done events | One state/done pair | Native black-box protocol test failed before and passed after |

The native cache A/B used the same optimised build and Lua fixture. The red
variant forced mask capture and uniform-location resolution on every effect
paint, while retaining the counters and invalidation fixes. The Clutter variant
restored the original region clone. Both variants rendered 300 sampled frames
and built the blur cache once. Total mask captures were 413 before and 1 after;
300 versus 0 occurred within the five-second sample. Region counts include
startup. Instrumented CPU was 27 versus 28 ticks, which establishes no CPU
improvement in this short sample; the demonstrated saving is the removed work.
The exact green sources were restored and rebuilt before regression tests.

The layer-shell change compares the effective window type, stacking layer,
keyboard state and exact reserved rectangle. Real geometry changes still update
work areas. The strut list is retained until its value changes.

Rule dependencies and shader paths are cached per configuration. Event batches
reuse their sets. Companion lookup reads each visible namespace once, with no
window scan when there are no requests. Switcher liveness uses set membership.
The bridge retains one unprocessed input suffix per read and limits slow-client
queues to 64 records and 4 MiB. Foreign-toplevel list removal uses its stored
list link. Last advertised strings are retained per handle and released on
handle destruction; this trades a small bounded cache for fewer protocol events.

## Native GNOME and Mutter work

The later pass prioritises patches to upstream C code. Each is committed and
pushed after native verification:

- Shell patch 64 caches the eight Cogl uniform identifiers used by masked blur.
  They are context identifiers, so shader recompilation does not invalidate
  them. Removing and rebuilding the mask layer does refresh the cache.
- Shell patch 67 reuses the captured alpha mask when only the backdrop changes.
  Client damage and changes to the crop, blur mode, radius or framebuffer
  invalidate it. Mask framebuffers retain full output resolution.
- Mutter patch 59 stores each surface's frame-callback queue link. Repeated
  commits and surface destruction no longer search the pending surface list.
  The change adds one pointer per surface and retains callback timing.
- Mutter patch 60 retains the stage's immutable redraw region for the synchronous
  paint context, removing one region clone per painted output. Offscreen
  contexts retain their existing ownership rules.

`GNOBLIN_BUILD_TYPE` now defaults to `debugoptimized` for local compositor
builds: optimisation level 2 with debug symbols. Both existing build directories
were using optimisation level 0. `GNOBLIN_BUILD_TYPE=debug` remains available.
This build change must be distinguished from the individual native patches
when comparing whole-process CPU samples.

The alpha-mask benchmark exports native capture and uniform-setup counters.
A separate preload probe (`scripts/build-mtk-region-copy-probe.sh`) records actual
MtkRegion copy callsites; `scripts/report-mtk-region-copy-probe.sh` resolves those
against the matching unstripped Clutter library. Instrumented runs establish
operation counts, not uncontaminated CPU timings.

## Separate concurrent work

The configuration rewrite and removal of native GNOME UI are active user tasks
in the same checkout. Their changes are preserved. The final shared build will
include them, so a final whole-process memory difference cannot be attributed
only to this performance pass.

Before the UI-removal changes, the resident-service probe measured 167,742 KiB
PSS. Overview was already a dummy; the transparent native panel had three
container actors and no status entries. There was no measured reason to change
its offscreen redirect. This pass discarded that proposed change. The separate
UI-removal task owns changes to the retained GNOME components.

## Native validation

- Real blur-region transport, cropped output, backdrop damage expansion, cache
  reuse and same-size mask movement passed.
- Gaussian blur and foreground fade together at eight opacity samples, including
  reversal. The sampled pixels passed.
- Native protocol boundaries passed, including 48 callbacks across 16 surfaces,
  repeated commits and destruction before callback delivery.
- Sixteen focused Node tests and the GJS native/legacy redraw lifetime test passed.
- Fresh `git am` replay passed for all 36 Shell and 33 Mutter patches present at
  that verification point. Ten existing malformed mail patches were repaired
  without changing their code. Source/replay differences outside this pass remain
  in pre-existing keyboard and fullscreen-helper work; the new native performance
  paths match replay. This is patch-application validation, not a claim that all
  unrelated dirty source is represented in the committed patch series.

## Compositor CPU and memory comparison

Sequential private Wayland sessions used one 1280x800 virtual output and the
same eight GTK windows. The AMD GPU render node supplied the GBM/EGL renderer;
these were headless GPU sessions, not a physical-output/scanout benchmark. CPU
is a percentage of one logical core over four-second samples. PSS is the
process's proportional resident memory, in MiB.

The before build used pinned Shell 49.6/Mutter 49.5 with optimisation disabled.
The after build uses the same upstream versions with the changes above and
optimisation level 2. It also includes the concurrent Lua configuration and
GNOME UI-removal work. The effects fixture was translated from TOML to equivalent
Lua, retaining all four rules. These measurements therefore describe the shared
build, not an isolated attribution to the native patches.

Stock is official Fedora Shell 49.9/Mutter 49.7, extracted from signed RPMs and
run in a private mount namespace with its own libraries and resources. The
system GNOME installation was not used as stock because it is Gnoblin-patched.
The stock versions and visual effects differ from Gnoblin's; this is a useful
reference, not a controlled comparison of identical rendering features.

| Phase | Gnoblin before CPU | Gnoblin after CPU | Stock GNOME CPU |
| --- | ---: | ---: | ---: |
| Empty | 1.00% | 0.75% | 0.75% |
| Eight static windows | 0.00% | 0.00% | 0.25% |
| Visible animation | 7.75% | 5.50% | 5.75% |
| Partly covered animation | 7.25% | 6.50% | 5.50% |
| Fully covered animation | 0.75% | 0.00% | 1.00% |
| Revealed animation | 7.50% | 5.50% | 4.75% |
| After close | 0.25% | 0.25% | 0.25% |

| Phase | Gnoblin before PSS | Gnoblin after PSS | Stock GNOME PSS |
| --- | ---: | ---: | ---: |
| Empty | 177.6 MiB | 171.1 MiB | 239.9 MiB |
| Eight static windows | 189.5 MiB | 188.1 MiB | 215.9 MiB |
| Visible animation | 188.3 MiB | 185.1 MiB | 215.7 MiB |
| Partly covered animation | 192.5 MiB | 188.6 MiB | 215.7 MiB |
| Fully covered animation | 198.4 MiB | 201.2 MiB | 217.0 MiB |
| Revealed animation | 198.3 MiB | 188.4 MiB | 217.0 MiB |
| After close | 197.0 MiB | 187.2 MiB | 216.0 MiB |

The visible-animation sample used about 29% less compositor CPU than the
previous Gnoblin build. Empty-session PSS was about 29% below stock GNOME and
about 4% below the previous Gnoblin build. With eight static windows it was
about 13% below stock. Gnoblin was not faster in every stock comparison, and
one covered-window memory sample increased relative to the previous build.
Repeated hardware measurements are needed before treating these small timings
as stable percentages.

All three runs rendered about 240 client frames per four-second visible phase,
zero while fully covered, and resumed after reveal. The invalid configuration
ownership run was discarded and is not in these tables. The later Lua-only
reader removal and new console work are not included in this snapshot.

Raw records are in [benchmarks/2026-09-12](benchmarks/2026-09-12). Bingux, other
session processes and GPU allocations are excluded. The active desktop was not
restarted; hardware latency, VRAM, scanout and power remain unmeasured.

Re-run the Gnoblin comparison with:

```sh
GNOBLIN_PREFIX="$PWD/install" \
GNOBLIN_CONFIG="$PWD/tests/performance-effects.lua" \
GNOBLIN_BENCH_REQUIRE_OCCLUSION=1 \
GNOBLIN_BENCH_REPORT=/tmp/gnoblin-desktop-performance.json \
GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/benchmark-desktop.py" \
bash scripts/run-gnome-shell.sh
```

## Reload and window-churn smoke check

The measured build passed `scripts/perf-smoke.sh`: private dirty memory after
settling was 100,360 KiB, changed by -5,372 KiB over the following minute, then
grew by 120 KiB across ten soft reloads. RSS grew by 44 KiB across fifteen
window open/close cycles. These are bounded regression checks, not a proof that
no long-term leak exists.
