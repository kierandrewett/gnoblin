# Extensions

An extension is a separate package that talks to Gnoblin through the owner socket, as an out-of-process program. It is not
linked into the compositor and it is not part of the root Cargo workspace.

- **Own workspace.** Each extension is a Cargo workspace with its own `Cargo.toml`, `Cargo.lock` and target directory. The
  root `Cargo.toml` excludes `extensions`, so `cargo` at the repository root never sees these packages. Build one from its
  own directory: `cd extensions/gnoblin-xr && cargo build`.
- **Own package.** An extension ships as its own package and has its own dependency tree (design/xr-extension.md, D22).
- **Owner socket only.** An extension reads and writes the scene through the owner socket, under the grants its manifest
  asks for. It does not reach into compositor internals.
- **Declared by a power.** A Lua power starts an extension with `[powers.process]` (command, restart policy and restart
  limit), as design/xr-extension.md shows for `gnoblin-xr`.

Current extensions:

| Path          | Status                                                           |
| ------------- | ---------------------------------------------------------------- |
| `gnoblin-xr/` | Scaffold: OpenXR runtime lookup and supervision. See its README. |
