# ABI compatibility

A shell built against Gnoblin ABI 1.0 keeps running when the core and std libraries are upgraded to a later 1.x, with no recompile. This page
is the policy: what the version numbers mean, which changes keep that promise, what an upgrade is, and how the project checks the
promise on every change.

## The version numbers

`libgnoblin-core` and `libgnoblin-std` are versioned apart, each as `<major>.<minor>.<patch>`. The file on disk carries all three; the
name a shell records carries the major only:

```text
lib/libgnoblin-core.so.1.2.0     the file (major 1, minor 2, patch 0)
lib/libgnoblin-core.so.1         the SONAME link: what a shell lists as NEEDED
lib/libgnoblin-core.so           the linker name, for a developer's build
```

| Number | Changes when                                                | A shell built earlier                                            |
| ------ | ----------------------------------------------------------- | ---------------------------------------------------------------- |
| major  | an existing call is reordered, retyped, removed or re-meant | is refused at load, with both versions named; it needs a rebuild |
| minor  | a function, constant, export or catalog member is added     | runs unchanged; it cannot call what it was not built with        |
| patch  | nothing in the interface changes                            | runs unchanged                                                   |

A shell built against a later minor than the core it finds still loads. The wrapper for a function the older core lacks returns
`Unsupported`, so a shell chooses to degrade and does not fail at start. Example 12 tests this both ways.

## Which changes are compatible

The rules, as a developer reads them:

| Change                                          | Allowed in a minor? | How it is done                                                                                                           |
| ----------------------------------------------- | ------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| a new function in the table                     | yes                 | appended to `GnoblinApi` (or `GnoblinStdApi`); `size` grows; the SDK asks `table_has` before it calls it                 |
| a new class, property, method or signal         | yes                 | the catalog grows and its hash changes; an old SDK ignores what it does not know                                         |
| a field appended to a struct passed as an array | yes                 | the stride of the array is the `size` of its first record; the core reads the fields a record reaches, the rest are zero |
| a new error code, `kind` value or flag bit      | yes                 | a caller treats an unknown code as an error and an unknown kind as unsupported                                           |
| a new exported symbol                           | rarely              | appended to `exports.txt` and `abi.lock`; a shell that names it fails to load on an older core, so prefer a table field  |
| reorder, resize, retype or remove a field       | no                  | a removed function keeps its slot as a stub that returns `E_UNSUPPORTED` until the next major                            |
| change what an existing call means              | no                  |                                                                                                                          |

A change that is not allowed in a minor is a new major: a new SONAME (`libgnoblin-core.so.2`) that installs beside the old one, so
shells built for each keep loading the library they were built for.

### The error code field

`GnoblinObjectApi` has a field `last_error_code`, appended at the end of the table within ABI 1.0. The table grew from 152 to 160 bytes.

- It copies the script API's code of the last failed call on the calling thread, as `gnoblin_last_error` copies the text.
- The code is ASCII, at most 16 bytes, NUL terminated and truncated to the buffer.
- It returns the full length, and zero when the failure has no code (a bad argument) or nothing has failed.
- The integer a call returns is unchanged.

The old-table rule is the one for every appended field:

- A client asks `table_has` before it reads the field.
- A library built before the field has a shorter table. The client never reads past its end, takes no code, and falls back to the code the integer says.
- A client built before the field never reads it, so the integer and the text are as they were.
- A code that is not one of the 13 words is ignored, and the integer decides.

### The retain field and objects of another library

`GnoblinObjectApi` has a field `retain(ctx, obj)`, appended after `last_error_code` within ABI 1.0. The table grew from 160 to 168
bytes. It adds one reference to an object, which the caller gives back with `release`, and fails with `E_STALE` when the object is gone.

A value of kind `VALUE_FOREIGN_OBJECT` (17) is an object that another library serves, such as a `gnoblin.model` client that std hands to a
shell. `bits` is the handle in that library and `ptr`/`len` is that library's namespace as UTF-8 text.

- The receiver reads the object through the table of that namespace, and asks it for the class with `class_of`.
- The reference the value carries belongs to that namespace, so the receiver releases it there.
- The library that hands the value out takes that reference with the owner's `retain`.
- A client asks `table_has` before it calls `retain`. A library built before the field has no `retain`, and another library cannot pass its
  objects on.

