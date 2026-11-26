# Shells

User-level shells that ship with Gnoblin. They are hand-written programs, not part of any generated SDK.

- `default/shell.ts`: the default shell. `cargo xtask dist` installs it as `share/gnoblin/default-shell/shell.ts`, `gnoblin` runs it
  when the user has no shell of their own, and `gnoblinctl config init` writes it out as a starting point.
- `starters/python/shell.py` and `starters/rust/`: the default desktop in Python and in Rust (the Rust one leaves out the two
  strip-layout focus keys). They are what `gnoblinctl config init --language python` and `--language rust` write. No session
  runs them as a default, so `cargo xtask dist` installs nothing for them: `gnoblinctl` carries their text.
  `starters/rust/Cargo.toml.in` is the manifest of the project it writes, with `@SDK_CORE@` and `@SDK_STD@` for the two SDK
  dependencies.

The Python and Rust starters are written out as the Python and Rust tabs of `docs/recipes/complete-starter-config.md` show them: change
both together.

This is not `examples/`, which holds the numbered examples: the default shell is part of the product.
