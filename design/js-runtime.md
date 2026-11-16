# Gnoblin JS/TS runtime (D36)

The JavaScript shell runs on QuickJS in the `gnoblin-js` process. The owner's API spec is
`design/js-research/owner-spec.md` and wins on API shape. This file records the engineering decisions around it.
The per-lane history of the work is in [`archive/js-runtime-changelog.md`](archive/js-runtime-changelog.md).

Inputs (all in `design/js-research/`):

- `quickjs-embedding.md`: engine, binding crate, hot reload, budgets.
- `ts-in-quickjs.md`: how `.ts` runs, source maps, types.
- `current-runtime-map.md`: what exists in Rust today and what a `gnoblin-js` crate replaces.
- `spec-review.md`: contradictions in the spec, effort ranking, first slice, acceptance tests.
- `popups-portals.md`: who draws built-in dialogs, API for them.

## Where the rest lives

- [Hot reload and generations](js-runtime-reload.md): the generation swap, failed reloads and seat claims across a
  reload.
- [Tests against a real compositor and test timing](js-runtime-testing.md): how the runtime is tested.
- [Product budgets](js-runtime-budgets.md): the wall-clock limits on a script call and their measurements.
- [Performance](js-runtime-performance.md): the bytecode cache and change gating.
- [Live verification](js-runtime-live-verification.md): pictures of the shipped shell and the defects they showed.
- [Time units and error codes](js-runtime-time-errors.md): milliseconds, easings and `ErrorCode`.

## Decisions

- **J1.** Engine is rquickjs 0.14 on quickjs-ng, in a separate supervised `gnoblin-js` process.

    Why: Same shape as the Lua shell VM today (owner socket, tree protocol). The compositor never waits on JS. rquickjs
    only binds quickjs-ng.

- **J2.** `.ts` runs directly. oxc (`=0.158.0`, pinned) strips types in the module loader, with a source map. No
  embedded type checker.

    Why: A user never runs a build step. oxc matches the workspace MSRV 1.97.0. TypeScript 7 has no embeddable compiler.

- **J3.** Editors get a generated `gnoblin.d.ts` and a `tsconfig.json` written on first run. `gnoblinctl check` checks
  the module graph and syntax itself, and runs an external `tsc` for types when one exists.

    Why: Types come from the schema, like `gnoblin.d.lua` does today.

- **J4 (revised).** The public API is the owner's spec. There is no declarative config layer: an `init.ts` runs top to
  bottom against live objects. The first design's plain-data tier was removed.

    Why: The owner never asked for a second tier. A setting with no spec object gets no invented home
    (design/archive/js-spec-native.md, 3.11).

- **J5.** Hot reload swaps a whole generation. New Runtime and Context per generation, stage, validate, commit at one
  compositor boundary, then drop the old one. A failed reload keeps the old generation and reports file, line and column
  in `.ts` coordinates. File watching uses inotify.

    Why: QuickJS has no module eviction. A fresh Runtime is the only clean invalidation.

- **J6.** Observable fields on user subclasses use a snapshot diff at transaction boundaries. A Proxy in the base class
  constructor is the fallback if the diff is not enough.

    Why: Layouts and bindings run in the JS process, so Rust does not have to see every write. No Proxy is visible to
    users.

- **J7.** XR stays an out-of-process program on the owner socket (D34/D35). It is not a `.so` loaded into any process.

    Why: `extensions/gnoblin-xr` already works that way. Spec section 17 is replaced by the owner-socket model.

- **J8.** Rust draws the security-critical built-in dialogs: portal consent, secret and passphrase entry, polkit, lock
  password entry. JS themes them (`dialogs.style`), can intercept them (`DialogListener`), and can replace them only
  through the owner connection. The compositor owns each request and its deadline. JS holds a lease.

    Why: A crashed or reloading runtime must never leave the keyring, polkit or the lock without a UI. See
    `popups-portals.md`.

    Recorded exception . The lock password entry is the one place J8 is not followed: the
    stock lock screen (`data/js/gnoblin/lock.ts`) draws the entry and collects the password in the script engine, and
    Rust checks it (`authenticate`, PAM) and is the only thing that unlocks. The compositor's own card
    (`gnoblin-wayland/src/lock_card.rs`) is the Rust-owned entry, and it is the entry whenever the shell's screen is
    absent, not alive or not answering: the screen holds a lease (`heartbeat`, 2 s) and the card takes the password 1 s
    after it lapses, or at once when the shell is gone.

    Why the screen stays the shell's. The lock exists to stop other people at the keyboard, and the shell is the user's
    own process (same uid, their `init.ts`): whoever can read its memory can already read the keyring, the agent sockets
    and every other password typed in the session. A Rust-owned entry that the shell could only style needs a tree
    class for the entry and a second paint path inside a script-drawn card, and it protects against nothing the shell
    process's own privileges leave open. It becomes the right design the day the shell runs with fewer privileges than
    the user (another uid, a sandbox); until then the boundary is pinned by tests and written down in `docs/session-lock.md`,
    "What this protects, and what it cannot". The tests: `crates/gnoblin/tests/js_lock_failures.rs` (hang, throw, crash, slow,
    a script that spins in a frame handler or a reload), `js_lock_boundary.rs` (no window hears the password; it is in no
    node of the tree), `js_guardian.rs` (a killed and a frozen runtime process under a lock) and
    `crates/gnoblin-js/tests/write_refusals.rs` (a lost check is answered; a lock frame that throws renews no lease).

