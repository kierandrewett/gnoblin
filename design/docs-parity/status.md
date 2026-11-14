# Docs parity status

The implemented surface is the published `gnoblin.d.ts`: the schema objects and the stock library as
`gnoblinctl types init` writes them (`crates/gnoblin-js-types/tests/golden/gnoblin.d.ts`, kept equal to the render by a
golden test). The docs gate (`cargo run -p xtask -- docs-parity`) checks every name a shell-API or events page documents
against it.

What the gate reads from the pages:

- A dotted `gnoblin.` name (`gnoblin.process.spawn`). It is followed member by member from the `Gnoblin` interface through
  the types it names; a member whose type the file does not declare ends the check.
- A `gnoblin.<object>.<signal>` event name. It must be a key of `GnoblinSignals`; a hyphen spelling means the underscored one.
- A `.method(` call. It must be a method that any class or interface of the file declares. Methods of the language
  (`map`, `log`, `padStart`) and calls on `console`, `Math`, `this` and the other built-ins are not checked.

Pages read: `docs/shell-api/`, `docs/events/`, `docs/user-shell.md` and `docs/user-scripts.md`.

Each name the docs promise but the product does not implement is listed in `allow.toml` with its owner slice. The gate fails on:

- a documented name that is neither implemented nor listed (new drift);
- a listed name that is now implemented or no longer documented (stale entry, remove the row).

The list is empty: the pages describe what the declarations provide, and the APIs the shell does not have (keyboard
grab, text insertion, tablet-pad help, privacy state) are named once on `docs/shell-api/index.md`, not documented as calls.

## Not covered by this gate

- Config keys and D-Bus interfaces are not checked. The `defineConfig` keys have their own test (`crates/gnoblin-js-types/tests/config_keys.rs`).
- A member the runtime implements and the declarations omit fails the gate when a page names it. Fix the declarations,
  not the page.
- `design/docs-parity/{api,config,guides}.md` and `legacy-gnoblin.d.lua.txt` are audits of the Lua-era docs. They are history.
