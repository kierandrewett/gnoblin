# Reference fixtures: what the TypeScript std did

Recorded from `data/js` by `cargo xtask oracle record` and held by `cargo xtask oracle check`
(`../../ORACLE.md` is the map). Native code compares its output with these files, and never rewrites them.

- `tiling_*.json`, `examples_*.json`, `anim_style_transitions.json`: written by the tests of this crate (`tests/tiling.rs`, `examples.rs`,
  `anim_style.rs`). `tiling_*` holds the script (the language of `gnoblin-core-impl/src/driver_script.rs`) and, for each `step`, the
  rectangle, `tiled`, `visible`, depth, rotation and scale of each window. `anim_style_transitions.json` holds the writes of each step, with
  the nodes the run made renamed by the order they were made.
- every other file is a scenario of `crates/gnoblin-js/tests/oracle/`: `world` (what the compositor held at boot), `source` (the
  `shell.ts` body), and `steps`. A step has `actions` (what the compositor did: open a window, press a key, advance the clock), `log`
  (what the script recorded with `rec`), `writes` (the transactions the shell sent, one array each, an op per line), `diagnostics`, and
  after some steps `trees` (a snapshot of a subtree) and `paints` (the paint lists written to `../../../../gnoblin-js/tests/fixtures/paint/`).
- A node the shell made is `n1`, `n2`... in the order it made them. A paint list is shown as its decoded ops and its bytes in hex.
- Text measurement answers come from the scenario (`answer_measures`: 0.6 of the size a glyph, 1.2 a line), never from a font.
- Numbers the scenario computes itself (`scene_math`) are rounded to ten decimals; compare them within a tolerance.
