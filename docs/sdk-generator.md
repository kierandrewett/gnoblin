# The SDK generator

Every language SDK of Gnoblin is written by one command from three sources of truth, so the SDKs cannot drift from the
compositor and adding a language is one file.

```sh
cargo xtask gen-sdks            # write every generated file
cargo xtask gen-sdks --check    # fail when a generated file is stale (CI runs this)
cargo xtask gen-sdks --target python
```

## The sources

| Source                                               | Says                                                                                          |
| ---------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| the schema catalog (`crates/gnoblin-schema/schema`)  | classes, properties, methods, signals and their types, defaults, read-only and animated flags |
| the core ABI (`crates/gnoblin-core-abi`, `types.rs`) | constants, handles, callbacks, `repr(C)` records and the function table of `libgnoblin-core`  |
| the std ABI (`crates/gnoblin-std-abi`, `types.rs`)   | the same for `libgnoblin-std`; its table is `object_verb` named, so it groups into classes    |

The ABI crates declare themselves as data (`reflect`), so the generator reads the compiler's own layouts and docs, never source text.
`Catalog::fingerprint()` hashes everything the generators read from the catalog. The core reports it as `gnoblin_catalog_hash`,
and every generated file names it in its banner.

## What is generated, and what is not

| Target       | Generated                                                                                                                                                          | Written by hand                                                                      |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------ |
| `rust`       | `crates/gnoblin-core/src/generated/`: schema name constants and the catalog hash, a typed wrapper per node class, `ErrorKind` with its code and symbol conversions | composites (`Window::geometry`), closures and trampolines, events, macros            |
| `python`     | `data/python/gnoblin/_generated/`: `classes.py` and `classes.pyi` (a descriptor per member), `abi.py` (constants)                                                  | `_base.py` (`Node`, `Property`, `Method`, `Signal`), the loader, named subscriptions |
| `typescript` | `data/js/gnoblin-core/generated/`: `schema.ts` (property, method and signal interfaces, the `TREE_CLASSES` table), `abi.ts`                                        | the class library of `gnoblin-core` and `gnoblin-std`                                |

A generated file starts with a banner that names the command and the catalog hash. Generated Rust passes through the pinned
`rustfmt`; generated Python and TypeScript are excluded from ruff, prettier and the pre-commit hooks.

## Freshness

`cargo xtask gen-sdks --check` regenerates in memory and compares with the tree. It fails on a changed file and on a file in a
generated directory that no target produces any more. `just gen-sdks-check` runs it and is part of `just workspace-ci`; the unit
test `sdk::tests::the_committed_sdks_are_fresh` makes `cargo test -p xtask` fail on drift too.

## Adding a language

1. Add `xtask/src/sdk/<language>.rs` with a type that implements `Target`: `name()`, `owned_dirs()` (directories whose every
   file is generated) and `generate(&SdkSpec) -> Vec<File>`.
2. Add one line to `targets()` in `xtask/src/sdk/mod.rs`.
3. Read the specification, not source text. `SdkSpec` holds the catalog (`node_classes`, objects, members, types) and, per library
   (`core`, `std`): constant groups (`ConstGroup::type_name` gives `EventKind` or `PlacementFlags`), handle typedefs, callbacks, records
   with each field classified as a scalar, handle, text, nested record, or an unrepresentable pointer, and the function table with every
   argument classified (`Arg::Text`, `Arg::Records`, `Arg::Out`, `Arg::Callback`, ...), its return, its object and its receiver.
   `text.rs` has the casing helpers and an indenting writer.
4. Run `cargo xtask gen-sdks`, commit the output, and add the generated directory to the formatter excludes if the language has
   a formatter that would rewrite it.

The three targets above are the proof: they share the model and nothing else.

## Not generated yet

The std ABI's function table is classified (`SdkSpec::library("std")`) but the Python and TypeScript std bindings over it are not
written: see `design/modular-framework.md`, section 25.
