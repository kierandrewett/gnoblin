# Lane issues

The epic and one issue per lane for the Rust compositor rewrite, as a checklist. The lane list is the table under
"Workstreams" in [rust-compositor.md](rust-compositor.md); the dependency edges are its flowchart.

## Why this is a file and not Beads issues yet

[AGENTS.md](../AGENTS.md) says to file work with `bd create` and publish it with `./scripts/gnoblin-issues push <id>`. That
could not be done cleanly from the Phase0Docs worktree on 2026-11-15:

- `bd list` in the worktree answers `no beads database found`. The embedded Dolt database is ignored by Git, and this
  worktree has only the shareable `.beads/config.yaml` and `.beads/metadata.json`. `./scripts/gnoblin-issues status`
  fails the same way, because it runs `bd github`.
- The database that exists is the one under `gnoblin-rust-wt/.beads`. `BEADS_DIR=<that directory> bd list` reads it, but
  `bd create` writes to it and appends to its tracked `.beads/interactions.jsonl`, which is dirty there and not this
  lane's. Another lane's worktree is not this lane's to edit.
- `bd init` in this worktree would start a second database for the same project, with its own ids, beside the one the
  other lanes use.

So the list is here, complete enough to paste. Whoever owns the main checkout's Beads database runs the commands under
"Publish", then ticks the boxes and replaces the placeholders with the ids `bd` returns.

## Epic

- [ ] **Rust compositor rewrite (0.2.0)**: replace the patched Mutter with a Rust and Smithay compositor, in the phases
      of [rust-compositor.md](rust-compositor.md). It is done when the parity report passes on the candidate for every
      scenario the plan names, and the Phase 0 checklist and every later phase gate are met. Progress ledger:
      [rust-compositor-progress.md](rust-compositor-progress.md).

## Lanes

Each lane issue is a child of the epic. "Blocked by" is the flowchart's incoming edges; "Starts in" is the phase column.
The generic gate for every lane is the plan's: a lane is done when its component card's "done when" items pass in the
parity report, not when its crate compiles.

- [ ] **Lane F: foundations.** Owns the workspace, `gnoblin-schema`, `gnoblin-proto`, CI and `xtask`. Needs nothing.
      Starts in phase 0. Blocks every other lane. Shared types change only through a short design note and an F lane review.
- [ ] **Lane P: parity harness and contracts.** Owns `gnoblin-testkit`, `gnoblin-parity` and `tests/parity`. Needs the F
      workspace. Starts in phase 0. Blocks INT. Open items are in the Phase 0 section of the plan.
- [ ] **Lane OBJ: object model, socket, gnoblinctl.** Owns `gnoblin-model`, `gnoblin-client` and `gnoblinctl`. Needs the F
      schema and proto. Starts in phase 1. Blocks INT and CMP. Done when the generated reference, `gnoblin.d.lua` and the CLI
      come from the schema, IPC 0.2.0 covers every recorded legacy IPC contract sample, and Bingux runs on IPC 0.2.0.
- [ ] **Lane CFG: configuration and Lua.** Owns `gnoblin-config` and `gnoblin-lua`. Needs the F schema. Starts in
      phase 1. Blocks INT. Done when the corpus gives identical accepted documents to the C core, apart from ledger entries
      for intended API changes, and the reload and recovery QEMU runs pass.
- [ ] **Lane SVC: services, portal, session.** Owns `gnoblin-session`, `gnoblin-services` and the portal relay. Needs the
      OBJ client. Starts in phase 1. Blocks INT.
- [ ] **Lane WM: policy core.** Owns `gnoblin-wm`, which is deleted in lane W6. Needs the F schema. Starts in phase 1.
      Blocks INT. Done when the model tests cover the failure modes and the parity scenarios for focus, stacking,
      workspaces, snapping and rules pass.