### The write stage field

`GnoblinApi` has a function `set_write_stage(ctx, stage, peek, user)`, appended after `object_api` within ABI 1.0. The table grew from 472 to
480 bytes. It installs, or clears, one hook on the shell thread that holds the writes an object's setter makes:

- The core offers the plain write of a window property (`client.opacity = 0.5`, `x`, `rotation`) to `stage(user, node, key, value)` before
  it sends it. When the hook returns nonzero the core sends nothing, and the hook's owner sends the write itself through the node API
  (`write_set`, `write_set_timed`), which is never offered.
- A read of such a property asks `peek(user, node, key, out)` first, so the code that wrote reads back what it wrote.
- A timed write through the object protocol (`set_timed`) is never offered. The core sends it with the seconds, delay, `tween` node and
  curve of its timing.
- `libgnoblin-std` installs the hook for as long as it stages a `requestTransition` callback, and clears it on every way out: a commit,
  an abort and a callback that failed. Clearing with `stage` null never fails and only clears the hook `user` installed.

The old-table rule is the rule for every appended field:

- The SDK asks `table_has` before it calls the function. `Gnoblin::stage_writes` in `gnoblin-core` answers `Unsupported` on a table that ends
  before the field.
- A std library on a core built before the field loads and works. A `requestTransition` callback on that core stages the writes the
  library itself makes (layout rectangles, styles, `Transition.set`) and the writes of a window's properties through the object stay plain:
  immediate, untimed and not rolled back, as they were before the field.
- A shell built before the field never calls it, so the golden shells run as they did.

## The object protocol

A class of the desktop or of the object model is not a function of a table. Both libraries serve their classes through
one table, `GnoblinObjectApi` (`gnoblin-core-abi`), that `object_api(ctx, namespace, out)` returns:

- the core serves `gnoblin.model`;
- std serves `gnoblin.std`.

The table is appended to `GnoblinApi` and `GnoblinStdApi` once. What it serves comes from a catalog, so adding a class,
a property, a method or a signal changes the catalog and never `abi.lock`:

| Change                                           | `abi.lock`    | What changes                                                                           |
| ------------------------------------------------ | ------------- | -------------------------------------------------------------------------------------- |
| a class, a member, an enum value or a callback   | unchanged     | a catalog file and its lock, `crates/gnoblin-{core,std}-abi/schema/lock/<module>.lock` |
| a function of a class                            | unchanged     | the same                                                                               |
| a new kind of value, a new function of the table | one new entry | the ABI crate, in the way of the table above, and a minor                              |

The rules a shell and a library keep, which the table's documentation states in full:

- Every function runs on the shell thread. Inputs are borrowed for the call and the library copies what it keeps. A value the library
  writes is lent until the next call into the table on that thread, and is never `VALUE_OWNED`.
- A handle (`GnoblinObj`) is a slot and a generation, so the handle of an object that is gone is `E_STALE` and never names a new object.
  A handle the library hands out, as a result, a signal's field or a callback's argument, is one reference the receiver `release`s. An
  object that nothing holds ends when the last reference goes, unless its class is retained or it has a parent.
- A class, a member and an enum value have the id the catalog gives them in this process. A base class's members come first, so a
  member of a base has the id it has in every class derived from it. A client looks a name up once.
- The library never runs a closure or an implemented method inside `create`, `update`, `set`, `call` or `destroy`. A change is told from the
  next turn of the dispatch, and the changes of one turn to a property are told once. A `listen` is for a signal or a property.
- A closure (`VALUE_CLOSURE`) or an implemented interface (`implement`) is adopted only when the call that carried it returns `E_OK`; after
  any other result the shell still owns its `user`. When the library lets go, `drop_user` runs once, while the generation's code is mapped.
- A class may extend a class another library serves (a std class whose base is a `gnoblin.model` class). The derived class's catalog lists
  the base's members first, with the ids they have there, and the derived library makes a base object in the other library when it makes
  its own, forwards those members to it (`get`, `set`, `call`, `listen`) and releases it with its own. A handle that a forwarded member
  returns belongs to the base library's table, and a client reads it through that table.
