# Model fixtures: listener order and the input matcher

Scenarios of `crates/gnoblin-js/tests/oracle/{model,input}.rs`, laid out as `crates/gnoblin-std-impl/tests/fixtures/reference/README.md`
says. The `log` of each step is the order in which the TypeScript model called listeners and bindings. In the input scenarios a step also
has `claim_calls` (what the shell asked the seat to claim) and `claims_held` (the claims live at the end of the step).
