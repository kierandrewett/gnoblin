# The SDK generator

Every language SDK of Gnoblin is written by one library, `gnoblin-sdkgen`, from three sources of truth, so the SDKs cannot drift from
the compositor and adding a language is one file.

**Nothing it writes is committed.** A build renders what it embeds, an install renders what an editor reads, and a person who wants to
read a generated file asks for a copy.

```sh
cargo xtask gen-sdks                    # a readable copy of every generated file, in target/sdk
cargo xtask gen-sdks --out DIR          # the same, somewhere else
cargo xtask gen-sdks --target python    # one target
cargo xtask gen-sdks --check            # prove the generator is sound (CI runs this)
cargo xtask gen-sdks --update           # record an additive change of the catalogue in the surface locks
```

## The API reference

The [API reference](reference/index.md) of the website is the output of each language's own documentation tool, run over the SDK that
language installs: TypeDoc over the declarations, Sphinx over the Python stubs and rustdoc over the Rust crates. The generator is what
makes them agree:

- The descriptions are the comments above each class, property, method, signal and enum in the schema files. The `object-types` target
  writes them as TSDoc comments and Python docstrings, and the `object-api` target writes them as Rust doc comments, so a description is
  the same sentence in the three references and in an editor's hover text. A comment above a member of an `[[augment]]` describes the
  member the library adds to the core's class.
- A member without a comment has the generic one-line description its kind has: "The property `x`." or "The method `y`."
- `cargo xtask docs-reference` writes the declarations the TypeScript and Python tools read into `infra/site/.generated/sdk`, and
  `yarn build` in `infra/site` runs the three tools. Nothing it writes is committed. [Documentation site](documentation-site.md) has the
  details.

## Where the files go

| Call site                              | When        | Writes                                                                                                     |
| -------------------------------------- | ----------- | ---------------------------------------------------------------------------------------------------------- |
| `build.rs` of `gnoblin-core`           | every build | `OUT_DIR/rust-core/generated.rs`, which the crate's `lib.rs` includes                                      |
| `build.rs` of `gnoblin-std-dyn`        | every build | `OUT_DIR/rust-std-dyn/generated.rs`, which the crate's `lib.rs` includes                                   |
| `build.rs` of `gnoblin-js`             | every build | `OUT_DIR/js/**`, and `stock_embedded.rs`, the table the crate's `stock.rs` serves the modules from         |
| `build.rs` of `gnoblin-runtime-python` | every build | `OUT_DIR/python/**`, and `python_embedded.rs`, the table the crate's `embedded.rs` serves the modules from |
| `cargo xtask dist install`             | install     | the TypeScript and Python files, and the declarations and stubs of the object classes, for editors         |
| `cargo xtask gen-sdks [--out DIR]`     | by hand     | a copy of everything, with the Rust passed through the pinned `rustfmt`                                    |

A `build.rs` runs again when the schema directory, an ABI crate or the generator changes (`cargo:rerun-if-changed` and the
build-dependency edge), and only then.

It renders only its own target and rewrites a file only when the text changed, so a build that did not touch the catalogue touches no
file. `gnoblin-sdkgen` is a build dependency of those four crates and of nothing else
(`cargo xtask deps` enforces it, and that the generator names only the schema and the two ABI crates).

Generated Rust is not formatted in a build: the compiler does not care, and `gen-sdks` formats the copy a person reads.

### What the runtimes do with it

- **Rust.** `gnoblin-core` and `gnoblin-std-dyn` `include!` the file inside an inline module, so `crate::generated::...` is the path
  it always was.
- **JavaScript and TypeScript.** `gnoblin-js` embeds `gnoblin-core/generated/*` and `gnoblin-std/generated/*` and serves them under
  those paths beside the files of a package that is installed. A generated module wins over a file of the same path in the library
  directory, so what runs is always what this runtime was built with.

    `stock_sources_in` and `editor_library_in` list both, which is why
    `gnoblinctl types init` and `gnoblinctl config check --types` give an editor the same modules the shell runs. A prefix that has no
    std package serves no std module, generated or not, so example 17 keeps its error.