- **J9.** The Lua shell stays as the reference until the JS shell passes the same golden geometry corpus. Then delete
  `gnoblin-lua`, `gnoblin-lua-sandbox`, `data/lua/**` and the Lua docs.

    Why: Spec section 22. No half-migrated state.

## Developer checks

QuickJS runs TypeScript after stripping types; it does not embed `tsc`. `scripts/js-typecheck.sh` checks the stock
library, host module and starter with strict TypeScript settings and no DOM library, on the native TypeScript 7.0.2
compiler.

`just js-check` adds `scripts/js-lint.sh` (ESLint on tracked JavaScript and TypeScript) and a Prettier check, and
`workspace-ci` runs it in CI.

ESLint runs on TypeScript 6.0.3, not 7.0.2. typescript-eslint 8.72.2 declares the peer range `typescript >=4.8.4 <6.1.0`
and loads the TypeScript library, and the 7.0.2 package exports only its version. The pins live in
`scripts/js-lint.sh`; the `eslint` hook of `.pre-commit-config.yaml` carries the same versions, and the script refuses
to run when they differ.

Shell modules use the repository's Prettier configuration. Keep a file header, document public exports, and comment why
a non-obvious choice or necessary `any` or type assertion exists. See
[contributor commands and comment guidance](../CONTRIBUTING.md#typescript-shell-code).

## Zero-config baseline

Without any `init.ts`, the stock default runs: the embedded starter file (`data/js/starter/init.ts`), which is
the `init.ts` that `gnoblinctl config init` writes.

- It declares the four floating workspaces `1` to `4` of the Lua stock config (`data/default-config/config/10-windows.lua`),
  titlebars, the window menu, the not-responding alert and the stock keys of the Lua shell.
- `Super+1` to `Super+9` show the workspace at that position, so `Super+1` to `Super+4` act and the rest wait for more
  workspaces.
- An `init.ts` replaces the default. A file that binds no keys has none, and one that declares no workspace gets one
  floating `desktop`.
- The stock library is itself TypeScript, shipped in `/usr/share/gnoblin/js/` and embedded as the `gnoblin` module.

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
| P4 stock library         | `FloatingLayout`, `MasterStackLayout`, `GridLayout`, bindings, goldens against Lua                                                  | P3            |
| P5 reload and supervisor | watcher, generation swap, failure keep-old, restart, Rust fallback                                                                  | P1, P3        |
| P6 dialogs               | `gnoblin.dialogs`, `DialogListener`, `SecureEntry`, lease and `request.release`, Rust built-ins for secret entry and polkit         | P3            |
| P7 console               | JSON-over-socket REPL and inspector, `gnoblinctl console`                                                                           | P3            |
| P8 later, separate sizes | styles and effect stack, Display mirror and span, transition planner                                                                | P4            |
| P9 XR                    | `gnoblin.extensions.xr` and `gnoblin/extensions/xr` over the out-of-process XR program (J7): room, ring, View, bindings             | P4            |

## Questions for the owner (the agent has a default for each)

1. Initial workspace rule for a new toplevel. Default: the workspace shown by the display under the
   pointer.
2. Inherited style lists: copy on write when an element is mutated. Default: yes. Done: ["P8a style status"](archive/js-runtime-log-ui.md#p8a-style-status).
3. `where()` queries are data only, so Rust can evaluate them. Default: yes.
4. Trigger grammar. Default: the existing Lua shortcut grammar, typed as template literal types.
5. Port `prompts.lua`? Default: no. Rust draws consent. See `popups-portals.md` section 8 for the
   other eleven questions.
6. Window-rule patterns: `gnoblin-config` depends on `gnoblin-lua-pattern` today. Default: plain
   globs and regular expressions in `rules`. Done: see ["Focus, rules and snapping status"](archive/js-runtime-log-policy.md#focus-rules-and-snapping-status).