- `track(ctx, cb, user)` installs a tracker on a library for the thread, or clears it with a null `cb`. The library calls it with the
  object, class and member of every property read that succeeded, from inside the `get`, so a binding that computes a value from
  properties of objects of several libraries learns what it depends on: it installs a tracker on each library, runs the compute and
  listens to what was read. The tracker must not call back into the library that told it.
- `catalog_hash` of the table is the hash of the catalog the library was built with. An SDK built from another catalog still works: it
  looks up by name what it uses and refuses only a name the library does not have. The `gnoblin` namespace's hash (`gnoblin_catalog_hash`)
  does not include a namespace a library serves.

`gnoblin-object` is the one implementation of both ends. Two checks hold it to the ABI:

- `cargo xtask gen-sdks --check` proves the catalogs of the two libraries and their locks.
- The golden shells prove the tables did not move what they were built against. B1 appended `object_api` to `GnoblinApi` and
  `GnoblinStdApi`, and `set_write_stage` was appended to `GnoblinApi` later. Later changes appended fields to `GnoblinObjectApi` (`track` in
  B1.1, then `last_error_code` and `retain`), and no function was moved.

## What a shell sees

Both libraries answer `gnoblin_abi_version` (`gnoblin_std_abi_version` for std) with their major, minor and patch. The SDK asks for the
table with `gnoblin_get_api(major, minor_built, &table)`, and the loader compares the major the shell was built for with the core's:

- the major is the one the shell was built for: the table comes back, and its `size` says how many functions it holds;
- the major differs: the load fails with `incompatible ABI: this shell was built for gnoblin ABI 1.0; the installed core is 2.0`, and
  for std `incompatible ABI: this shell was built for gnoblin std ABI 1.0; the installed libgnoblin-std is 2.0`. The compositor logs the
  text and goes on without the shell, as it does for any shell that cannot load.

In Rust, `gnoblin::installed_abi()` and `gnoblin_std::installed_abi()` return the numbers of the libraries the shell found, and
`gnoblin::ABI_MINOR` is the minor it was built against.

## Upgrading the libraries

Replace the library files, keep the SONAME links, and restart the session. A shell binary is not touched:

1. Install the new `libgnoblin-core.so.1.<minor>.<patch>` and move the `libgnoblin-core.so.1` link to it. Do the same for
   `libgnoblin-std` if it is installed. They upgrade independently: std is a client of the core's ABI and runs on any core of that major.
2. Leave `bin/gnoblin` and the installed shells as they are. A package manager does this by replacing the library package.
3. Start the session again. A running compositor keeps the library it started with mapped (both libraries are linked `-z nodelete`,
   because a retired shell generation may still hold pointers into them), and a shell reload replaces the shell, not the libraries.

## How the promise is checked

| Check                                                  | Runs                              | What it catches                                                                                                                |
| ------------------------------------------------------ | --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `cargo xtask abi-check` (`just abi-check`)             | every CI run, `just workspace-ci` | an entry of `abi.lock` that changed or went away; a stale lock after an addition; a golden shell that is not the recorded file |
| `crates/gnoblin/tests/abi_upgrade.rs` (`just modular`) | every CI run, the `modular` job   | example 12 below, and the dependency validation                                                                                |
| `cargo xtask dist-check` (`just dist-check`)           | every CI run                      | the libraries' SONAME, exports and run paths                                                                                   |

`abi.lock` is one file per library (`crates/gnoblin-*-abi/abi.lock`). It records every export, constant, callback type, struct layout and
field offset, with the ABI version that introduced it.

- A removal or change of an entry of a released version fails the check, with no way to bless it inside the major.
- An addition is recorded with `cargo xtask abi-check --update`, in the same commit.
- Until the first release the lock says `released none`: the ABI may still change, and the lock only says what it is.

### Example 12: the golden shells

`tests/modular/golden/bin/` holds three shells built against ABI 1.0 and stored as binaries:

- `libgolden_core_shell.so`: core only.
- `libgolden_std_shell.so`: core and std.
- `libgolden_effects_shell.so`: core only. It writes four effects in one array, and was built before `GnoblinEffect`
  gained its shader fields, so its records are 152 bytes where the core's struct is 184.

Each logs the ABI it was built for and the one it finds installed, and places windows or writes its effects. The test
never rebuilds them. It starts the shipped `gnoblin` from a private prefix whose libraries are:

