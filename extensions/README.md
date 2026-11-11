# Extensions

An extension is a separate package that talks to Gnoblin through the owner socket, as an out-of-process program. It is not
linked into the compositor and it is not part of the root Cargo workspace.

- **Own workspace.** Each extension is a Cargo workspace with its own `Cargo.toml`, `Cargo.lock` and target directory. The
  root `Cargo.toml` excludes `extensions`, so `cargo` at the repository root never sees these packages. Build one from its
  own directory: `cd extensions/gnoblin-xr && cargo build`.
- **Own package.** An extension ships as its own package and has its own dependency tree (design/xr-extension.md, D22).
- **Owner socket only.** An extension reads and writes the scene through the owner socket, as any same-user client of the
  tree does (the owner predicate in `crates/gnoblin-compositor/src/ipc/privileged.rs`). There are no manifests, powers or
  grants: the D22 extension tier was removed. It does not reach into compositor internals.
- **Started by the guardian.** `gnoblin-xr` is started by the guardian when `GNOBLIN_XR=1` is set, with its own Wayland
  connection for view capture (see design/xr-extension.md).

Current extensions:

| Path          | Status                                                           |
| ------------- | ---------------------------------------------------------------- |
| `gnoblin-xr/` | Scaffold: OpenXR runtime lookup and supervision. See its README. |