- **Python.** `gnoblin-runtime-python` serves the whole package to its own interpreter, through a finder first on `sys.meta_path`:
    - It embeds the generated modules: `gnoblin._generated.*`, `gnoblin.std` and the object classes `gnoblin.objects.std` and
      `gnoblin.objects.model`.
    - The rest of the package is Rust in the same crate: `gnoblin`, `gnoblin._base`, `gnoblin.api`, `gnoblin.events`, `gnoblin.errors`,
      `gnoblin._std_base` and `gnoblin._std_sugar`.
    - Nothing of the package is read from disk, and an older file of the same name in a prefix is never imported.

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

| Target       | Generated                                                                                                                                                                                                                           | Written by hand                                                                                                    |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `rust`       | `rust-core/generated.rs`: schema name constants and the catalog hash, a typed wrapper per node class, `ErrorKind` with its code and symbol conversions                                                                              | composites (`Window::geometry`), closures and trampolines, events, macros                                          |
| `std-dyn`    | `rust-std-dyn/generated.rs`: the dynamic call of every std table function, the record readers and writers, the callback and vtable trampolines                                                                                      | `Dyn`, `StdLib` (`dlopen`), the arena, the conversions (`crates/gnoblin-std-dyn/src`)                              |
| `python`     | `python/gnoblin/_generated/`: `classes.py` and `classes.pyi` (a descriptor per member), `abi.py` (constants), `verbs.py` (the `gnoblin` namespace's verbs and signals); `gnoblin/std.py` and `std.pyi`: the std table, typed        | Rust in `gnoblin-runtime-python`: `Node` and the descriptors, `_std_base`, `_std_sugar`, the loader, subscriptions |
| `typescript` | `js/gnoblin-core/generated/`: `schema.ts` (property, method and signal interfaces, the `TREE_CLASSES` table), `abi.ts`, `verbs.ts` (the `gnoblin` namespace's verbs and signals); `js/gnoblin-std/generated/`: the std table, typed | `gnoblin-std/native.ts` (the spellings), the class library of `gnoblin-core` and `gnoblin-std`                     |

A generated file starts with a banner that names the generator, the target and the catalogue hash.

## The namespaces a library serves

`gnoblin.model` (the core) and `gnoblin.std` (the std library) are not the compositor's: a library serves their classes through the
object protocol (`GnoblinObjectApi`, [the ABI page](abi-compatibility.md)). Their catalogs are schema files beside the ABI crates,
the two ABI crates' `schema/` directories, one file per module, each with `local = true`.

A local file may use keys a compositor file may not:

- `kind` (`handle`, `record`, `interface`, `service`), `ctor`, `retained`, `[[enum]]` and `[[callback]]`.
- `returns`, `required` and `default = "native"` on a method of an interface.
- `optional` on a property: a record may leave the field out.
- `always` on a property of an object type: the library always has an object to answer, so the read has no absent case. Rust, TypeScript
  and Python read `T` where they read `Option<T>`, `T | undefined` and `T | None` without it, and a Python class refuses a none answer
  with a `GnoblinError` that names the property.
    - It is a declaration for the SDKs, on no wire and in no catalog hash.
    - The roots of `gnoblin` carry it, and so does `InputManager.primary`, whose read is refused when there is no seat.
- `optional` on a method: its `object:` result may be none, as `Option<T>` in Rust, `T | undefined` in TypeScript and `T | None` in
  Python. The library flags the method `MEMBER_OPTIONAL` and refuses a none answer from a method that is not marked.
- `optional` on a method of an interface, or on a `[[callback]]`: the shell's method or function may answer nothing (`T | void` in
  TypeScript, `T | None` in Python; a Rust closure always answers). The library reads nothing as the answer the comment gives. The
  core's bindings use it: `key_action`, `on_press`, `on_start` and `on_begin` decline only with `false`.
- `python = { dunder = ".." }`, `inert`, `abi = "fn:<name>"` and `export = { js, py, rust }`.
- `number = "<field>"` on a record that is not a family of variants: a bare number stands for the record with only that number field.
  `corner_radius` says `all`, so a script may write `style.cornerRadius = 12`. TypeScript types the input `CornerRadiusInit | number`
  and Python `CornerRadius | float`; Rust takes the record.
- A family of variants (`tag` with `[[object.variant]]`) also takes an object with one key, a variant's tag: the value is the variant's
  one property (`{ blur: 24 }`), or its options when it has more (`{ shader: { source, params } }`). TypeScript declares the forms as
  `<Family>Literal` (`FilterLiteral`); Python takes a dict with one key; Rust takes the variant.
- `find = "Kind"` on a method of a service that returns a list of objects and takes one optional record (its filter). Each method of a
  service names a kind of its own. In the core, `find_clients` is `find = "Window"` and `find_outputs` is `find = "Output"`. The service
  then gets:
    - `find(kind, filter)` in JavaScript, TypeScript and Python, where `filter` is a record, a table of field values or a function. It
      calls the method of the kind, and refuses a kind it has no method for with an error that lists the kinds that work.
    - `find::<T>(filter)` and `find_matching::<T>(function)` in Rust, over the trait `Finds` that each kind implements. The method
      of the kind takes only the record, because the object protocol moves values and not closures into a call that answers a list, so
      `find_matching` lists the kind and keeps the objects the closure accepts, as the script runtimes do for a function.
- A `base` of another namespace's class.

The wire never carries such a namespace and the `gnoblin` catalog hash (`gnoblin_catalog_hash`) does not include it. The namespace has
a hash of its own (`Catalog::namespace_fingerprint`), which its object table reports.

### Members a library adds to a class of the core

A local file of a library above the core may declare `[[augment]]` with `target = "core.<class>"`, where the target is a class of the
core catalog. Its `[[augment.property]]` and `[[augment.method]]` blocks are written as an object's members are. The std catalog
augments six classes:

- `core.gnoblin` gets `workspaces`, `displays`, `scene`, `gestures`, `request_transition`, `show` and `find_workspaces`.
- `core.output_collection` gets `profiles`.
- `core.input_context` gets `active_display`, and `core.client_collection` gets `style`.
- `core.client` gets `style`, `computed_style`, `style_warnings`, `workspace`, `move_to_workspace`, `move_with_keys`,
  `resize_with_keys`, `output`, `move_to_output`, `frame_inset`, `floating` and `floating_bounds`, and `core.output` gets `workspace`,
  `area` and `show`.

The last four are `receiver = true`.

The rules:

- The core catalog never reads the file, and no symbol of the core names std. The core's namespace fingerprint, its static catalog and
  its typed Rust API are the same bytes with and without the std files (`cargo test -p gnoblin-sdkgen` proves it).
- An augment's property is read-only unless the augment is `receiver = true`, and its method is a method of the object, not a
  function.
- `receiver = true` makes each member act on the object it is used on (`gnoblin.clients.where({ ... }).style`), not on every object of
  the class alike (`gnoblin.workspaces`). Each member of the lowered class takes that object as its first parameter, `target`. A
  property becomes a method that takes only `target`, which each generator installs as a property of the target. A receiver
  augment targets a handle, every augment of one target sets `receiver` alike, and no method of it has a parameter named `target`.
- A writable property of a receiver augment (`floating`) also becomes the method `set_<name>(target, value)`. TypeScript and Python
  install it as the property's setter, so `client.floating = true` works; Rust has `client.set_floating(true)` on the extension trait.
- `find = "Kind"` on a method of an augment adds the kind to the `find` of the class it extends. `find_workspaces` is
  `find = "Workspace"`, so `gnoblin.find("Workspace", { active: true })` works once std is loaded, and a shell that loads no std is
  refused the kind. Rust gets an implementation of the core's `Finds` for the library's class (`model.find::<Workspace>(..)`).
- `assign = true` on a handle, or on an augment of one, gives the class `set(props, options)` in TypeScript and Python:
    - It writes each property `props` names in one transition of the root's `request_transition`.
    - Without `options` the change is not animated.
    - A key that is not a writable property is refused before anything is written.
    - The namespace must add `request_transition` to `core.gnoblin`. Rust has no `set`: a Rust shell writes inside
      `request_transition`.
    - In std, `core.client` and `workspace` have it.
- The catalog refuses a target the core catalog lacks (the message names `core.<class>`), a record or an interface as a target, and a
  member whose name the target has as a property, a method or a signal, its own or inherited (the message names the member, the
  augment, the file of each and the core class). Two augments of one target may not declare one member.
- The members of every augment of one target are lowered into one service class of the augmenting namespace, `<class>_augment`
  (`gnoblin_augment`, `output_collection_augment`). The library implements it like any class, with each member answering through the
  library's own service (`crates/gnoblin-std-impl/src/augment.rs`), and every generator renders it.
- The surface lock records the augment, and not the lowered class, in the lock of the module that declares it
  (`augment | core.gnoblin | ...` and `member | core.gnoblin.workspaces | ...`). The members are the std library's ABI, in the std
  locks, and the core's locks and `abi-check` hold none of them.
- Each generator installs the members on the core's class when the library's module is loaded, and reads each through that service for
  the generation that is running, so a member never binds to a library of another generation:
    - TypeScript: importing `gnoblin-std/generated/objects` defines the members on the prototype of the core's class, and its
      declarations merge them into it (`declare module "gnoblin-core/generated/objects"`).
    - Python: importing `gnoblin.objects.std` sets them on the core's class. The stub of the core extends a stub-only class for each
      target (`GnoblinStdMembers`, `ClientStdMembers`, `InputContextStdMembers` and the others, written by `gnoblin.objects.std`), so a
      checker finds the members as in TypeScript. The lowered classes (`GnoblinAugment` and the others) stay for calls through the service.
    - Rust: an extension trait for each target, the core's typed class and the suffix `StdExt` (`ModelRootStdExt`, `ClientStdExt`,
      `OutputStdExt`, `InputContextStdExt`, `ClientCollectionStdExt`, `OutputCollectionStdExt`), is implemented for the core's typed
      class, and `gnoblin_std::prelude` brings them in.
