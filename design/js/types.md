# JavaScript shell types: the library is the types

## Why

The types of `import ... from "gnoblin-core"` and `"gnoblin-std"` used to be a second copy of the library.

- A hand-written file of about 4,200 lines (`crates/gnoblin-js-types/static/gnoblin-stock.d.ts`) declared `module "gnoblin"`
  and `module "gnoblin-std/extensions/xr"` next to the TypeScript in `data/js/gnoblin-core` and `data/js/gnoblin-std`.
- A test (`crates/gnoblin-js/tests/dts_match.rs`) policed the two for drift, but the copy still disagreed in ways it
  could not see: the XR `Camera3D` extended a simplified `Node`, not the scene `Node`.
- `gnoblin-std/scene`, `gnoblin-core/input` and `gnoblin-std/style` did not resolve.
- Go-to-definition opened the blob, and hover showed the blob's comments.

## Decision

The sources are the types.

- `gnoblinctl types init` writes the embedded library as real files next to the user's config.
- A generated tsconfig maps `gnoblin-core`, `gnoblin-std` and their submodules to them.
- The blob and the drift test are deleted, so nothing declares a library module any more.

```text
<config>/tsconfig.json       user-owned, written once: extends .gnoblin/tsconfig.json, includes ./**/*.ts
<config>/.gnoblin/           generated, rewritten on every run, ignores itself (.gitignore holds "*")
  tsconfig.json              strict ES2023, bundler resolution, paths gnoblin-core, gnoblin-std and their submodules -> lib
  gnoblin.d.ts               the globals gnoblin and console
  lib/gnoblin-core/**.ts     the sources of gnoblin-core; VERSION names the release that wrote them
  lib/gnoblin-std/**.ts      the sources of gnoblin-std
```

- Source of the files: `gnoblin_js::editor_library()`, which lists the same table `crates/gnoblin-js/build.rs` embeds for
  the shell. Packaging ships no `data/js` tree (the library lives in the binary), so there is no installed copy to point
  at, and the files cannot disagree with the binary that wrote them.
- `index.ts` is the one file that is not byte for byte the source. The shell appends `export * from "gnoblin-core/host"` to
  the stock index (`stock::host_export`), because the stock modules are tested without a host. The editor copy gets
  the same line from the same function, so `Workspace` and `gnoblin` come from `"gnoblin"` in both.
- The user's tsconfig extends the generated one. An upgrade changes options and paths without touching a user file. A
  user `files` list replaces the generated one, so it must keep `gnoblin.d.ts`; the docs say so.
- Refresh: every call rewrites a file only when its bytes differ, removes a module the library dropped, and removes
  an emptied directory. The config watcher is not recursive, and `.gnoblin` is not a module graph, so a refresh
  triggers no reload.
- Upgrade from the old layout: a root `gnoblin.d.ts` that still starts with the old generated header is deleted, and a
  root `tsconfig.json` that equals (as a value) the old generated one is replaced. Any other file is left, and `types
init` warns when a tsconfig does not extend the generated one.

## Barrels

A module sits beside the directory it fronts, as `ui.ts` does with `ui/`.

- `scene.ts`, `style.ts` and `input.ts` do the same for `scene/`, `style/` and `input/`.
- The names the owner wrote then resolve at run time and in the editor.
- `index.ts` re-exports `style` and `input` through them, so each list exists once.

## What stays generated

`gnoblin.d.ts` holds the two globals, `gnoblin` and `console`, and nothing else. They are in
`crates/gnoblin-js-types/static/globals.d.ts` under a header the crate writes. `gnoblin` is
`import("gnoblin-core/host/gnoblin").Gnoblin`, so its members are declared where they are implemented.

The file used to carry one more part: the ambient modules `gnoblin-core/schema` and its `gnoblin`, `tree` and `signals`
children, generated from the object schema. They are gone.

- No module had that name at run time, so `import { session } from "gnoblin-core/schema/gnoblin"` type-checked and then
  failed to load.
- The interfaces declared `onSignal(handler)` members that no object has: the library's listeners are classes with
  `onXxx` methods, and the named subscriptions are on the library's own classes.
- No library source imported them.
- The schema's signals still have a reader: `xtask docs-parity` takes the event names the documents use from the schema
  itself.

## Alternatives weighed

- Emit declarations with `tsc --declaration` at build time and ship maps. The emit would be a third form of the
  library, and a map into sources that are not on the user's disk opens nothing. Rejected.
- A `node_modules/gnoblin` package with an `exports` map. It works for tsc and the editor but adds a package layout the
  runtime does not have, and a new thing to keep right. Paths is what `scripts/js-typecheck.sh` already uses.
- Install the sources under `/usr/share/gnoblin/js` and point at them. No package installs them (see above), and a copy
  installed by one package could outlive the binary it matches.

## What proves it

- `crates/gnoblin-js-types` tests: the layout, byte-exact rewrite, pruning, the upgrade, and that the generated file
  declares the two globals and nothing else. The goldens are `tests/golden/gnoblin.d.ts` and the two `tsconfig` files
  beside it (`UPDATE_GOLDEN=1` rewrites them).
- `crates/gnoblin-js` `stock` tests: the editor library is the embedded sources, and `gnoblin-std/scene`, `style`, `input`
  and `ui` are modules.
- `scripts/js-types-check.sh`, run by `scripts/js-typecheck.sh`, writes the files with `types init` and puts the starter
  `init.ts` beside the programs of `tests/js-types`. Then:
    - `tsc` 7.0.2 runs over the user's own tsconfig. The `ok` programs compile, and each `bad` line fails with the error
      its `// expect:` marker names and no other line fails.
    - The compiler API of TypeScript 6.0.3, the engine of tsserver, checks that each module name resolves to its real file.
    - It checks that go-to-definition opens `scene/camera.ts` for `Camera3D`.
    - It checks that `Camera3D` extends the `Node` of `scene/node.ts`, `XRCamera3D` extends `Camera3D` and `Client`
      extends `NativeObject`.
    - It checks that hover shows the source comment.
- `xtask docs-parity` reads the library sources and the schema's signals, so the docs are checked against the real classes.

## Limits

- The user's compiler options apply to the library sources too. They pass under `strict` and `isolatedModules`; an option
  such as `noUnusedLocals` would report in library files.
- A check parses about 200 library files.
- The docs hold fragments that never compiled. A comparison of every `ts` block before and after found only the
  blocks the library types had been too loose or too narrow for, and those were fixed in the library or the page.
