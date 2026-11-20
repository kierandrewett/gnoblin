# Paint fixtures: the bytes of each widget in each state

`<scenario>.<state>.bin` is the paint list of every canvas under the node the scenario named, in tree order, each as a little-endian
`u32` length and the bytes (`GNPL`, version, op count, ops). The scenario file `crates/gnoblin-std-impl/tests/fixtures/reference/<scenario>.json`
lists, in the `paints` of the step that took it, each canvas with its rectangle and decoded ops. Written by `cargo xtask oracle record`.