- A shell that loads no std has none of the members: the attribute is missing in TypeScript and Python, and a call of an extension
  trait that is not imported does not compile in Rust.

A catalog may also take a schema file that is not on disk (`Catalog::load_dirs_with`). The std catalog does, for the built-in layouts:

- `layouts.toml` is written from `gnoblin_std_abi::layouts::LAYOUT_KINDS` (`crates/gnoblin-sdkgen/src/layout_schema.rs`), one class
  `<kind>_layout` for each kind, so the table stays the one source.
- A kind may have other names (`aliases` in the table). Each is a class `<alias>_layout` that extends the kind's class and adds nothing:
  `kiosk_layout` (`KioskLayout`) is the monocle and `carousel3d_layout` (`Carousel3dLayout`) the carousel.
- Its surface lock is `crates/gnoblin-std-abi/schema/lock/layouts.lock`.
- Each class extends the interface `layout`: it has the interface's methods and its own options and actions, and a call that takes a `layout`
  takes it, or an object a shell implemented.

| Target         | Writes                                                                                                 | Consumed by                                                         |
| -------------- | ------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------- |
| `objects`      | `rust-objects/{model,std}_catalog.rs`: the static `CATALOG` of each namespace and its `CATALOG_HASH`   | `build.rs` of `gnoblin-core-impl`, `-std-impl`                      |
| `object-api`   | `rust-objects/{model,std}_api.rs`: the typed Rust API of each namespace, over the protocol's client    | `build.rs` of `gnoblin-core`, `gnoblin-std`                         |
| `object-js`    | `js/<package>/generated/objects.js` for each namespace, and `js/gnoblin-core/generated/object_base.js` | `build.rs` of `gnoblin-js`                                          |
| `object-py`    | `python/gnoblin/objects/{model,std}.py`, with `__init__.py` and `_base.py`                             | `build.rs` of `gnoblin-runtime-python`                              |
| `object-types` | `js/<package>/generated/objects.d.ts` and `object_base.d.ts`; `python/gnoblin/objects/*.pyi`           | `build.rs` of `gnoblin-js` (the editor library), `cargo xtask dist` |

