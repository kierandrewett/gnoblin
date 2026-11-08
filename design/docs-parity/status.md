# Docs parity status

The implemented surface (schema and bindings, emitted as `docs/public/gnoblin.d.lua`) is the source of truth.
The docs gate (`cargo run -p xtask -- docs-parity`) checks every documented name against it.

Each name the docs promise but the product does not implement is listed in `allow.toml` with its owner slice.
The gate fails on:

- a documented name that is neither implemented nor listed (new drift);
- a listed name that is now implemented (stale entry, remove the row).

## Counts (gate run, 2026-11-08)

- Documented names checked: 374
- Implemented and documented: 145
- Documented but not implemented (listed in `allow.toml`): 229
- Unlisted drift: 0
- Stale allow entries: 0

## Burn-down by owner slice

| Owner      |    Rows | Meaning                                                                                                  |
| ---------- | ------: | -------------------------------------------------------------------------------------------------------- |
| docs wrong |      81 | No slice in `design/runtime-surface-plan.md` plans this. Plan it in a slice, or remove it from the docs. |
| S2         |      37 | Planned feature, owned by slice S2.                                                                      |
| S7         |      34 | Planned feature, owned by slice S7.                                                                      |
| S8         |      28 | Planned feature, owned by slice S8.                                                                      |
| S6         |      18 | Planned feature, owned by slice S6.                                                                      |
| S10        |      12 | Planned feature, owned by slice S10.                                                                     |
| S5         |       9 | Planned feature, owned by slice S5.                                                                      |
| S4         |       7 | Planned feature, owned by slice S4.                                                                      |
| S1         |       3 | Planned feature, owned by slice S1.                                                                      |
| **Total**  | **229** |                                                                                                          |

The 25 shell-api pages that list a not-yet-available member carry a
`> **Status:** not available yet` line under the title. Remove the line when the member lands and its
allow row is deleted.

## Not covered by this gate

- Config keys and D-Bus interfaces are not checked yet. They need their own source of truth before they can be gated.
- `design/docs-parity/legacy-gnoblin.d.lua.txt` is the hand-written target surface from before the generated stub. It is a reference for the burn-down, not a published file.
