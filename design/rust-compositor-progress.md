# Rust compositor progress

**Status:** what is on `origin/rust-compositor` at `8bf36b68`. Each commit is
`git log --format=%h -1 -- crates/<crate>/src`, and each count is the number of
`#[test]` declarations in the crate at that commit. A count is declarations, not
a run.

[rust-compositor.md](rust-compositor.md) is the plan. This page is what exists
against it.

| Lane | Crate                                                                                             | What landed                                                         | Commit     | Tests |
| ---- | ------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- | ---------- | ----- |
| F    | `gnoblin-types`                                                                                   | shared primitives and the duplicate-type report                     | `b98dd5cb` | 20    |
| F    | `gnoblin-schema`                                                                                  | schema model, rules, first slice, the `request` object              | `4dedd5ca` | 22    |
| F    | `gnoblin-proto`                                                                                   | framed envelope, handshake, version rule, generated wire types      | `4dedd5ca` | 28    |
| F    | `xtask`                                                                                           | dependency rules, `gen-proto`, `gen-config`, `dupes`, compat report | `4dedd5ca` | 23    |
| P    | `gnoblin-parity`                                                                                  | scenario runner, targets, normaliser, report                        | `c3779eed` | 50    |
| P    | `gnoblin-testkit`                                                                                 | runtime directory, clock, recorder                                  | `c3779eed` | 13    |
| OBJ  | `gnoblin-model`                                                                                   | object graph, revisions, request broker, in-process loopback        | `ebd69a80` | 14    |
| OBJ  | `gnoblin-client`                                                                                  | IPC client                                                          | `15cbc315` | 27    |
| OBJ  | `gnoblinctl`                                                                                      | generated tree, typed arguments, `ipc dump` redaction               | `1412c6a8` | 45    |
| CFG  | `gnoblin-config`                                                                                  | document, merge, salvage, last-good store                           | `0461a103` | 82    |
| CFG  | `gnoblin-config-codegen`                                                                          | the schema generator; its output is tested in `gnoblin-config`      | `0461a103` | 0     |
| CFG  | `gnoblin-lua`                                                                                     | sandboxed host, loader, generated bindings                          | `a3253b75` | 42    |
| SVC  | `gnoblin-session`                                                                                 | guardian supervision state machine                                  | `37056428` | 22    |
| WM   | `gnoblin-wm`                                                                                      | stacking, focus, workspaces, placement, snapping, rules             | `37056428` | 88    |
| SEC  | `gnoblin-lock`                                                                                    | lock and protection-cover machine                                   | `37056428` | 27    |
| SEC  | `gnoblin-authority`                                                                               | attribution, activation, grants                                     | `37056428` | 29    |
| ENG  | `gnoblin-wayland`                                                                                 | globals, xdg-shell, session lock, frame loop                        | `7d718789` | 38    |
| BE   | `gnoblin-backend`                                                                                 | backend trait, headless, software renderer, screenshot readback     | `7d718789` | 45    |
| BE   | `gnoblin-output`                                                                                  | output model, layout, transactional apply                           | `31692467` | 25    |
| RND  | `gnoblin-render`                                                                                  | scene, damage, tween player, cursor selection                       | `ae3b21c7` | 68    |
| INP  | `gnoblin-input`                                                                                   | chords, bindings, gestures, policy                                  | `c0dc1b10` | 65    |
| A11Y | `gnoblin-a11y`                                                                                    | sticky, slow, bounce and mouse keys, magnifier                      | `c0dc1b10` | 34    |
| CMP  | `gnoblin-compat-core`                                                                             | adapter frame, support matrix, refusal                              | `23cc3f86` | 32    |
| CMP  | 17 vendor adapters                                                                                | named after the vendor, no code                                     | `b89fb68d` | 0     |
| INT  | `gnoblin-compositor`, `gnoblin`                                                                   | skeleton, crate doc only                                            | `efe5eed0` | 0     |
| —    | `gnoblin-capture`, `gnoblin-diag`, `gnoblin-services`, `gnoblin-wasm`, `gnoblin-xwayland`, portal | skeleton, crate doc only                                            | `efe5eed0` | 0     |
