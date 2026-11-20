# The SDK generator

Every language SDK of Gnoblin is written by one library, `gnoblin-sdkgen`, from three sources of truth, so the SDKs cannot drift from
the compositor and adding a language is one file. **Nothing it writes is committed.** A build renders what it embeds, an install
renders what an editor reads, and a person who wants to read a generated file asks for a copy.

```sh
cargo xtask gen-sdks                    # a readable copy of every generated file, in target/sdk
cargo xtask gen-sdks --out DIR          # the same, somewhere else
cargo xtask gen-sdks --target python    # one target
cargo xtask gen-sdks --check            # prove the generator is sound (CI runs this)
cargo xtask gen-sdks --update           # record an additive change of the catalogue in the surface locks
```

## Where the files go

| Call site                              | When        | Writes                                                                                                     |
| -------------------------------------- | ----------- | ---------------------------------------------------------------------------------------------------------- |
| `build.rs` of `gnoblin-core`           | every build | `OUT_DIR/rust-core/generated.rs`, which the crate's `lib.rs` includes                                      |
| `build.rs` of `gnoblin-std-dyn`        | every build | `OUT_DIR/rust-std-dyn/generated.rs`, which the crate's `lib.rs` includes                                   |
| `build.rs` of `gnoblin-js`             | every build | `OUT_DIR/js/**`, and `stock_embedded.rs`, the table the crate's `stock.rs` serves the modules from         |
| `build.rs` of `gnoblin-runtime-python` | every build | `OUT_DIR/python/**`, and `python_embedded.rs`, the table the crate's `embedded.rs` serves the modules from |
| `cargo xtask dist install`             | install     | the same TypeScript and Python files under `share/gnoblin/js` and `share/gnoblin/python`, for editors      |
| `cargo xtask gen-sdks [--out DIR]`     | by hand     | a copy of everything, with the Rust passed through the pinned `rustfmt`                                    |

A `build.rs` runs again when the schema directory, an ABI crate or the generator changes (`cargo:rerun-if-changed` and the
build-dependency edge), and only then. It renders only its own target and rewrites a file only when the text changed, so a build
that did not touch the catalogue touches no file. `gnoblin-sdkgen` is a build dependency of those four crates and of nothing else
(`cargo xtask deps` enforces it, and that the generator names only the schema and the two ABI crates).

Generated Rust is not formatted in a build: the compiler does not care, and `gen-sdks` formats the copy a person reads.

### What the runtimes do with it

- **Rust.** `gnoblin-core` and `gnoblin-std-dyn` `include!` the file inside an inline module, so `crate::generated::...` is the path
  it always was.
- **JavaScript and TypeScript.** `gnoblin-js` embeds `gnoblin-core/generated/*` and `gnoblin-std/generated/*` and serves them under
  those paths beside the files of a package that is installed. A generated module wins over a file of the same path in the library
  directory, so what runs is always what this runtime was built with. `stock_sources_in` and `editor_library_in` list both, which is why
  `gnoblinctl types init` and `gnoblinctl config check --types` give an editor the same modules the shell runs. A prefix that has no
  std package serves no std module, generated or not, so example 17 keeps its error.
- **Python.** `gnoblin-runtime-python` embeds `gnoblin._generated.*` and `gnoblin.std` and puts a finder first on `sys.meta_path` of
  its own interpreter. The hand-written package on disk finds them through it, so an older file of the same name in a prefix is never
  imported.

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