The typed Rust API (`gnoblin_core::objects`, `gnoblin_std::objects`; a shell gets the handle from `g.objects()` and `std.objects()`) is one
Rust type for each catalog type:

- an **enum** is a Rust enum whose discriminants are the ids the protocol numbers its values with (`ALL`, `as_str`);
- a **record** is a struct of its properties in catalog order. A field with a schema default starts at it, an `optional` field is an
  `Option` that starts as `None`, and any other field is always there: a record that has some is made with `new`, and one that has none
  has `Default`. The record of a family (see [Value classes](#value-classes)) is an enum of structs, one for each variant;
- a **handle** or a **service** is a struct that holds a reference and releases it when dropped. It has a constructor (`new`, or `get`
  for a service), a getter and a setter for each property, a method for each method, and an associated function for each `function`.
    - An object or a record reads as an `Option` unless the property is marked `always`, and so does the result of a method marked
      `optional`.
    - A listener ends when the returned guard is dropped; `keep` lets it last until the generation ends.
    - A handle is `Clone`: a clone takes a reference of its own, so the object lives until the last clone is dropped;
- an **interface** is a struct of handlers, one boxed closure per method. A method the library has a native default for is an
  `Option` of one, and `implement` makes the object the library calls them through.
    - A field has the method's name in the catalog. The `export` name a method has in Rust is for the handle, where `set_split` would
      be the setter of a `split` property.
    - A struct with no method the shell must implement has `Default`, so `Handlers { hook: Some(..), ..Default::default() }` names one hook.
    - A struct with such a method has `new`, which takes those and starts the rest at `None`;
- a **callback** type is a Rust closure wherever a property, a constructor or a method takes one.

The ids of a class are looked up by name the first time a call needs them. A shell built against an older catalog runs against a newer
library and fails only on a name the library lacks.

A member whose type names a class of another namespace is refused at build time with a
message that says so: a typed reference across two libraries needs both libraries' `Objects`, which no catalog needs yet. A shell that
makes classes of both libraries reaches the std `Objects` from the core's: `g.objects()?.std()?` or `g.std_objects()?`, both in
`gnoblin_std::prelude`.

### Check which class an object is

A property typed as a base class or an interface (`workspace.layout`) holds the class the library made, which may extend that type. A shell
that wants a member only the concrete class has asks for the class first. Each language uses its own check:

| Language   | Check                                                                                                   |
| ---------- | ------------------------------------------------------------------------------------------------------- |
| TypeScript | `layout instanceof StripLayout`                                                                         |
| Python     | `isinstance(layout, StripLayout)`                                                                       |
| Rust       | `StripLayout::downcast(&objects, &layout)?`, which is `Some(strip)` for a strip and `None` for the rest |

The check is true for the class and for every class that extends it. In Rust each handle class has one `downcast(objects, from)`:

- `objects` is the namespace's `Objects`, and `from` is any value that holds an object, such as a `Layout`.
- It returns `Ok(Some(class))` when the object is of that class or of a class that extends it, and `Ok(None)` when it is of another.
- It fails when the object is gone.
- The result holds a reference of its own, so `from` stays valid.

`as_<base>` is the other direction, from a class to a class it extends. A record, an interface and a service have no `downcast`.

### The classes in JavaScript, TypeScript and Python

`object-js`, `object-py` and `object-types` write the same surface for the script languages, from the same catalog, over the object
protocol that `gnoblin-runtime-js` (`__gnoblinObjects`) and `gnoblin-runtime-python` (the built-in module `_gnoblin_objects`) expose.

A class, a property, a method, a signal and the conversion of every value are rendered. Nothing is written by hand per class, and a
class added to a catalog is a class in all three languages with no change to a runtime.

The root of the object model, `gnoblin` of `gnoblin.model`, is reached as `gnoblin` in TypeScript (`gnoblin-core/generated/objects`), as
`ModelRoot::get` in Rust and as `Gnoblin.get()` in Python. Python has a second way, `from gnoblin import gnoblin`, the shell's own object:

- The stub writes the root's members once, as `GnoblinRoot` in `gnoblin.objects.model`. Both `Gnoblin` and the stub of the shell's
  object extend it.
- At run time the shell's object hands any member it has none of its own for to the root.
- The members are the catalog's, never a list kept by hand, so a property or a method added to the root is on both.

The names follow the language: `camelCase` members and `PascalCase` classes in TypeScript, `snake_case` members in Python. An
`export = { js, py }` attribute covers a name the mechanical conversion does not give (a Python keyword gets a trailing underscore:
`raise_`).

A failed call raises the language's own error:

- A call the library refuses is a `GnoblinError` with a `code` and a `target`.
- The target is `Class#id.member`, or `Class.member` for a function, a constructor or an object with no `id`.
- A bad argument stays a `TypeError` or a `RangeError`, with no code.

- TypeScript: every generated getter, setter, method, constructor, `on` and `destroy` is wrapped to throw `GnoblinError`. The
  class and the `ErrorCode` union are exported beside the generated classes.
- Python: the same members are decorated to raise `gnoblin.errors.GnoblinError`, with `code` and `target` set.
- Rust: a generated member returns `Res<T>`, and its `Fail` (`gnoblin_core::Fail`, also in `gnoblin_std::prelude`) carries `code()`,
  `reason()` and `target()`. The errors of the core and of the std library convert into `Fail`, so `?` works on any of their calls inside
  a handler.

A refusal words the member as the language spells it ([Member names in a refusal](shell-api/errors.md#member-names-in-a-refusal)):

- TypeScript: `Class.memberName`, from the table `spell_<class>` that `B.raise` reads.
- Python: `Class.member_name`, from the table `_spell_<class>` that `_B.named` reads.
- Rust: the schema's `member_name`, unchanged.

| Catalog type | TypeScript (`gnoblin-std/generated/objects`)                   | Python (`gnoblin.objects.std`)                       |
| ------------ | -------------------------------------------------------------- | ---------------------------------------------------- |
| enum         | a string-literal union (`"click" \| "sloppy"`)                 | an `IntEnum`; a name is accepted where one is read   |
| record       | a class, and an `Init` option bag for its constructor          | a `dataclass`                                        |
| handle       | a class: accessors, methods, `on(event, handler)`, `destroy()` | a class: properties, methods, `on(event[, handler])` |
| service      | a class of static functions (and `get()` when it has members)  | the same                                             |
| interface    | an abstract class a shell extends, optional methods optional   | a base class a shell subclasses                      |
| callback     | a function                                                     | a callable                                           |

A parameter may name a handle of the core's catalog, such as `object:gnoblin.model.client` in `gnoblin.std`. Each language takes the
core's own class for it:

- TypeScript: `Client`.
- Python: `gnoblin.objects.model.Client`.
- Rust: `&gnoblin_core::objects::Client`.

TypeScript and Python refuse any other value with a `TypeError` that names the class (`expected a Client, got string`). A result, a
property or a signal field of another catalog's class is not generated.

### Constructor options

The constructor of a handle takes the properties the class's `ctor` names, in that order, and then the options: every other writable
property of the class, in catalog order, each optional. A property of a base class that another library serves is not an option, and
neither is a read-only one. A service has none: `get` takes nothing.

| Language   | The constructor of `workspace` (`ctor = ["id"]`, options `name`, `gaps`, `layout`)                              |
| ---------- | --------------------------------------------------------------------------------------------------------------- |
| TypeScript | `new Workspace(id, options?: WorkspaceOptions)`, an interface with `name?`, `gaps?` and `layout?`               |
| Python     | `Workspace(id, *, name=None, gaps=None, layout=None)`, and the stub lists the same keywords                     |
| Rust       | `Workspace::new(objects, id)`, and `Workspace::new_with(objects, id, WorkspaceOptions)` with a `Default` struct |

A class with no `ctor` takes the options alone: `new Box({ gap: 4 })`, `Box(gap=4)` and `Box::new_with(objects, BoxOptions { .. })`.
`Box::new(objects)` makes it with none.

A field that is left out (`undefined`, `None`) is not sent, so the property keeps the library's default.

The sent options go in the same create call as the positional ones. The object server hands the constructor what it takes and sets the
rest through the setters before it answers. A value that is refused ends the object and fails the call, so a shell never holds a
half-made object.

The name of the options type is the class's name and `Options`. Where a catalog already has a type of that name (the record
`binding_options` beside the class `binding`), it is the class's name and `CreateOptions`. The generator refuses a catalog where both
are taken, and one whose constructor has a parameter called `options`.

### Value classes

A record is a value class in every language, from one model (`crates/gnoblin-sdkgen/src/values.rs`), so a name, a field, the order of
the fields and a default cannot differ between languages. `cargo test -p gnoblin-sdkgen` proves it for every record of both catalogs.

| Language   | A value class                                                                                                         |
| ---------- | --------------------------------------------------------------------------------------------------------------------- |
| TypeScript | `class Border` with the fields as properties, and `BorderInit`, the option bag its constructor takes                  |
| Python     | a `dataclass`, with keyword defaults                                                                                  |
| Rust       | a struct with `Default` (or `new` for the fields that must be given), so `Border { width: 3.0, ..Border::default() }` |

In JavaScript the constructor starts each field at its default. A value of `0`, `false` or `""` that a caller gives stays. A list default
is a new list for each value. A field that is not in the class, and a required field that is missing, are a `TypeError`. A call that takes
a record also takes a plain object of its fields.

A value class is a **detached value**. It holds what it was given and checks nothing itself.

A record you read (`style.borders[0]`) is a copy, so changing its field does not change the window. Write the record or the list back to
change it (`style.borders = borders`).

The library checks a value when it takes it. It refuses a bad one with the path of the field (`Client#42.style.borders[0].width: ...`)
and changes nothing.

A record can be the **family** of a tagged value. A family has a `tag`, an enum property that says what a value is, and a
`[[object.variant]]` for each value of that enum (the catalog refuses a value without a variant, and two variants for one):

```toml
[[object]]
name = "filter"
kind = "record"
tag = "kind"

[[object.variant]]
name = "blur_filter"
tag = "blur"

[[object.variant.property]]
name = "radius"
type = "double"
default = 12.0
field = "amount"
```

Each variant is a value class of its own, so a filter is one of five classes:

- `BlurFilter`
- `SaturateFilter`
- `BrightnessFilter`
- `ContrastFilter`
- `ShaderFilter`

The family is the type that holds any of them: a union in TypeScript and Python, and in Rust the enum `Filter`, with `From` for each
variant and a `kind()` method.

A call that takes the family takes an instance of a variant and refuses anything else with an error that names them. A family read from
the library is an instance of the variant its tag says. A tag no variant has is an error.

The `field` of a variant's property says where the family's record carries it: a field of the family (`amount`), or a field of a record
that field holds (`shader.source`).

The wire carries the family's record and nothing else, so a variant adds no class to the object table and no byte to the encoding of a
record. A variant's name is not a class of the namespace, and its surface is in the module's lock (`variant` and `variant_field` lines).

A script's reference to a library object ends when the script's last reference goes. The library's object lives on until `destroy()`,
its parent's end or the generation's end. How each language gives the reference back:

- JavaScript: each script object holds a native guard. When the engine frees it, the guard files the slot, and the slot goes back to the
  library at the end of the frame.
- Python: the handle's dealloc queues the release to the shell thread.

The release is never a script callback. The engine runs each `FinalizationRegistry` callback as a promise job, and a job left at the end
of a frame makes the next frame due at once, so a shell that reads and drops objects on every event would never rest.

One library object is one script object in each language, so a shell can compare objects and use them as keys:

- JavaScript keeps a `WeakRef` to the script object of each library object it holds, keyed by the object's namespace and handle. A second
  delivery answers that script object and releases the reference it carried; the entry of a collected object is dropped when its identity is
  handed over again, and by a sweep when the map has doubled.
- Python files each object in a `weakref.WeakValueDictionary`, keyed by namespace, generation and handle, with the same effect.
- Rust's handle types implement `PartialEq`, `Eq` and `Hash` by the object they hold, so two values of one object are equal. They are
  `Clone` too, and a clone is another value of the same object, held through a reference of its own.

An event is a signal, or `<property>Changed` (`<property>_changed` in Python) for each property, and a listener lasts until it is stopped
or the generation ends, not until it is collected.

An interface is implemented by giving the library an instance of the script's class:
the methods it defines are the ones the library calls, and the rest run natively.

A class that extends another namespace's class is
rendered flat, as in Rust, with the members of its base; a member whose type names a class of the other namespace is refused with the
same message the Rust target gives.

The JavaScript is embedded in `libgnoblin-runtime-js.so` and the Python in `libgnoblin-runtime-python.so`:

- the JavaScript modules are `gnoblin-std/generated/objects`, `gnoblin-core/generated/objects` and `gnoblin-core/generated/object_base`,
  which both import;
- the Python modules are `gnoblin.objects.std` and `gnoblin.objects.model`.

The declarations are what an editor and `gnoblinctl check --types` read. `gnoblin-js` embeds them with the library sources
(`editor_library_in`, which leaves the JavaScript out), and `cargo xtask dist install` writes them to `share/gnoblin/js` and
`share/gnoblin/types/python`.

Importing `gnoblin-std/generated/objects` opens `libgnoblin-std.so.1` and throws `gnoblin-std is not installed (looked in: ...)`
without it, as `gnoblin-std` does; `gnoblin.objects.std` raises `ImportError` the same way.

The catalog carries no
documentation strings yet, so a member's doc comment says what it is and not what it does.

A crate that serves a catalog of its own, as a test fixture, renders the same two files with `gnoblin_sdkgen::emit_catalog_for_build`
(`crates/gnoblin-object/fixtures/probe` covers one class of every kind, so the generated tables and the generated typed API are compiled
and run by its tests).

A library implements the classes of its catalog with a `gnoblin_object::server::Builder` (`crates/gnoblin-std-impl/src/object/`), and the
two-way completeness test of the library (`Library::problems`) fails when a class or a member of the catalog has no implementation, or an
implementation names a member the catalog lacks.

Class ids are positions in the catalog (ordered by name), and a member's id is its
position in its class's list, a base class's members first, so a base member has the same id in every derived class.

The surface lock of a local namespace is one file per catalog file: `crates/gnoblin-{core,std}-abi/schema/lock/<module>.lock`. A lane that
adds classes to its module edits that module's file and that module's lock, and never `abi.lock`. `--update` records an additive change.

## What `--check` proves

No file is committed, so there is no committed file to be stale. What `cargo xtask gen-sdks --check` (and `just gen-sdks-check`, part of
`just workspace-ci` and the `gen-sdks` job of CI) proves instead:

1. **Every target renders, deterministically.** The generator renders twice and compares the bytes. It reads no clock, environment
   variable or locale, and orders everything by name.
2. **The Python compiles and the JavaScript parses.** The rendered Python modules go through `python3 -m py_compile` and the JavaScript
   ones through `node --check` (each skipped, and said so, where the tool is missing). Cargo checks the Rust; `scripts/js-typecheck.sh`
   checks the TypeScript, and the declarations of the object classes, against the hand-written library.
3. **The public surface is the one on record.** `crates/gnoblin-schema/schema/lock/<namespace>.lock` lists every object and member
   of each namespace with the version that added it, one line each:

    ```text
    member | tree | window.title | property string rw | 0.1.0
    ```

    The comparison is the one `abi-check` makes for the C ABI. A recorded entry that is gone or changed fails when the lock's
    `released` line covers its `since`, with no way to bless it inside the major, and is only a stale lock before the release.

    A new entry is additive: `cargo xtask gen-sdks --update` records it. A hand edit of a lock is caught the same way. The SDKs of every
    language follow the catalogue, so a lock that holds proves they keep every released member.

4. **The C ABI is the one on record.** The same command runs what `cargo xtask abi-check` runs: `abi.lock` of both libraries, and the
   golden shells of example 12.

The unit tests of the crate (`cargo test -p gnoblin-sdkgen`) cover the same ground from inside: two renders are equal, every file
carries the banner and the hash, no two targets write one path, and a surface lock round-trips.

They also render the `probe` catalog (one class of every kind) and the two libraries' own as the script targets do, and check that the
Python parses and the JavaScript parses as a module.

When a `tsc` is found (`GNOBLIN_TSC`), they check that the declarations type-check a script that uses an enum, a record with defaults,
a closure property, a signal and an interface, and refuse one that gets them wrong.

## Adding a language

1. Add `crates/gnoblin-sdkgen/src/<language>.rs` with a type that implements `Target`: `name()` and `generate(&SdkSpec) -> Vec<File>`.
   A file's path is logical (`python/gnoblin/std.py`): it is relative to whatever directory the caller writes the files under.
2. Add one line to `targets()` in `crates/gnoblin-sdkgen/src/lib.rs`.
3. Read the specification, not source text. `SdkSpec` holds the catalog (`node_classes`, objects, members, types) and, per library
   (`core`, `std`):
    - constant groups: `ConstGroup::type_name` gives `EventKind` or `PlacementFlags`;
    - handle typedefs and callbacks;
    - records, with each field classified as a scalar, handle, text, nested record, or an unrepresentable pointer;
    - the function table, with every argument classified (`Arg::Text`, `Arg::Records`, `Arg::Out`, `Arg::Callback`, ...), its
      return, its object and its receiver.

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

A hand-written layer remains, on purpose small. It holds the spellings of each language (`len(collection)`, `clients()`, the tile
rounding of `Placement.tile`, a workspace in one call) and no behaviour of the desktop:

- Python: `gnoblin._std_sugar`, Rust in `gnoblin-runtime-python`.
- TypeScript: `gnoblin-std/native.ts`.

Two test files hold the typed layer to the native shells' rectangles, through the real loader over the fake compositor:
`crates/gnoblin-runtime-python/tests/shells.rs` and `crates/gnoblin-runtime-js/tests/shells.rs`.

- The tests named `typed_table_*` run the shells of `tests/fixtures/typed_table`: example 3 as it was before it moved to the
  generated object API.
- Example 3 itself (`examples/03-python-std`, `examples/03-typescript-std`) is written on the generated object API and is held to
  the same rectangles.
