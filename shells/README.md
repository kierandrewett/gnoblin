# Shells

User-level shells that ship with Gnoblin. They are hand-written TypeScript programs, not part of any generated SDK.

- `default/shell.ts`: the default shell. `cargo xtask dist` installs it as `share/gnoblin/default-shell/shell.ts`, `gnoblin` runs it
  when the user has no shell of their own, and `gnoblinctl config init` writes it out as a starting point.

This is not `examples/`, which holds the numbered examples: the default shell is part of the product.
