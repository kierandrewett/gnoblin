# API reference

The classes, methods, properties and signals a shell uses to read and change the desktop, in the three languages a shell can be
written in. Each language has the reference its own tools produce, so a reader finds it the way they find the reference of any
other library in that language.

| Language   | Reference                                      | Made with                                                   | Covers                                                                 |
| ---------- | ---------------------------------------------- | ----------------------------------------------------------- | ---------------------------------------------------------------------- |
| TypeScript | [TypeScript reference](/reference/typescript/) | [TypeDoc](https://typedoc.org/)                             | The `gnoblin-core` and `gnoblin-std` packages.                         |
| Python     | [Python reference](/reference/python/)         | [Sphinx](https://www.sphinx-doc.org/) with `sphinx-autoapi` | The `gnoblin` package: `gnoblin`, `gnoblin.std` and `gnoblin.objects`. |
| Rust       | [Rust reference](/reference/rust/)             | [rustdoc](https://doc.rust-lang.org/rustdoc/)               | The `gnoblin-core`, `gnoblin-std` and `gnoblin-macros` crates.         |

The core library is the compositor's own objects: windows, outputs, input, the session and processes. The std library is the desktop
above it: workspaces, layouts, the scene, styles and widgets. A shell is complete on the core alone.

The guides explain how to use the classes: [TypeScript shell](../typescript-shell.md), [Python shell](../python-shell.md) and
[Rust shell](../rust-shell.md).

## What the references have in common

- Each reference is written from the declarations that the SDK itself ships: the `.d.ts` files an editor reads for TypeScript, the
  stubs a type checker reads for Python, and the crates a shell builds against for Rust. A member listed there is a member the SDK has.
- A description is the comment above the class or member in the schema files under `crates/gnoblin-core-abi/schema` and
  `crates/gnoblin-std-abi/schema`, so it says the same thing in all three languages and in an editor's hover text. To change one, edit that
  comment. [The SDK generator](../sdk-generator.md) says how it reaches each language.
- A name is spelled the way its language spells it: `isIdle` in TypeScript, `is_idle` in Python and `is_idle()` in Rust.

## Link to a type

The references are not pages of this site, so a link to one of them names its file:

| Language   | The `IdleWatch` class                                            |
| ---------- | ---------------------------------------------------------------- |
| TypeScript | `/reference/typescript/classes/gnoblin-core.IdleWatch.html`      |
| Python     | `/reference/python/autoapi/gnoblin/objects/model/IdleWatch.html` |
| Rust       | `/reference/rust/gnoblin_core/objects/struct.IdleWatch.html`     |
