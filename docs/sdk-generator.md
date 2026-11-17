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

| Target       | Generated                                                                                                                                                                                                                                     | Written by hand                                                                                                       |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `rust`       | `crates/gnoblin-core/src/generated/`: schema name constants and the catalog hash, a typed wrapper per node class, `ErrorKind` with its code and symbol conversions                                                                            | composites (`Window::geometry`), closures and trampolines, events, macros                                             |
| `std-dyn`    | `crates/gnoblin-std-dyn/src/generated.rs`: the dynamic call of every std table function, the record readers and writers, the callback and vtable trampolines                                                                                  | `Dyn`, `StdLib` (`dlopen`), the arena, the conversions (`crates/gnoblin-std-dyn/src`)                                 |
| `python`     | `data/python/gnoblin/_generated/`: `classes.py` and `classes.pyi` (a descriptor per member), `abi.py` (constants), `verbs.py` (the `gnoblin` namespace's verbs and signals); `gnoblin/std.py` and `std.pyi`: the std table, typed             | `_base.py` (`Node`, `Property`, `Method`, `Signal`), `_std_base.py`, `_std_sugar.py`, the loader, named subscriptions |
| `typescript` | `data/js/gnoblin-core/generated/`: `schema.ts` (property, method and signal interfaces, the `TREE_CLASSES` table), `abi.ts`, `verbs.ts` (the `gnoblin` namespace's verbs and signals); `data/js/gnoblin-std/generated/`: the std table, typed | `gnoblin-std/native.ts` (the spellings), the class library of `gnoblin-core` and `gnoblin-std`                        |

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

## The std binding

The std ABI's function table is the third source, and it is declarative enough that **no language hand-writes a binding of it**.
The model (`xtask/src/sdk/model.rs`) classifies every table argument and every record field into the shapes a script runtime can
marshal:

| Shape                                  | Example                                                      | A script sees                             |
| -------------------------------------- | ------------------------------------------------------------ | ----------------------------------------- |
| scalar, handle, text                   | `collection_len`, `workspace_by_id`                          | number, class, string                     |
| record in / out, list of records       | `workspace_new(spec)`, `collection_where(terms)`             | dataclass or interface, list              |
| pointer and count (`Array`)            | `GnoblinStdMembers`, the layout context's `children`         | a list                                    |
| `sink` and `emit` (`Sink`)             | the layout context                                           | the list `compute` returns                |
| vtable, `user`, `drop_user` (`Vtable`) | `GnoblinStdLayoutSpec`                                       | an object whose methods are the functions |
| tagged record (`kind` and its fields)  | `GnoblinStdProp`, `GnoblinStdStyleEntry`, `GnoblinStdOption` | a record with a constructor per kind      |
| `begin`, closure, `commit` or `abort`  | `transition_begin` and its two ends                          | `Transition.run(options, body)`           |
| a record with room for `count`         | `layout_ring`                                                | a list                                    |

From it the generator writes three things:

1. **The call layer**, in Rust (`std-dyn`): `StdLib::call(ctx, "collection_len", &[Dyn])`. Each language runtime hand-writes only the
   conversion of its values to and from `Dyn` (`gnoblin-runtime-python/src/std_native.rs`, `gnoblin-runtime-js/src/std_native.rs`) and
   exposes one native, `gnoblin._std.call(name, *args)` and `__gnoblinStd.call(name, args)`. A function added to the table is callable
   from both the day the generator runs.
2. **The typed classes**: `gnoblin/std.py` and `std.pyi`, `data/js/gnoblin-std/generated/*.ts`: a class per handle type and per object
   of the table, a record per `repr(C)` record, an enumeration per constant group.
3. **The layouts and bindings**: one class per entry of `gnoblin_std_abi::layouts::{LAYOUT_KINDS, BINDING_KINDS}` with its options as
   properties (defaults, bounds and enumerations from the table), its actions as methods, and `attach`/`start`.

A hand-written layer remains, on purpose small: the spellings in `_std_sugar.py` and `gnoblin-std/native.ts` (`len(collection)`,
`clients()`, the tile rounding of `Placement.tile`, a workspace in one call). Nothing in them is behaviour of the desktop.

The proof is `crates/gnoblin-runtime-python/tests/shells.rs` and `crates/gnoblin-runtime-js/tests/shells.rs`: example 3 (a workspace with
a layout of the shell's own, a listener on the clients, a chord) and the built-in layouts run through the real loader over the fake
compositor and give the rectangles of the native Rust shells.