- the libraries of this tree (the baseline), and the shells report `installed core ABI 1.0.0`;
- libraries built with the test knob `next-minor`: ABI 1.1, patch 1, one function appended to each table. The same binaries load, run,
  place windows and report `installed core ABI 1.1.1`; `/proc/PID/maps` shows one core mapping, and one std mapping with the std shell, all
  in the prefix;
- a core built with `next-major`: it answers major 2 under the file name `.so.1`. The loader refuses both shells with the text above, the
  compositor keeps serving clients and exits normally on a signal; a std library built the same way refuses the std shell alone;
- in the other direction, a shell built for 1.1 on the 1.0 core: it loads, and the function the core lacks reports `None`.

The dependency validation runs on the golden shells and on freshly built ones: `readelf -d` names the core (and std for the std shell)
and no other library of ours, a shell carries no run path, `nm -D --defined-only` lists `gnoshell_entry_v1` alone, `ldd` resolves the libraries
into the prefix, and the process maps no copy of the core or std from anywhere else.

`cargo xtask abi-check` checks the goldens too: each file against the sha256 and size in `tests/modular/golden/golden.lock`, the
dynamic facts above, and that the ABI the lock names is one the tree has.

## For a maintainer

### Adding a function to a table

1. Append the field to `GnoblinApi` in `crates/gnoblin-core-abi/src/types.rs` (or `GnoblinStdApi` in `gnoblin-std-abi`), with its
   documentation; never between two fields.
2. Implement it in the library's `table.rs` and set the field in the static table.
3. Give the SDK a wrapper that checks the table's `size` with `table_has` and returns `Unsupported` (`None` for a getter) on a table that is too short.
4. Run `cargo xtask abi-check --update`, and commit the lock with the change. Nothing generated is committed: the SDKs are rendered by the
   build ([the generator](sdk-generator.md)), and `cargo xtask gen-sdks --check` proves they still render. Raise the
   minor in `ABI_MINOR` when the change ships in a release.
5. Add the function to the upgrade test only when it needs behaviour of its own. The test knob already appends one.

A change the table above does not allow is a new major, which is a decision for the owners and a design note first.

### Growing a struct that is passed as an array

A struct that starts with `size: u32` may gain fields at its end, and a shell built earlier keeps passing the shorter struct. When the struct is
passed one at a time that is enough. When it is passed as an array (`GnoblinEffect` in a `VALUE_EFFECTS` value), the core must not index the array
with its own `sizeof`:

1. Read the stride from the `size` of the first record, require every record to carry that `size`, and require at least the size the struct had in
   its first release.
2. Copy each record into a zeroed struct of the core's size, up to the shorter of the two, so a field the shell's record ends before is zero. Zero must
   be the empty value of every appended field.
3. Add a golden shell that passes the array, built against the SDK from before the field (`tests/modular/golden/regenerate.sh` explains how the effects
   shell is built), so the check is a binary that was never rebuilt.

### The test knobs

The ABI crates carry two features that exist only for the upgrade tests, `next-minor` and `next-major`. The libraries and the core SDK
forward them. They never go on in a build that ships, and `cargo xtask abi-check` refuses a build that has them on.

- `next-minor` makes the version 1.1 at patch 1 and appends `uptime_ms` to each table.
- `next-major` makes the major 2 and leaves the SONAME alone, so the same prefix layout carries a library of another major.

### Replacing the golden shells

Only an owner does this, and rarely: when the golden set must start over (the first ABI commit) or when the toolchain makes the stored
files unusable. A failing upgrade test means the new library broke an old shell; the answer is to fix the library or make it a new major, never
to rebuild the golden.

```sh
GNOBLIN_GOLDEN_OWNER=yes tests/modular/golden/regenerate.sh   # build, write bin/ and golden.lock
tests/modular/golden/regenerate.sh --check                    # does this toolchain build the stored files?
```

The recipe is in the header of the script:

- a workspace of its own, release profile, size optimised, symbols stripped from the file and not from the dynamic table;
- the repository's `Cargo.lock` for the dependency versions;
- path prefixes and the build id removed, so the file does not name the machine that made it.

Two builds on one toolchain give the same bytes. Another linker may not, which is why the check verifies the stored hash and does not rebuild.
After a release, a golden of the new minor is added beside these with the same recipe, and the old ones stay.