- [ ] **Lane SEC: lock, authority, capture embargo.** Owns `gnoblin-lock` and `gnoblin-authority`. Needs the failure-mode
      lists. Starts in phase 1. Blocks INT and HST. Done when the model tests show no reachable state where a non-lock
      surface gets input or is captured while not unlocked, and the QEMU lock runs with swaylock and the Qt frame-callback
      locker pass on both targets. The capture-embargo failure-mode list is in the crate docs of `gnoblin-capture`.
- [ ] **Lane ENG: protocol handlers.** Owns `gnoblin-wayland` and `gnoblin-xwayland`. Needs the F workspace. Starts in
      phase 2. Blocks INT and HST. Done when wlcs passes for every protocol it covers, the protocol scenarios match the
      reference, and `wayland-info` lists the same globals, apart from ledger entries.
      [adapter-support-matrices.md](adapter-support-matrices.md) lists the reference session's globals.
- [ ] **Lane BE: backends.** Owns `gnoblin-backend` and `gnoblin-output`. Needs the F workspace. Starts in phase 2. Blocks
      INT and HST.
- [ ] **Lane RND: renderer, effects, animation.** Owns `gnoblin-render`, `gnoblin-capture` and `gnoblin-diag`. Needs the F
      workspace. Starts in phase 2. Blocks INT, CMP and A11Y.
- [ ] **Lane INP: input.** Owns `gnoblin-input`. Needs the F workspace. Starts in phase 2. Blocks INT and A11Y.
- [ ] **Lane INT: integration.** Owns `gnoblin-compositor` and `crates/gnoblin`, and is the only lane that edits them.
      Needs skeleton pieces from WM, ENG, BE, RND and INP. Starts in phase 2. Blocked by WM, ENG, BE, RND, INP, SEC, OBJ, CFG,
      SVC, CMP, A11Y, HST and P. Merges lane work into the skeleton at least once per lane milestone and runs the full parity suite before each
      merge to `main`.
- [ ] **Lane CMP: compatibility crates.** Owns the `compat/` crates, one per path, and their support matrices. Needs the
      OBJ IPC and RND frame export. Starts in phase 3. Blocks INT. Done when each enabled crate runs its real consumer
      against both targets, its support matrix matches the parity result, and `cargo xtask deps` shows no edges between compat
      crates. The matrices are compared with a recorded reference session in
      [adapter-support-matrices.md](adapter-support-matrices.md).
- [ ] **Lane A11Y: accessibility.** Owns `gnoblin-a11y`, the keyboard monitor, overlay accessibility and the Orca
      scenarios. Needs INP and RND. Starts in phase 3. Blocks INT. Done when Orca reads and controls a session through its
      key bindings, each keyboard accessibility mode passes its input scenarios, and Orca can read the recovery overlay.
- [ ] **Lane DOC: docs migration.** Owns `docs/`. Needs the OBJ generated reference. Starts in phase 3. Blocked by INT.
- [ ] **Lane HST: session hosting.** Owns the nested backend acceptance and the bsm manager integration. A nested
      session is an ordinary client and session, so there is no hosting protocol. Needs the BE nested backend and ENG. Starts in phase 4. Blocks INT.
- [ ] **Lane PKG: packaging and distro matrix.** Owns `packaging/` and the release workflow. Needs the INT binary. Starts in
      phase 4. Each target passes the `package-targets.json` gates: package build, install, login and removal. The native
      dependency probe is in `tests/native-probe/`.

## Publish

Run from the main checkout, where the Beads database is. `bd create` prints each id; use the epic's id as `--parent`.

```sh
./scripts/gnoblin-issues sync --pull-only
bd search "Rust compositor rewrite"          # file nothing that already exists
bd create "Rust compositor rewrite (0.2.0)" -t epic -p 1
bd create "Lane F: foundations" -t task --parent <epic> -l lane
# ... one `bd create` per lane above, then publish each id:
./scripts/gnoblin-issues push <id>
# the blocking edges of the flowchart, one per edge:
bd dep <blocker-id> --blocks <blocked-id>
```