| Target       | Generated                                                                                                                                                                                                                           | Written by hand                                                                                                       |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `rust`       | `rust-core/generated.rs`: schema name constants and the catalog hash, a typed wrapper per node class, `ErrorKind` with its code and symbol conversions                                                                              | composites (`Window::geometry`), closures and trampolines, events, macros                                             |
| `std-dyn`    | `rust-std-dyn/generated.rs`: the dynamic call of every std table function, the record readers and writers, the callback and vtable trampolines                                                                                      | `Dyn`, `StdLib` (`dlopen`), the arena, the conversions (`crates/gnoblin-std-dyn/src`)                                 |
| `python`     | `python/gnoblin/_generated/`: `classes.py` and `classes.pyi` (a descriptor per member), `abi.py` (constants), `verbs.py` (the `gnoblin` namespace's verbs and signals); `gnoblin/std.py` and `std.pyi`: the std table, typed        | `_base.py` (`Node`, `Property`, `Method`, `Signal`), `_std_base.py`, `_std_sugar.py`, the loader, named subscriptions |
| `typescript` | `js/gnoblin-core/generated/`: `schema.ts` (property, method and signal interfaces, the `TREE_CLASSES` table), `abi.ts`, `verbs.ts` (the `gnoblin` namespace's verbs and signals); `js/gnoblin-std/generated/`: the std table, typed | `gnoblin-std/native.ts` (the spellings), the class library of `gnoblin-core` and `gnoblin-std`                        |

A generated file starts with a banner that names the generator, the target and the catalogue hash.

## The namespaces a library serves

`gnoblin.model` (the core) and `gnoblin.std` (the std library) are not the compositor's: a library serves their classes through the object
protocol (`GnoblinObjectApi`, [the ABI page](abi-compatibility.md)). Their catalogs are schema files beside the ABI crates,
`crates/gnoblin-core-abi/schema/` and `crates/gnoblin-std-abi/schema/`, one file per module, each with `local = true`. A local file may use
the keys a compositor file may not: `kind` (`handle`, `record`, `interface`, `service`), `ctor`, `retained`, `[[enum]]`, `[[callback]]`,
`returns`, `required` and `default = "native"` on a method of an interface, `optional` on a property (a record may leave the field out),
`python = { dunder = ".." }`, `inert`, `abi = "fn:<name>"`,
`export = { js, py, rust }`, and a `base` of another namespace's class. The wire never carries such a namespace and the `gnoblin`
catalog hash (`gnoblin_catalog_hash`) does not include it; the namespace has a hash of its own (`Catalog::namespace_fingerprint`), which
its object table reports.

| Target       | Writes                                                                                               | Consumed by                                    |
| ------------ | ---------------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| `objects`    | `rust-objects/{model,std}_catalog.rs`: the static `CATALOG` of each namespace and its `CATALOG_HASH` | `build.rs` of `gnoblin-core-impl`, `-std-impl` |
| `object-api` | `rust-objects/{model,std}_api.rs`: the typed Rust API of each namespace, over the protocol's client  | `build.rs` of `gnoblin-core`, `gnoblin-std`    |

The typed Rust API (`gnoblin_core::objects`, `gnoblin_std::objects`; a shell gets the handle from `g.objects()` and `std.objects()`) is one
Rust type for each catalog type:

- an **enum** is a Rust enum whose discriminants are the ids the protocol numbers its values with (`ALL`, `as_str`);
- a **record** is a struct of its properties in catalog order. A field with a schema default starts at it, an `optional` field is an
  `Option` that starts as `None`, and any other field is always there: a record that has some is made with `new`, and one that has none
  has `Default`;
- a **handle** or a **service** is a struct that holds a reference and releases it when dropped: a constructor (`new`, or `get` for a
  service), a getter and a setter for each property (an object or a record reads as an `Option`), a method for each method, an associated
  function that takes the `Objects` for each `function`, and `on_<signal>` and `on_<property>_changed` listeners that end when the
  returned guard is dropped (`keep` lets one last until the generation ends);
- an **interface** is a struct of handlers, one boxed closure per method (an `Option` of one for a method the library has a native
  default for), and `implement` makes the object the library calls them through;
- a **callback** type is a Rust closure wherever a property, a constructor or a method takes one.

The ids of a class are looked up by name the first time a call needs them, so a shell built against an older catalog runs against a newer
library and fails only on a name the library lacks. A member whose type names a class of another namespace is refused at build time with a
message that says so: a typed reference across two libraries needs both libraries' `Objects`, which no catalog needs yet.

A crate that serves a catalog of its own, as a test fixture, renders the same two files with `gnoblin_sdkgen::emit_catalog_for_build`
(`crates/gnoblin-object/fixtures/probe` covers one class of every kind, so the generated tables and the generated typed API are compiled
and run by its tests).

A library implements the classes of its catalog with a `gnoblin_object::server::Builder` (`crates/gnoblin-std-impl/src/object/`), and the
two-way completeness test of the library (`Library::problems`) fails when a class or a member of the catalog has no implementation, or an
implementation names a member the catalog lacks. Class ids are positions in the catalog (ordered by name), and a member's id is its
position in its class's list, a base class's members first, so a base member has the same id in every derived class.

The surface lock of a local namespace is one file per catalog file: `crates/gnoblin-{core,std}-abi/schema/lock/<module>.lock`. A lane that
adds classes to its module edits that module's file and that module's lock, and never `abi.lock`. `--update` records an additive change.

## What `--check` proves

No file is committed, so there is no committed file to be stale. What `cargo xtask gen-sdks --check` (and `just gen-sdks-check`, part of
`just workspace-ci` and the `gen-sdks` job of CI) proves instead:

1. **Every target renders, deterministically.** The generator renders twice and compares the bytes. It reads no clock, environment
   variable or locale, and orders everything by name.
2. **The Python compiles.** The rendered modules go through `python3 -m py_compile` (skipped, and said so, where there is no `python3`).
   Cargo checks the Rust; `scripts/js-typecheck.sh` checks the TypeScript against the hand-written library.
3. **The public surface is the one on record.** `crates/gnoblin-schema/schema/lock/<namespace>.lock` lists every object and member
   of each namespace with the version that added it, one line each:

    ```text
    member | tree | window.title | property string rw | 0.1.0
    ```

    The comparison is the one `abi-check` makes for the C ABI. A recorded entry that is gone or changed fails when the lock's
    `released` line covers its `since`, with no way to bless it inside the major, and is only a stale lock before the release. A new
    entry is additive: `cargo xtask gen-sdks --update` records it. A hand edit of a lock is caught the same way. The SDKs of every
    language follow the catalogue, so a lock that holds proves they keep every released member.

4. **The C ABI is the one on record.** The same command runs what `cargo xtask abi-check` runs: `abi.lock` of both libraries, and the
   golden shells of example 12.

The unit tests of the crate (`cargo test -p gnoblin-sdkgen`) cover the same ground from inside: two renders are equal, every file
carries the banner and the hash, no two targets write one path, and a surface lock round-trips.

## Adding a language

1. Add `crates/gnoblin-sdkgen/src/<language>.rs` with a type that implements `Target`: `name()` and `generate(&SdkSpec) -> Vec<File>`.
   A file's path is logical (`python/gnoblin/std.py`): it is relative to whatever directory the caller writes the files under.
2. Add one line to `targets()` in `crates/gnoblin-sdkgen/src/lib.rs`.
3. Read the specification, not source text. `SdkSpec` holds the catalog (`node_classes`, objects, members, types) and, per library
   (`core`, `std`): constant groups (`ConstGroup::type_name` gives `EventKind` or `PlacementFlags`), handle typedefs, callbacks, records
   with each field classified as a scalar, handle, text, nested record, or an unrepresentable pointer, and the function table with every
   argument classified (`Arg::Text`, `Arg::Records`, `Arg::Out`, `Arg::Callback`, ...), its return, its object and its receiver.
   `text.rs` has the casing helpers and an indenting writer.
4. Call `gnoblin_sdkgen::emit_for_build(&["<language>"])` from the `build.rs` of the crate that embeds the output, and add the crate
   to `SDKGEN_USERS` in `xtask/src/deps.rs`.

The three targets above are the proof: they share the model and nothing else.

## The std binding

The std ABI's function table is the third source, and it is declarative enough that **no language hand-writes a binding of it**.
The model (`crates/gnoblin-sdkgen/src/model.rs`) classifies every table argument and every record field into the shapes a script runtime can
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
2. **The typed classes**: `gnoblin/std.py` and `std.pyi`, `gnoblin-std/generated/*.ts`: a class per handle type and per object
   of the table, a record per `repr(C)` record, an enumeration per constant group.
3. **The layouts and bindings**: one class per entry of `gnoblin_std_abi::layouts::{LAYOUT_KINDS, BINDING_KINDS}` with its options as
   properties (defaults, bounds and enumerations from the table), its actions as methods, and `attach`/`start`.

A record that holds a pointer and its count as a field (`GnoblinStdLayoutQuery.children`) is written by every language as a list, and a
record that is only a list (`GnoblinStdPlacements`) is read as one. `builtin_*` is the built-in layout that belongs to no workspace
(`Builtin` in TypeScript and Python): the library computes it for a shell that keeps its own workspaces.

A hand-written layer remains, on purpose small: the spellings in `_std_sugar.py` and `gnoblin-std/native.ts` (`len(collection)`,
`clients()`, the tile rounding of `Placement.tile`, a workspace in one call). Nothing in them is behaviour of the desktop.

The proof is `crates/gnoblin-runtime-python/tests/shells.rs` and `crates/gnoblin-runtime-js/tests/shells.rs`: example 3 (a workspace with
a layout of the shell's own, a listener on the clients, a chord) and the built-in layouts run through the real loader over the fake
compositor and give the rectangles of the native Rust shells.
