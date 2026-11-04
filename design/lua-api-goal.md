# Goal: overhaul the Gnoblin Lua API into an object-centric API

Make the Lua runtime feel like a living runtime that shell developers can do real work in. Replace snapshots plus loose
functions that take IDs with live handles, explicit operations and signals.

## Model

- Every thing a shell touches (window, layer surface, output, workspace, process, prompt request, shortcut, launch,
  tween) is an object with a stable identity and a lifetime. Objects that end report it with a signal.
- Each object type declares its own properties (typed, readable, observable, some writable), methods and signals.
  Objects do not all share the same capabilities. A member exists only where it makes sense, and unsupported requests
  fail with a clear error.
- Generic services (watch, tween, find) work through a type's declared property schema. They apply only to types and
  properties that declare support.
- The Lua contract is defined on its own. It does not depend on Mutter or GObject internals.

## Rules

1. Positional functions. No `fn {a = 1}` calls. Optional arguments go last.
2. A write is an explicit call that returns an Operation. Never hidden assignment. Reads carry a revision.
3. One lifecycle for every request: offered, answered or ended, one `finished` outcome. Keep per-request answer schemas
   and permissions.
4. Separate attribution (the compositor proves who started a window or process) from authority (what they may do). Keep
   the focus-stealing, lock and secret boundaries.
5. Aim for one descriptor table that drives the Lua stub, the socket protocol and gnoblinctl, where the audit shows the
   contracts fit. Remove aliases and duplicates. Annotate every registered function.
6. When two or more real shell tasks need the same thing, build one shared primitive for it. Keep domain-specific
   behaviour explicit. Do not add an abstraction for a single use.

## Process

- Start from `design/shell-api-redesign.md`. The audit evidence is in it. Check each finding against the source before
  acting on it.
- Work in small phases. Each phase is its own PR and leaves the tree working. Cut over every caller and doc in the same
  PR. Do not keep shims.
- Verify behaviour in a real session (build, run, observe). Passing stubs or unit tests alone are not enough.
- Update `docs/` and the generated `gnoblin.d.lua` with each change. Run
  `scripts/markdown-style.py review --fail-on-flags` on changed docs.

## Out of scope until proven

- Coroutines or `await`: needs a scheduler design first.
- Per-frame Lua animation: tweens run natively.
- Mixing native and Lua steps in one tween.
