# Gnoblin JS/TS runtime (D36)

Status: proposed by the agent on 2026-11-13, after the owner replaced the Lua shell language with
JavaScript on QuickJS. The owner's API spec is `design/js-research/owner-spec.md` and wins on API
shape. This file records the engineering decisions around it. Each decision can be overruled.

Inputs (all in `design/js-research/`):

- `quickjs-embedding.md`: engine, binding crate, hot reload, budgets.
- `ts-in-quickjs.md`: how `.ts` runs, source maps, types.
- `current-runtime-map.md`: what exists in Rust today and what a `gnoblin-js` crate replaces.
- `spec-review.md`: contradictions in the spec, effort ranking, first slice, acceptance tests.
- `popups-portals.md`: who draws built-in dialogs, API for them.

## Decisions

| ID  | Decision                                                                                                                                                                                                                                                                                                                           | Why                                                                                                                                   |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| J1  | Engine is rquickjs 0.14 on quickjs-ng, in a separate supervised `gnoblin-js` process.                                                                                                                                                                                                                                              | Same shape as the Lua shell VM today (owner socket, tree protocol). The compositor never waits on JS. rquickjs only binds quickjs-ng. |
| J2  | `.ts` runs directly. oxc (`=0.158.0`, pinned) strips types in the module loader, with a source map. No embedded type checker.                                                                                                                                                                                                      | A user never runs a build step. oxc matches the workspace MSRV 1.97.0. TypeScript 7 has no embeddable compiler.                       |
| J3  | Editors get a generated `gnoblin.d.ts` and a `tsconfig.json` written on first run. `gnoblinctl check` checks the module graph and syntax itself, and runs an external `tsc` for types when one exists.                                                                                                                             | Types come from the schema, like `gnoblin.d.lua` does today.                                                                          |
| J4  | Two tiers. Tier 1: `export default defineConfig({...})`, plain data, a closed set of key actions, diffable on reload. Tier 2: the full object API from the spec, reached from `setup(gnoblin)` or from any module.                                                                                                                 | A user who wants a key bound or a gap set does not need `new Workspace(...)`. Tier 1 desugars to tier 2.                              |
| J5  | Hot reload swaps a whole generation. New Runtime and Context per generation, stage, validate, commit at one compositor boundary, then drop the old one. A failed reload keeps the old generation and reports file, line and column in `.ts` coordinates. File watching uses inotify.                                               | QuickJS has no module eviction. A fresh Runtime is the only clean invalidation.                                                       |
| J6  | Observable fields on user subclasses use a snapshot diff at transaction boundaries. A Proxy in the base class constructor is the fallback if the diff is not enough.                                                                                                                                                               | Layouts and bindings run in the JS process, so Rust does not have to see every write. No Proxy is visible to users.                   |
| J7  | XR stays an out-of-process program on the owner socket (D34/D35). It is not a `.so` loaded into any process.                                                                                                                                                                                                                       | `extensions/gnoblin-xr` already works that way. Spec section 17 is replaced by the owner-socket model.                                |
| J8  | Rust draws the security-critical built-in dialogs: portal consent, secret and passphrase entry, polkit, lock password entry. JS themes them (`dialogs.style`), can intercept them (`DialogListener`), and can replace them only through the owner connection. The compositor owns each request and its deadline. JS holds a lease. | A crashed or reloading runtime must never leave the keyring, polkit or the lock without a UI. See `popups-portals.md`.                |
| J9  | The Lua shell stays as the reference until the JS shell passes the same golden geometry corpus. Then delete `gnoblin-lua`, `gnoblin-lua-sandbox`, `data/lua/**` and the Lua docs.                                                                                                                                                  | Spec section 22. No half-migrated state.                                                                                              |

## Zero-config baseline

Without any `init.ts`, the stock library runs: one `desktop` workspace, floating layout, one display
per output, and the stock key bindings of the Lua shell. The stock library is itself TypeScript,
shipped in `/usr/share/gnoblin/js/` and embedded as the `gnoblin` module.

## Slice 1 (first vertical slice, smallest proof)

`~/.config/gnoblin/init.ts` with a `./x.ts` import runs in the `gnoblin-js` process, talks to the
compositor over the existing tree protocol, tiles real windows with a `compute()` layout, binds
keys, and reloads on save without losing window ids.

Done criteria (headless, on Agent37):

1. `init.ts` with type annotations and an `enum`, importing `./x.ts`, evaluates. A thrown error
   reports the `.ts` line and column.
2. A `MasterStackLayout` `compute()` gives the exact rectangles of the Lua `tiling.lua` golden
   corpus.
3. Editing `init.ts` triggers a reload. Window ids and floating positions survive.
4. A syntax error or a throw in the new file keeps the previous generation running and reports the
   error.
5. An infinite loop in a handler is interrupted by the CPU budget. The compositor keeps rendering
   and the runtime restarts.
6. Killing the runtime leaves the compositor responsive. The Rust fallback places new windows and
   the supervisor restarts the runtime.
7. `gnoblinctl check init.ts` reports a syntax error with a `.ts` location.

## Work packages

| Package                  | Content                                                                                                                             | Depends on    |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------- | ------------- |
| P1 `gnoblin-js` core     | rquickjs runtime, generation, module loader and resolver, oxc strip with source map, interrupt budget, memory limit, bytecode cache | none          |
| P2 types                 | `gnoblin.d.ts` generator from the schema, `tsconfig.json` writer, `gnoblinctl check`                                                | P1 loader API |
| P3 tree binding          | JS host objects over `tree_link` and `tree_mirror`, `Collection<T>`, `Client`, `Workspace`, listener classes                        | P1            |
| P4 stock library         | `defineConfig`, `FloatingLayout`, `MasterStackLayout`, `GridLayout`, bindings, rules, goldens against Lua                           | P3            |
| P5 reload and supervisor | watcher, generation swap, failure keep-old, restart, Rust fallback                                                                  | P1, P3        |
| P6 dialogs               | `gnoblin.dialogs`, `DialogListener`, `SecureEntry`, lease and `request.release`, Rust built-ins for secret entry and polkit         | P3            |
| P7 console               | JSON-over-socket REPL and inspector, `gnoblinctl console`                                                                           | P3            |
| P8 later, separate sizes | styles and effect stack, Display mirror and span, transition planner, XR program on the new API                                     | P4            |

### P2 status

`crates/gnoblin-js-types` renders `gnoblin.d.ts` from the schema catalog (`render_dts`) and writes it with a
`tsconfig.json` into the config directory (`ensure_user_files`: the declarations are refreshed on every call, the
`tsconfig.json` only when absent). One interface per schema object; a signal `mode_changed` is
`onModeChanged(handler)`. The hand-written stock library (`defineConfig`, `Client`, `Collection<T>`, `Layout`, ...) is
checked in as `static/gnoblin-stock.d.ts` and marked proposed. `gnoblinctl check` and `gnoblinctl types init` are not
done.

## Questions for the owner (the agent has a default for each)

1. Initial workspace rule for a new toplevel. Default: the workspace shown by the display under the
   pointer.
2. Inherited style lists: copy on write when an element is mutated. Default: yes.
3. `where()` queries are data only, so Rust can evaluate them. Default: yes.
4. Trigger grammar. Default: the existing Lua shortcut grammar, typed as template literal types.
5. Port `prompts.lua`? Default: no. Rust draws consent. See `popups-portals.md` section 8 for the
   other eleven questions.
6. Window-rule patterns: `gnoblin-config` depends on `gnoblin-lua-pattern` today. Default: plain
   globs and regular expressions in `rules`.
