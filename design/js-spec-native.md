# Spec-native shell: what removing `defineConfig` touches, what the shell already runs of the owner's examples, and how to replace it

Lane SpecNativeAudit. Status: inventory, harness and design only; **nothing is removed in this lane**.

The owner reviewed the TypeScript shell and said they never asked for `defineConfig`. It was added by the agent (proposed in
design/js-research/spec-review.md section 4, adopted as decision J4 in design/js-runtime.md). The owner chose to remove it. The
shell's user API is then exactly the owner's specification (design/js-research/owner-spec.md, the authority), the owner's
reference config (`init.js`, section 20 of the specification) and its 128 code fences. This document has three parts:

1. **Inventory**: every dependency on `defineConfig` and on the config-document path, with `file:line`, and what replaces it.
2. **Conformance**: how much of the owner's own examples the shell runs today (`crates/gnoblin-js/tests/spec_conformance.rs`,
   table in design/js-spec-conformance.md), the missing API by feature, and the effect on an earlier decision.
3. **Replacement design**: where each `defineConfig` key goes in the spec's object model, how the zero-config default, the document
   commit, reload and the tools work without a config object, and an ordered removal plan.

Everything marked **NEW** in part 3 is not in the owner's specification and is a proposal the owner can veto.

## Part 1. Inventory

Counts below come from `git grep -E "defineConfig|desugarConfig|mergeConfig|defaultConfig"` on the lane's checkout: 885 hits in 157
files, of which 297 are in 86 pages under docs/. Two hits in `infra/site/vite.config.ts` (lines 7 and 79) are Vite's own
`defineConfig` and stay; the removal check excludes that path.

The words the grep cannot find also count: the config-document path (`documentPatch`, `autostartList`, `evaluate_document`,
`__register`, `loadDefault`) is listed below even where the identifiers `defineConfig` never appear.

### 1.1 Runtime code

<!-- prettier-ignore-start -->

| Where | What it is | In the spec-native world |
| --- | --- | --- |
| `data/js/gnoblin/config.ts:771` | `defineConfig`: checks the shape (`desugarConfig`), calls `gnoblin.__register`, returns the config | deleted |
| `data/js/gnoblin/config.ts:815` | `mergeConfig(base, user)` and `mergeKeys` (779): the layering rules (lists replace, tables merge, keys merge by canonical trigger) | deleted; layering is gone, a user `init.ts` replaces the stock default (3.4) |
| `data/js/gnoblin/config.ts:914` | `desugarConfig`: config to `ConfigPlan` (workspaces, bindings, rules, listeners, document, autostart, ...) | deleted; each plan field is a direct object write (3.1) |
| `data/js/gnoblin/config.ts:740` | `defaultConfig`: four floating workspaces, stock keys, `decorations`, `menus`, `alerts` | imperative library code `gnoblin/starter` (3.4) |
| `data/js/gnoblin/config.ts:671,695,631,657` | `CONFIG_KEYS`, `SHAPES`, `DOCUMENT_SECTIONS`, `DOCUMENT_LISTS`, `RETIRED_KEYS` (663): key tables and shape checks | deleted; sections become the schema-generated tree behind `gnoblin.settings` (3.1) |
| `data/js/gnoblin/config.ts:406-487` | `GnoblinConfig` and its helper interfaces (`StyleConfig`, `ProtocolsConfig`, `GestureConfig`, `DialogsConfig`, `OutputConfig` 168, `WorkspaceSetConfig` 301, `ConfigListeners` 316) | deleted; `OutputConfig`/`OutputTransform` move to the output module as the `OutputProfile` init type |
| `data/js/gnoblin/config.ts:205` | `outputRequest(setting, path)`: the compositor request text for an output setting; also used by `Output.apply` | stays, moves to the output module (`OutputProfile` and `Output.apply` both need it) |
| `data/js/gnoblin/config.ts:542-621` | key action helpers `spawn`, `show` (`numbered`, `position`, `positions`), `focusColumn`, `binding`, `KeyAction` union (111-124) | deleted as config actions; `input.bind` takes a function (3.1). `show.numbered` becomes the library helper `bindWorkspaceKeys` |
| `data/js/gnoblin/config.ts:1103` | `planStockLauncher`, `planLauncher` (1081): `Super+space` opens the stock launcher unless a key or `launcher` takes it | a line in `gnoblin/starter`: `input.bind("Super+space", openLauncher)` |
| `data/js/gnoblin/config.ts:952,985,997` | `planWorkspaceSets`, `commandArgv`, `planAutostart` (rejects a duplicate command) | sets deleted (3.1); autostart check moves to `gnoblin.session.autostart.add` |
| `data/js/gnoblin/config.ts:1244-1292` | `planDocument`, `checkDecorations`, `checkSwitch`, `checkSetup`, `deepFreeze` | deleted; document patch comes from the settings ledger (3.5) |
| `data/js/gnoblin/index.ts:21` | `export * from "gnoblin/config"` | the surviving helpers are re-exported from their new modules |
| `crates/gnoblin-js/js/gnoblin.ts:4464-4497` | `Gnoblin.__register(config)`: merge, desugar, `checkDocument`, add workspaces, sets, `decorate()`, `startMenus()`, `startAlerts()`, `bindKeys`, `installStock`, `replaceStyle`, `configuredOutputs`, `attachOn`, `documentPatch`, `autostartList`, `setup` | deleted; the file body does these through the objects |
| `crates/gnoblin-js/js/gnoblin.ts:13,23` | import of `desugarConfig`, `defaultConfig`, `mergeConfig`, `outputRequest`, `planStockLauncher` from `gnoblin/config` | import shrinks to `outputRequest` |
| `crates/gnoblin-js/js/gnoblin.ts:3941-3974` | `installStock(plan)`: attaches the `RuleEngine` and `FocusPolicy` to `gnoblin.clients`; stores `seatOverrides` (3668) and `focusOverrides` (3932) | the focus policy is always attached by the runtime; rules become `WindowRules` (3.3); overrides become `InputContext.settings` writes |
| `crates/gnoblin-js/js/gnoblin.ts:3412-3416` | `configuredOutputs` and `configureOutput`: the `outputs` table applied to each live and each hotplugged output | `OutputProfile` objects in `gnoblin.outputs.profiles` |
| `crates/gnoblin-js/js/gnoblin.ts:3982,4155,4186` | `decorate()`, `startMenus()`, `startAlerts()`: the three stock services the flags switch on | library listeners attached by the starter (3.1) |
| `crates/gnoblin-js/js/gnoblin.ts:4108` | `attachOn`: the `on` handlers as listeners | deleted; `gnoblin.clients.addListener` |
| `crates/gnoblin-js/js/gnoblin.ts:4098,4101,4144,4644-4645` | `documentPatch`, `autostartList`, `checkDocument` (throws `GnoblinError("invalid-config", "defineConfig", ...)`), the `document()` and `autostart()` entry exports the shell reads | the ledger replaces the two variables; `document()`/`autostart()` stay as entry exports with a new source (3.5) |
| `crates/gnoblin-js/js/gnoblin.ts:4198,4500` | `effectiveConfig` and `__effectiveConfig()` | the ledger dump (3.7) |
| `crates/gnoblin-js/js/gnoblin.ts:4670` | `loadDefault: () => gnoblin.__register(defaultConfig)` | `loadDefault` evaluates the starter module (3.4) |
| `crates/gnoblin-js/js/gnoblin.ts:1215,1226` | `Workspace.layout` and `.gaps` setters call `resolveLayout` and `checkGaps` | stay; the layout-by-name string path of `resolveLayout` goes (instances only, as in the spec) |
| `crates/gnoblin-js/js/gnoblin.ts:2440-2448` | `ensureDefaultWorkspace`: a script that declares none gets one floating `desktop` | stays: the spec requires the shell to work without an `init.js` (section 18) |
| `crates/gnoblin-js/src/config.rs:1-30,198` | the configuration document from the shell's side: `ConfigState::check`/`apply`/`candidate` with `retract`, `leaves`, `overlay`, `remove`; the header names `defineConfig` | stays; only the comments change. This is the generation-overlay mechanism for the document (3.6) |
| `crates/gnoblin-js/src/document.rs:32-212` | `evaluate_document(entry)`: throwaway generation with a `PROBE` host whose `__register` merges and desugars; 1 s, 32 MiB; `DOCUMENT_BUDGET`; 9 unit tests that write `defineConfig` files | stays as an entry point, rewritten around the recording dry-run host (3.5); the tests are rewritten |
| `crates/gnoblin-js/src/role.rs:97,128,246` | `JsRoleConfig::owns_document`: the role commits the document at start | stays; comment only |
| `crates/gnoblin-js/src/runner.rs:11,276,291-334` | the runner commits at start, serves `config.reload`, and tells the guardian one `autostart` line per enabled entry then `config-accepted` (`publish`, `autostart_of`) | stays unchanged (3.5, 3.6): its input is the committed document, which is still a document |
| `crates/gnoblin-js/src/shell.rs:216,387-420,486-560,688` | `apply_settings`, `start_autostart`, `apply_document`, `add_autostart_section`, `own_document`, `load_default_config` ("declared exactly as a user's `defineConfig({})`") | stays; `load_default_config` calls `loadDefault` (new meaning), comments change |
| `crates/gnoblin-js/src/autostart.rs:1` | `JsAutostart`: starts commands once per session, only new ones on reload | stays; fed by `gnoblin.session.autostart` (3.1) |
| `crates/gnoblin-js/src/lib.rs:24-27`, `reload.rs` | crate docs; the reload stage with the last good generation kept on failure | comments only; `reload.rs` stays (spec 18, step 5) |
| `crates/gnoblin/src/main.rs:218` | the role locates and evaluates `init.ts`/`init.js` itself and commits the document | stays |

<!-- prettier-ignore-end -->

### 1.2 Library modules that read the registered config

They never import `defineConfig`; they reach the config through the plan or through a comment that tells users to switch them with it.

<!-- prettier-ignore-start -->

| Where | Dependency | In the spec-native world |
| --- | --- | --- |
| `data/js/gnoblin/rules.ts:1,6,27`; `compileRules` 271, `RuleEngine` 396 | the data-only rules engine, compiled by `planRules` (config.ts:1154) and installed by `installStock`; 540 lines | kept as `WindowRules`, a `CollectionListener` helper (3.3), or deleted: the owner's choice |
| `data/js/gnoblin/focus.ts:12,58,109` | `FocusConfig`, `FOCUS_KEYS`, `focusSettings(overrides, "config.focus", base)`; the policy reads `focusOverrides` | settings read from `InputContext.settings` (3.1); the policy class stays |
| `data/js/gnoblin/style.ts` (`checkStyleInit`), config.ts:151 | `config.style` is checked with a throwaway style; `replaceStyle(scopeStyle(clientRoot).style, plan.style)` swaps the layer | `gnoblin.clients.style` is written directly; `checkStyleInit` stays for `new ClientStyle(init)`; the layer swap is the reload mechanism (3.6) |
| `data/js/gnoblin/dialogs.ts:10,19` | the example in the header uses `defineConfig({ setup })`; `dialogs.style` is a document section | header example rewritten; `dialogs.style` becomes `gnoblin.dialogs.style` (3.1) |
| `data/js/gnoblin/workspace.ts:7,36,91,115,153` | `resolveLayout` (name or instance), `rejectUnknown`, `checkGaps`, `Workspace` options check; "the host and `defineConfig` import the helpers by name" | stays except the layout-name path; `rejectUnknown` is used by `Workspace`, `Style`, and `WindowRules` |
| `data/js/gnoblin/input.ts`, `bindings.ts`, `gestures.ts`, `gesture_actions.ts:5,249` | `parseTrigger`/`checkTrigger` (used by `planKeys`), `WorkspaceSlideBinding` (referenced by `defaultConfig`), `GestureActionConfig` and `gestureActionBinding` | trigger parsing stays; `WorkspaceSlideBinding` is renamed `WorkspaceSwitchBinding` (spec 11); `gestureActions` table becomes direct `input.bind` |
| `data/js/gnoblin/titlebar.ts:5`; `menus.ts:21,696,820`; `alerts.ts:19,383` | comments name `defineConfig({ style / menus / alerts })`; `windowMenu.start`, the alert start | the services stay; they are started by the starter and stopped by not starting them |
| `data/js/gnoblin/sets.ts`; `launcher.ts`; `idle.ts:8-10`; `authentication.ts:10-19` | `defineSets` (workspaceSets), `openLauncher`, examples in the headers that use `defineConfig` | `sets.ts` deleted with `workspaceSets`; the others stay with new header examples |
| `data/js/gnoblin/inspect.ts:429,485-488` | the inspector reports `__effectiveConfig()`; "null when the entry never called `defineConfig`" | reports the settings ledger and the generation's writes (3.7) |

<!-- prettier-ignore-end -->

### 1.3 Zero config and the starter

<!-- prettier-ignore-start -->

| Where | What | In the spec-native world |
| --- | --- | --- |
| `data/js/starter/init.ts:1-50` | the starter: `export default defineConfig({ gaps, decorations, workspaces, keys, rules, autostart })` with commented examples | rewritten in the owner's style and used as the stock default (3.4) |
| `crates/gnoblinctl/src/js.rs:601-700` (`INIT_FILE`, `write_starter`, `config_init`) | `config init [DIR] [--force]` writes the starter and the editor files | writes the new starter; same flags |
| `crates/gnoblin-js/src/shell.rs:688`; `gnoblin.ts:4670` | the shell with no `init.ts` runs `loadDefault` | runs the starter module |

<!-- prettier-ignore-end -->

### 1.4 Command-line tools and the converter

<!-- prettier-ignore-start -->

| Where | What | In the spec-native world |
| --- | --- | --- |
| `crates/gnoblinctl/src/js.rs:72-148` (`check`, `load_graph`, `STUB` 60-62) | `check FILE` loads the module graph with an inert `gnoblin` stub, then optionally runs `tsc` | runs the dry-run host (3.7); the stub stays for modules that are not entries |
| `crates/gnoblinctl/src/js.rs:153-185` (`validate_define_config`, `VALIDATE_BUDGET`), `js.rs:788-794` (a test about `Could not find export 'defineConfig'`) | loads a converted config with the stock library and a no-op `__register`, so `defineConfig` runs every check | `validate_config`: the same, over the dry-run host (3.7) |
| `crates/gnoblinctl/src/lib.rs:146-166`, `cli.rs:335-345,477-530`, `verbs.rs` | `check` routing: a module goes to `js::check`, anything else to `config validate` (the document schema) | unchanged; `config validate` validates the document file, which does not go away |
| `crates/gnoblinctl/src/migrate.rs:5,225` | `config migrate`: converts, then loads the result once "with the runtime's own `defineConfig`" | loads the result through the dry-run host |
| `crates/gnoblin-config-migrate/src/engine.rs:5,162,293,562,588,1484-1488,1561` | the converter's output target: `import { defineConfig, spawn } from "gnoblin"; export default defineConfig({ ... })`, and its checks that mirror `defineConfig` | new output target: imperative statements (3.8) |
| `crates/gnoblin-config-migrate/src/{lib.rs:13-39,names.rs:1-66,192,chord.rs:4-172,pattern.rs:3}` | the mapping tables from Lua sections to `defineConfig` keys and camelCase names, messages that say "one trigger in defineConfig" | tables retargeted to object paths; messages reworded |
| `crates/gnoblin-config-migrate/tests/convert.rs` (27 tests, 5 hits), `tests/fixtures/starter/init.ts.golden` (2) | converter tests and the starter golden | goldens regenerated |
| `crates/gnoblinctl/tests/config_migrate.rs` (15 tests, 9 hits), `js_check.rs` (26, 3), `docs_examples.rs` (3, 1) | the CLI tests, including a docs example check that loads every `defineConfig` example in docs/ | rewritten for the new target; `docs_examples` follows the docs |

<!-- prettier-ignore-end -->

### 1.5 Configuration crates and services

<!-- prettier-ignore-start -->

| Where | What | In the spec-native world |
| --- | --- | --- |
| `crates/gnoblin-config/src/locate.rs:18-209` | `locate`: `init.ts`, else `init.js`, else a warning about a Lua `init.lua` | stays |
| `crates/gnoblin-config/src/document.rs:156,165`, `tests/contract.rs:76` | refusal messages for a `window_rules`/`workspaceSets` key in the document: "set `rules` and `workspaceSets` in `defineConfig` in init.ts" | messages rewritten ("write a `WindowRules` listener in init.ts") |
| `crates/gnoblin-config/src/{apply,last_good,merge,autostart,keyring}.rs` | the document, its last-good store, `enabled_autostart` (the keyring conflict rule) | stays: the document is the wire format and the last-good record |
| `crates/gnoblin-services/src/keyring.rs:13,53,174` and tests 620-702 (17 tests, 6 hits) | first-boot keyring decision: `evaluate_document(located.path())` before the runtime role has committed anything | stays; calls the rewritten `evaluate_document` (3.5); the tests write imperative `init.ts` files |

<!-- prettier-ignore-end -->

### 1.6 Type declarations

<!-- prettier-ignore-start -->

| Where | What | In the spec-native world |
| --- | --- | --- |
| `crates/gnoblin-js-types/static/gnoblin-stock.d.ts:650,1361,1510,1782,1855,2150,2479,2499,3110` (the `Config` interface from 1782, `defineConfig` 1855, `WorkspaceSetConfig`, `GestureActionConfig`, `effectiveConfig`) | the hand-written declarations of the config surface | deleted; declarations of `gnoblin.settings`, `OutputProfile`, `Display`, ... added |
| `crates/gnoblin-js-types/tests/golden/gnoblin.d.ts` (9 hits) | the golden of the generated `gnoblin.d.ts` | regenerated |
| `crates/gnoblin-js-types/tests/config_keys.rs` (1 test), `crates/gnoblin-js/tests/dts_match.rs` (4 tests, 8 hits) | the `Config` interface names every key `defineConfig` takes and no other; the d.ts matches the stock sources | replaced by a test that the `gnoblin.settings` declarations match the document schema |

<!-- prettier-ignore-end -->

### 1.7 Tests

Hits are lines that mention the identifiers; counts are `#[test]` functions in the file.

<!-- prettier-ignore-start -->

| Group | File (tests, hits) | Disposition |
| --- | --- | --- |
| gnoblin-js: config surface | `config_slots.rs` (35, 49), `config_keys.rs` (13, 21) | each key test becomes a test of the object that now carries the key (3.1); the plan-shape and merge tests are deleted with `desugarConfig`/`mergeConfig` |
| gnoblin-js: stock library | `stock_ui.rs` (46, 36), `stock_layouts.rs` (24, 28), `stock_layouts2.rs` (29, 10), `stock_focus.rs` (52, 16), `ui.rs` (24, 7), `outputs.rs` (34, 5), `style.rs` (17, 4), `input.rs` (44, 4), `workspaces.rs` (21, 3), `window_policy.rs` (40, 3), `shell.rs` (38, 3), `tiling_bindings.rs` (14, 1), `client_requests.rs` (34, 1), `common/policy.rs` (4 hits) | `defineConfig` is the fixture that sets the world up; the same worlds are built with `new Workspace`, `input.bind`, property writes. Behaviour assertions stay |
| gnoblin-js: unit tests | `src/document.rs` (9), `src/config.rs` (10; the `defineConfig` mentions are comments) | `document.rs` rewritten; `config.rs` unchanged |
| gnoblin: integration | `crates/gnoblin/tests/js_shell.rs` (52, 40), `js_runtime_role.rs` (10, 11), `js_guardian.rs` (11, 6) | the `init.ts` text each writes is rewritten; the assertions (document revisions, guardian reports) stay |
| gnoblinctl | `config_migrate.rs` (15, 9), `js_check.rs` (26, 3), `docs_examples.rs` (3, 1), `src/js.rs` (8), `src/migrate.rs` (1) | see 1.4 |
| converter | `gnoblin-config-migrate/tests/convert.rs` (27, 5), golden `init.ts.golden` | see 1.4 |
| services, config, types | `gnoblin-services/src/keyring.rs` (17, 6), `gnoblin-config/src/document.rs` (3, 1), `gnoblin-config/tests/contract.rs` (2, 1), `gnoblin-js-types/tests/{config_keys,tree_classes}.rs` (1, 10) | see 1.5 and 1.6 |
| scripts | `scripts/js-shell-screenshots.sh:241-411` (14 hits): seven scenarios write a `defineConfig` file | the scenarios write spec-style files |

<!-- prettier-ignore-end -->

The harness of this lane adds one test file, `crates/gnoblin-js/tests/spec_conformance.rs` (1 test, 129 files judged).

### 1.8 Documentation, site and changelog

Every page that mentions the identifiers (297 mentions, 86 pages; the full per-page list is in the appendix at the end):

<!-- prettier-ignore-start -->

| Area | Pages | Disposition |
| --- | --- | --- |
| `docs/typescript-config.md` | 1 (533 lines, "defineConfig reference") | replaced by `docs/js-settings.md`: the settings tree, `session.autostart`, `InputContext.settings`, how a setting reverts (3.6). Every key section of the old page moves to the object that carries it |
| `docs/config/configure/**` | 22 pages, 76 mentions | each page shows how to set its section through `gnoblin.settings` (or the object that now carries it) |
| `docs/guides/` | 15 pages, 65 mentions | examples rewritten |
| `docs/shell-api/` | 18 pages, 57 mentions | examples rewritten |
| `docs/recipes/` | 13 pages, 31 mentions | examples rewritten; `complete-starter-config.md` shows the new starter |
| `docs/` top level | 15 pages, 58 mentions | rewritten (`typescript-shell.md`, `migrating-from-lua.md`, `user-shell.md`, `js-style.md`, `js-ui.md`, `config.md`, ...) |
| `docs/config/`, `docs/events/` | `configure.md`, `window_rule.md`, `index.md` | rewritten |
| site | `infra/site/src/lib/nav.ts:149` (the "defineConfig reference" entry, slug `typescript-config`) | renamed to the new page; `infra/site/vite.config.ts` hits are Vite's |
| `CHANGELOG.md` | 14 lines (7, 8, 10, 18, 26, 28, 29, 32-35, 39, 44, 46) | history stays as written (excluded from the removal check); one new entry in the removing step |
| `design/` | `js-runtime.md` (44), `js-parity.md` (10), `js-outputs.md` (3), `js-research/spec-review.md` (8), `js-research/popups-portals.md` (1) | `js-runtime.md`, `js-parity.md` and `js-outputs.md` are rewritten to the spec-native model; `spec-review.md` and `popups-portals.md` and this document move to `design/archive/` in the last step |

<!-- prettier-ignore-end -->

## Part 2. Conformance of the shell to the owner's examples

### 2.1 What the harness is

`crates/gnoblin-js/tests/spec_conformance.rs` loads the owner's reference `init.js` and each of the 128 fences of the specification
archive into a freshly booted `JsShell` over the fake compositor of `tests/common`, behind documented fixtures for fragments.
The inputs are stored verbatim as `tests/spec/**/*.txt`. `tests/spec/expected.toml` records the accepted `runs`, `fails` or
`skipped` status for every file and its SHA-256; any input-byte edit fails until the owner-approved digest is recorded.

The ordinary test compares every actual status with that baseline, so both regressions and newly fixed examples require a
deliberate baseline update in the same commit. `UPDATE_SPEC=1 cargo test -p gnoblin-js --test spec_conformance` regenerates the
human report and may upgrade only `fails` to `runs`; it refuses newly failing examples or a `runs` -> `fails` downgrade. A
downgrade requires a reasoned hand edit to `expected.toml`. The ignored `the_owners_reference_init_js_runs_unchanged` test
executes the owner's input without fixtures and checks its workspaces, displays, output profiles and key bindings. Remove its
ignore after it passes as part of the spec-native cutover.

`just spec-gate` is the CI-visible command; it includes ignored acceptance gates via
`cargo test -p gnoblin-js --test spec_conformance -- --include-ignored`. The ordinary suite covers expected statuses and fixture
honesty without that future cutover test. Current ordinary results: 129 files, 68 run, 51 fail and 10 skipped.

### 2.2 Result

129 files: **68 run, 51 fail, 10 skipped.**

<!-- prettier-ignore-start -->

| Kind | Run | Fail | Skipped |
| --- | --- | --- | --- |
| JavaScript (`init.js` and 110 fences) | 64 | 47 | 0 |
| TypeScript declarations (10 fences; a class-existence check) | 4 | 4 | 2 |
| Diagrams, manifest and C sketch (8 fences) | 0 | 0 | 8 |

<!-- prettier-ignore-end -->

The owner's own `init.js` fails at the first import (`Could not find export 'ArcLayout'`; `OutputProfile`, `Display`, `Viewport`
and `Camera2D` follow). Section 20 (fence 128) is the same file and fails the same way.

### 2.3 Missing API by feature

Rows are the files of design/js-spec-conformance.md that touch the gap; a row's first error can be another gap listed higher in the table (a fence that needs a `Display` and an `ArcLayout` shows the first one the file reaches). Effort: S is a day or less of library code and tests, M is
a week, L needs compositor or renderer work. Where the compositor engine already exists (profiles, mirror, per-output workspaces) the
class is S or M; where nothing exists below it the entry is L.

<!-- prettier-ignore-start -->

| Feature | What is missing | Rows (fence numbers) | Effort |
| --- | --- | --- | --- |
| `OutputProfile` | the class, `gnoblin.outputs.profiles` collection, `position: { rightOf, align }`, `vrr`, `transform`, `match` by `connector`/`make`/`model`/`serial`. The compositor already keeps profiles by connector and EDID identity and restores them on hotplug (design/js-outputs.md, "Profiles") | init.js, 028, 030, 031, 033, 107, 128 | M (class over `Output.apply`); `vrr` is L (a backend feature) |
| `Display` | the class, `gnoblin.displays`, `input.activeDisplay` (typed `null` today), `mode: "mirror" \| "span"`. A mirror and the span already exist in the engine (`mirror_of`, one global space); a `Display` is the object a script holds for them | 031, 033-035, 078, 093, 107-110, 123, init.js, 128 | M |
| `Viewport` | the class with `content` and `camera`; `content` assignment replaces `gnoblin.show(ws)` as the way to show a workspace on an output | 022-024, 031, 033-035, 078, 107-110, 123 | M |
| `Camera2D` | the class (`position`, `zoom`, `rotation`); needs a per-output content transform. `Camera3D` exists as the XR scene node, but not as a `Viewport` camera | 022, 059, 080, 085, 107, init.js | M |
| `Camera3D` on a flat workspace | perspective and orthographic projection of a workspace's windows, hit-testing from the same transform state (spec 16, "render from the same transform state that input hit-testing uses"); animating `position` and `fov` through `requestTransition` | 059, 080, 081, 107, 110, 111, init.js | L (renderer and input) |
| `ArcLayout` | the export. The fragment's `compute()` already runs as user code (058 runs), and `carousel-3d` exists as a stock layout name; what is missing is the stock class and honouring `transform` placements on a 2D workspace (needs the 3D camera) | 055, 059, 076, 080, 081, 111, init.js, 128 | S for the class, L with the camera |
| Scene and `View` nodes | the `Scene` export and `gnoblin.scene`; `View.input` is `boolean`, the spec says `"forward" \| "capture" \| "none"` (026); a `View` of a workspace (027: "a workspace is not a tree node, so a View cannot show one yet"); `View.rotation` takes an `{x, y, z}` Euler (079); `Client.committedSize` (064) | 022, 026, 027, 064, 079 | S each; `View` of a workspace is M |
| Writable transforms | `Node.transform` / `client.set({ transform })` (the shell answers "transform is not a writable property") | 082, 124 | M |
| Property bindings without polling | `node.bind(name, fn)` and `node.properties.bind(name, fn)`: a tracked expression that re-runs when what it read changes | 083, 084, 085, 121 | M |
| Style lists | `ObservableCollection.add` on `style.borders` and friends (the array-assign form and in-place edits of `shadows[0].blur` already run) | 016 | S |
| Input contexts and devices | `new InputContext({ name })` and `gnoblin.input.contexts.add` (the constructor needs a host seat today); the device classes `Keyboard`, `Mouse`, `Touchpad`, `Touchscreen`, `Tablet` as exports with `sensitivity`, `acceleration`, `repeatRate`, `repeatDelay`, `naturalScroll`, `tapToClick`; the `DragBinding` base; `NativeObject` | 004, 037, 040, 041, 043, 120 | exports S; per-device libinput settings M; a second context M |
| Extensions registry | `gnoblin.extensions` iterable with `id` and `status` | 091 | S |
| XR extension | `SpatialLayout` triggers beyond `move` and `resize` (`rotate`, 065); `xr.ControllerDragBinding` as a base class (067, 069, 070, 071); `xr.ViewMoveBinding({ tracking: "ray" })` (068); `VirtualOutput` and `gnoblin.outputs.add` (118) | 065, 067-071, 118 | S to M |
| Not a gap in the shell | fences 066, 068 and 117 call `spatial.bindings.add(new xr.ViewMoveBinding({ id: "move" }))` while a binding with id `move` exists: either `add` replaces by id, or the fragments omit a `remove`. The shell refuses a duplicate id. An owner decision, not work | 066, 068, 117 | decide |
| Not decided by this harness | `requestTransition` with `duration` and `easing` runs (072, 074, 075, 077, 086, 112, 113, 125). `autoplay: false`, `progress`, `reverse`, `finished` (053, 081, 123) are not exercised: those rows only define a class or fail on an earlier gap | 053, 081, 123 | unknown |

<!-- prettier-ignore-end -->

Summed by group, the bulk is `Display`/`Viewport`/`Camera` (twenty rows, one init.js that cannot load) and `OutputProfile`. Everything
else is small. `Camera3D` on a flat workspace is the one item with real renderer work.

### 2.4 Must "Display, Viewport and Camera are not built" be reversed?

design/js-outputs.md ("Not built, on purpose") decided that the spec's `Display`, `Viewport` and `Camera` classes are not added,
because per-output desks already give each output its own workspace, `Output` carries a profile, span is the default layout, and "XR
owns cameras, a display-mode camera would be a second owner".

**Yes, the decision must be reversed.** The owner's reference config uses all of them, and uses them for the one thing every config does:

- Switching workspaces is `input.activeDisplay.viewport.content = desktop` (init.js lines 104-112, snippets 021, 032, 036, 106). There is no `gnoblin.show(workspace)` in the specification.
- Splitting, mirroring and spanning are `new Display({ outputs: [...], mode: "mirror" | "span", viewport })` (init.js 81-90; snippets 034, 035, 108, 109).
- The 2D to 3D transition is `mainDisplay.viewport.camera = camera3D` inside `requestTransition` (init.js 127-143).

The reasons given for not building them do not survive the owner's rule that the shell's API is the specification:

1. "A thin class over `gnoblin.outputs` with no behaviour of its own" is the wrong picture. `Display.viewport.content` **is** the workspace-to-output assignment that per-output desks implement today, so the class is the specification's name for an existing behaviour; `Viewport.camera` is the one place a view transform lives.
2. "XR owns cameras" is answered by the specification itself: `xr.Camera3D` extends the core `Camera3D` and `xr.Viewport` extends `Viewport` (fence 060). Core owns the classes; XR subclasses them. The shell's XR `Camera3D` (data/js/gnoblin/scene.ts) becomes that core class, and the second owner never appears.
3. Without them the reference file cannot load, and about twenty of the 51 failing rows stay failing.

What to build, in order: `OutputProfile` and `Output` properties (engine exists), `Display` and `Viewport` over the per-output desks
(`content` is the shown workspace; `mode: "mirror"` is `mirror_of`; `mode: "span"` is the default row), `Camera2D` as the identity
or zoom/pan of the content, `Camera3D` last. `design/js-outputs.md` gets a replacement section when the first step lands.

### 2.5 Outputs, displays, viewports and cameras (lane SpecOutputs)

Built over the engine that exists; the design and its proofs are in design/js-outputs.md ("Displays, viewports and
cameras"). Where the specification is ambiguous the lane chose the reading that makes the owner's examples run:

- **A display for every output.** `input.activeDisplay` (spec 9A) and `gnoblin.displays.map(...)` (fence 093) need a
  display with no `new Display` in the script, so the shell keeps a `single` display for each output no script display
  presents. `activeDisplay` is the display under the pointer, else the primary output's (as a plain `show` acts on);
  it is `null` only while no output has a desk.
- **A workspace is on one display.** Fence 107 assigns one workspace to two displays. The renderer cannot draw a
  workspace's windows twice (that is a `View` of a workspace, lane SpecScene), so `viewport.content = ws` keeps
  `gnoblin.workspaces.show`'s swap: the display that showed `ws` takes the workspace the assigned one showed, and
  `content` reads what the display shows. Shared views of one workspace (spec 9B) are not applied.
- **`span` is one layout area.** One workspace is laid out over the bounding box of the outputs' work areas (the union of
  unequal outputs has corners no output shows). The spec's authority policy (fixed bounds, primary, last active) is not
  built.
- **`displays.add` rejects, `replace` swaps.** The spec allows "replace or reject"; the lane rejects with both ids
  (acceptance DW6) and `replace(old, new)` also removes any other script display that overlaps `new`, as the spec says.
- **`mode: "single"` takes one output.** More are a `TypeError` naming `mirror` and `span`.
- **Profile identity.** A profile's `id` defaults to its `match` written out; a profile matches the first connected output
  that fits, and `serial` is the third part of the EDID identity. A relative `position` is sent after the neighbour's
  answer. `vrr` is accepted and reported as not applied.
- **`Camera2D` is a node.** `position`, `zoom` and `rotation` are the node's `x`/`y`, scale and turn about Z, so a
  `requestTransition` animates them. Nothing in the tree can apply them to the windows (design/js-outputs.md), so a
  non-identity one is kept and reported once per reload.
- **`Camera3D` on a viewport is accepted, not drawn.** `position`, `projection`, `fov`, `near` and `far` are the camera
  node's; `size` is held by the script object (the tree has no property). The lane's brief listed `target`; the spec has
  `rotation` instead, which `Node` owns, so no `target` was added. One notice per reload:
  `not applied: camera3d (needs a projected workspace)`.
- **`Viewport.clearColor`** is kept and reported as not applied.

### 2.6 Input contexts, devices, extension registry and style lists (lane SpecInput)

Built (data/js/gnoblin/{devices,input,input_list,native_object}.ts; tests in crates/gnoblin-js/tests/input_devices.rs):

- **Device classes.** `Keyboard`, `Mouse`, `Touchpad`, `Touchscreen`, `Tablet` extend `InputDevice`, chosen from the seat's
  capability words (`tablet-*` is a `Tablet`, `touch` a `Touchscreen`, `gesture` a `Touchpad`, `keyboard` a `Keyboard`, other
  `pointer` a `Mouse`; a lid switch stays an `InputDevice`). `NativeObject` is a runtime export; `Client`, `Workspace`,
  `Output`, `InputContext` and `InputDevice` extend it. `Node`, `Layout` and `Binding` do not yet (their lanes own those files).
  `DragBinding` is a runtime export (abstract); `PointerDragBinding` now extends it, so it is no longer a `PointerBinding`.
- **Device settings** (`sensitivity`, `acceleration`, `naturalScroll`, `tapToClick`, `repeatRate`, `repeatDelay`). The only
  backing the compositor has is the document's `input` section, which is per kind, not per device: a write on one `Mouse` is
  every mouse's setting. Reading choice: `sensitivity` is libinput's -1..1 speed, `repeatRate` is repeats per second (the
  document holds the interval in ms). The keyboard group is applied; mouse and touchpad are accepted and reported "not
  applied" until a libinput applier reads them (crates/gnoblin-config/src/apply.rs). The shell commits the writes with the
  entry and again a frame after a later write (`documentRevision`). `enabled = false` is refused (`unsupported`): nothing
  backs it. Tablet and touchscreen settings are keyed by vendor/product or belong to the seat and are not exposed.
- **A second `InputContext`.** The compositor has one input seat and attaches every device to it
  (`DeviceInfo::seat`, `State::device_added`), and a tree `seat` node made by the shell would carry no devices, so a script
  context is not a second `wl_seat`. It is an object over the primary seat: it holds the devices assigned to it and claims
  each of its chords per held device (a device-scoped claim). Not native yet: separate keyboard/pointer focus per context, a
  primary-context chord not firing for another context's device, layout bindings per context, and a device keeping its
  context across unplug.
- **Extension registry.** `for (const ext of gnoblin.extensions)` yields `{ id, status }`: `xr` is `ready` while the XR
  program has a session (its `xr` seat is in the tree) and `unloaded` otherwise; `gnoblin.extensions.xr` is null then.
- **Style lists.** `style.borders/shadows/filter/backdropFilter` arrays have non-enumerable `add(entry)` and `remove(element)`
  that check and apply at once; the array-assign and in-place forms still work.

**Open owner question: `bindings.add` with an id that is already there.** Fences 066, 068 and 117 add an XR binding whose id
(`move`, `resize`) a layout already holds. The shell refuses a duplicate id (`a binding with id "move" is already in this
collection`) and keeps doing so. Either `add` replaces by id, or the fragments are missing a `remove`. The owner has not
answered; the three fences stay `fails`, pending the owner decision (replace by id or refuse).

### 2.7 The XR extension surface (lane SpecXr)

Spec section 13 and fences 060 to 071 and 114 to 118, over the out-of-process XR program (design/js-xr.md; decision J7 stays,
there is no `.so`). The harness table moves by five rows: fences 065, 067 and 069 to 071 run (066, 068 and 117 wait for the owner's
decision above), and 118 fails only on the
compositor (below). Built, with the readings the specification leaves open:

- **`xr.ControllerDragBinding`** is `XRControllerDragBinding`, the base the shell's own bindings already used; the extension now
  carries it, with `ViewRotateBinding` and `ViewScaleBinding` beside `ViewMoveBinding` and `ViewResizeBinding` (060). A script's
  class extends it with `super({ id, trigger })` as in fences 067 and 069 to 071.
- **`rotate` trigger** (065). `SpatialLayout` takes `triggers.rotate`, default `Trigger+Thumbstick+Drag`, and installs a
  `ViewRotateBinding` unless it is `null`. The layout installs no `scale` binding: the specification's `triggers` lists only
  `move`, `resize` and `rotate`, and 070 shows the scale as a user class, so `ViewScaleBinding` (`Grip+Thumbstick+Drag`) is added
  by a script.
- **`ViewScaleBinding`** adds the metres the hand moved along the direction it pointed in when the drag began to the View's
  scale, kept between `minScale` and `maxScale` (0.2 and 3). The specification's text adds an unsigned distance, which can only
  grow the panel; a signed travel lets a pull shrink it, and the limits are the specification's.
- **`ViewMoveBinding({ tracking: "ray" })`** (068) already existed. Fences 066, 068 and 117 fail on "a binding with that id is
  already in this collection" and stay `fails`, pending the owner decision (replace by id or refuse; see "Not a gap in the shell"
  above). `SpatialLayout.bindings` is a plain `BindingCollection`: a duplicate id and a second binding on one chord at one
  priority are refused, as in every other layout.
- **Pinning moved into the runtime.** A user binding such as 067 writes the View's transform and says "keep the new spatial
  pose" in `onEnd`; the ring re-placed an unpinned View a frame later. The runtime now pins the View under the ray for each drag
  and unpins it on a cancel unless it was pinned before, so no binding needs to know about the ring.
- **`ViewResizeBinding.edges`** (069, 117): `"quadrant"` (default, the corner nearest the ray) or one fixed corner.
- **`ThrowViewBinding`** (071) is not built: the specification makes it a user class with an author-written
  `runThrowSimulation`, "not a hidden magical API".
- **`xr.Camera3D extends Camera3D`** holds (core `Camera3D` is `scene.ts`'s class). **`xr.Viewport extends Viewport`** waits for
  the core `Viewport` of lane SpecOutputs; see the end of this section for what landed.
- **Fences 067, 070 and 071 define their classes and run.** Driving the bodies of 070 and 071 needs `view.rotation.clone()`,
  `view.position` and `view.scale` as the specification has them (lane SpecScene); 067's `tracking: "controller"` branch drives
  today (`the_owners_move_binding_of_snippet_067_...`).

**`VirtualOutput` and `gnoblin.outputs.add`** (118). `new VirtualOutput({ name, width, height, scale?, attachment? })` is a checked
description (`attachment` is `"viewer"` or none). The compositor cannot create an output when a script asks, so
`gnoblin.outputs.add(hud)` throws `GnoblinError` with code `unsupported` and the message "virtual outputs are not supported by
this backend", and the collection does not change; the fence fails on that and nothing else. What exists: the headless backend's
`connect_virtual_output` (`crates/gnoblin-backend/src/headless.rs`, reached only through the test-only `OutputTap`), and the
XR program's View layers. What is missing, in order, all compositor work (design/xr-extension.md, G6):

1. A tree request that creates an output for a holder of a grant: a `screen` node (or a `virtual_output` class) with `name`,
   `width`, `height`, `scale`, `attach` (`viewer`) and `on_remove`, owned by the creating connection and destroyed with it.
2. A backend-independent virtual output: an offscreen render target added to a running session (DRM, winit or headless), laid out
   outside the physical layout so the pointer does not reach it, with its own workspace desk.
3. Output protocols for it (`wl_output`, `xdg-output`, layer-shell) so a layer-shell client such as Bingux can choose it by name.
4. A way for the XR program to read its picture (output capture or a `View` of it) and to place a viewer-attached plane.
5. The removal policy (`move` or `close`) when the connection or the session ends.

Then `addVirtualOutput` in `crates/gnoblin-js/js/gnoblin.ts` calls the new host request and returns the `Output`; the class and
the collection hook stay.

## Part 3. Replacement design

### 3.0 Rules

1. The shell's user API is the owner's specification. An `init.ts` is a program that runs top to bottom against live objects (spec 3, 18, 20). There is no config object, no plan, no merge.
2. Whatever a `defineConfig` key carried is a property write or method call on a spec object. Where the specification has no home, part 3 proposes the smallest object that fits and marks it **NEW**.
3. A library helper is allowed where the specification itself shows one in user code (`FocusAppearance`, `LogOutputs`, the stock layouts written in JS). Helpers are ordinary classes composed from spec objects; none is a second API.
4. The stock default is code the user can read (3.4). Nothing the shell does for a user is invisible config.

### 3.1 Every `defineConfig` key and the object that carries it

The 37 keys of `CONFIG_KEYS` (config.ts:671-692, with the 18 sections of `DOCUMENT_SECTIONS` and the one list of `DOCUMENT_LISTS`):

<!-- prettier-ignore-start -->

| Key | Spec object that carries it | Status |
| --- | --- | --- |
| `style` | `gnoblin.clients.style.<property> = ...` (spec 6); same property names as `StyleConfig`: `cornerRadius`, `opacity`, `background`, `borders`, `shadows`, `filter`, `backdropFilter`. A per-workspace default is `workspace.clients.style`; a per-selection one is `gnoblin.clients.where({...}).style` | spec |
| `workspaces` | `new Workspace({ id, name, layout })` then `gnoblin.workspaces.add(ws)` (spec 7). A layout is an instance, `new FloatingLayout({...})`; the string names (`"floating"`) go. Reconciliation by stable `id` on reload is spec 18 | spec (exists) |
| `gaps.inner` | the layout's `gap` option (`new MasterStackLayout({ gap: 12 })`, `GridLayout({ gap: 8 })`, spec 7 and 12) | spec |
| `gaps.outer` | `Workspace.gaps` (`{ inner, outer }`), which the shell already has (`LayoutContext.gap`, d.ts:1058); the layout's `area` is the workspace rectangle less the outer gap | in the shell, **not in the spec**; keep |
| `keys` | `gnoblin.input.primary.bind(trigger, fn)` (spec 10, 11). The closed `KeyAction` union goes: `spawn(...)` is `() => gnoblin.spawn([...])`, `"close"` is `() => input.focusedClient?.close()`, `show(ws)` is `() => { input.activeDisplay.viewport.content = ws }`, `focusColumn` is a function over the layout. A drag trigger is a layout constructor option (spec 11) | spec |
| `gestureActions` | `input.bind("Swipe4Up", fn)`, `input.bind("Pinch3In", fn)` (spec 10, fence 038 uses `Swipe3Left`). The stock three-finger slide is `WorkspaceSwitchBinding` bound at `Swipe3Left` and `Swipe3Right` (spec 11, fence 053) | spec; rename `WorkspaceSlideBinding` |
| `launcher` | `input.bind("Super+space", () => gnoblin.spawn(cmd))`; the starter binds the stock `openLauncher` | spec |
| `workspaceSets` | none. The spec has no sets: "each display chooses independently from the same global workspace registry" (spec 9E). A set is a function that assigns `display.viewport.content` per display; `activateSet` goes. Library helper `showSet({ "DP-1": web, "HDMI-A-1": dev })` is optional | deleted |
| `outputs` (`mode`, `scale`, `transform`, `position`) | `new OutputProfile({ match: { connector }, mode, scale, transform, position })` and `gnoblin.outputs.profiles.add(profile)` (spec 9). `mode: "3440x1440@144"` becomes `{ width, height, refreshRate }`; `position` as `[x, y]` becomes `{ x, y }` or `{ rightOf, align }` | spec (class missing, 2.3) |
| `outputs.<c>.enabled` | `OutputProfile.enabled` | **NEW** (one property) |
| `outputs.<c>.mirrorOf` | `new Display({ outputs: [a, b], mode: "mirror", viewport })` (spec 9C) | spec |
| `decorations` | library listener `Decorations extends CollectionListener`: `onAdded(client)` puts a `Titlebar` under the window. On: `gnoblin.clients.addListener(new Decorations())`. Off: not added | library helper (spec 4 pattern) |
| `menus` | library `WindowMenu`: `new WindowMenu().start()` answers the compositor's menu requests; not started means unanswered | library helper |
| `alerts` | library `NotRespondingAlert`: `.start()`, as `menus` | library helper |
| `rules` | listeners and `where({...}).style` (spec 4, 6, 7). Decision for the owner in 3.3: delete `rules.ts`, or keep it as `WindowRules` | spec / library (3.3) |
| `focus.mode`, `newWindows`, `raiseOnClick`, `autoRaise`, `autoRaiseDelay`, `changeOnPointerRest` | `gnoblin.input.primary.settings.focus.mode = "sloppy"` and the like (a `focus` table with the six keys). `InputContext.settings` exists read-only today (`SeatSettings`, d.ts:970); it becomes writable and stays an override over the document's `windowManagement` section, as `focusOverrides` and `seatOverrides` are now (gnoblin.ts:3668, 3932). Seat-wide focus policy has no spec home, and an `InputContext` is the specification's seat | **NEW** (writable `settings`) |
| `edgeTiling`, `dragConstraint` | `gnoblin.input.primary.settings.edgeTiling = true`, `.dragConstraint = "..."`, the same object | **NEW** |
| `input`, `inputSources` (keyboard, pointer, touchpad) | per-device: `Keyboard.repeatRate`, `repeatDelay`; `Mouse.sensitivity`, `acceleration`; `Touchpad.naturalScroll`, `tapToClick` (spec 10, fence 040). A new device is handled by `gnoblin.input.devices.addListener(new class extends CollectionListener { onAdded(device) {...} }())`. Keyboard layouts (`inputSources`): `Keyboard.layouts` | spec (device classes missing, 2.3) |
| `cursor` | `gnoblin.input.primary.pointer.cursorTheme`, `.cursorSize` (`PointerState` of spec 10) | **NEW** properties on a spec object |
| `dialogs` | `gnoblin.dialogs.style.accent = "#7c3aed"`; `DialogManager` exists in the shell and is policy over the compositor's dialogs, so its look belongs there | **NEW** (one property object) |
| `autostart` | `gnoblin.session.autostart.add(["waybar"])`: a mutable `Collection<Command>`. `gnoblin.session` exists (status, verbs); the specification has no session-scoped startup list | **NEW** |
| `on` (`clientAdded`, `clientRemoved`, `titleChanged`, `focusChanged`) | `gnoblin.clients.addListener(new ClientListener / CollectionListener)` (spec 4, fences 005, 007) | spec |
| `setup(gnoblin)` | deleted: the body of the file is the setup | deleted |
| `compositor`, `windowManagement`, `appearance`, `accessibility`, `layerShell`, `keyring`, `auth`, `location`, `compat`, `capture`, `monitors`, `xwayland`, `logging` | `gnoblin.settings.<section>.<setting> = value` (`gnoblin.settings.keyring.enabled = false`). A typed tree generated from the document schema (`gnoblin-config`); a write is checked against the schema and recorded (3.5). It is the compositor-wide settings object the specification lacks | **NEW** |
| `protocols` | `gnoblin.settings.protocols.wlrLayerShell = false` | **NEW** (same tree) |
| `touchpadGestures` (a list of path gestures that start a command) | `gnoblin.settings.touchpadGestures.add({ name, gesture, fingers, path, command })`: a list in the same tree. A cardinal swipe is `input.bind`; a drawn path has no spec form | **NEW** (same tree) |

<!-- prettier-ignore-end -->

Where this puts the new surface: three objects the owner has not seen (`gnoblin.settings`, `gnoblin.session.autostart`, writable
`InputContext.settings`) and four single properties (`OutputProfile.enabled`, `PointerState.cursorTheme`/`cursorSize`,
`DialogManager.style`, `Workspace.gaps`, already in the shell). Everything else is the specification's own objects, three
library helper classes (`Decorations`, `WindowMenu`, `NotRespondingAlert`) and, if kept, `WindowRules`.

### 3.2 Writing a spec-style file: the key set of the old starter, after

```javascript
import { Workspace, FloatingLayout, Decorations, WindowMenu, NotRespondingAlert, openLauncher } from "gnoblin";

const input = gnoblin.input.primary;

gnoblin.clients.addListener(new Decorations());
new WindowMenu().start();
new NotRespondingAlert().start();

const workspaces = ["1", "2", "3", "4"].map(
    (id) => new Workspace({ id, name: `Workspace ${id}`, layout: new FloatingLayout() }),
);
for (const ws of workspaces) gnoblin.workspaces.add(ws);

// Super+1 .. Super+4 show the workspace on the display the input is on.
workspaces.forEach((ws, i) => {
    input.bind(`Super+${i + 1}`, () => {
        if (input.activeDisplay) input.activeDisplay.viewport.content = ws;
    });
});
// Super+Left and Super+Right move focus between the columns of a StripLayout, as the stock keys do (3.4).
input.bind("Super+space", openLauncher);
input.bind("XF86AudioMute", () => gnoblin.spawn(["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]));
```

This is the shape; the exact text is written when the starter lands (3.4).

### 3.3 Window rules: keep `WindowRules` or delete `rules.ts`

The data-only engine (`data/js/gnoblin/rules.ts`, 540 lines) takes `{ appId, title, focused, onWorkspace, workspace, floating, size,
position, style, fullscreen, maximised, secure, skipFocus, first, reapply }`. In the specification's model the same things are
listeners:

```javascript
gnoblin.clients.addListener(
    new (class extends CollectionListener {
        onAdded(client) {
            if (client.appId === "pavucontrol") client.floating = true; // not in the spec: see below
            if (client.appId === "firefox") development.clients.add(client); // spec 7: transfer membership
        }
    })(),
);
gnoblin.clients.where({ appId: "firefox" }).style.cornerRadius = 18; // spec 6
```

<!-- prettier-ignore-start -->

|  | Delete `rules.ts` | Keep as `WindowRules extends CollectionListener` (recommended) |
| --- | --- | --- |
| Surface | only the specification's objects | one library class, built from the specification's listeners; `gnoblin.clients.addListener(new WindowRules([...]))` |
| Behaviour that goes away | glob and RegExp patterns, "later rule wins per field", `first`, re-application when `appId`, `title` or focus changes, `skipFocus` ordering ahead of the focus policy (the rules hear a window first), key-path errors (`rules[3].appId`), the inspector's rule dump | none |
| Tests | `stock_focus.rs` (52), `window_policy.rs` (40) and the rule cases in `config_slots.rs` are rewritten as listener recipes, with real loss of coverage on matching and ordering | the same tests, with the world built by `addListener` instead of `defineConfig({ rules })` |
| Risk | the focus policy reads `rules.skipsFocus(client)` today and needs another route (a `client.skipFocus` property, **NEW**) | `installStock` already attaches the engine as a listener on `gnoblin.clients`; the change is deleting `planRules` and the config key |
| Actions that need a spec home either way | `floating`, `fullscreen`, `maximised`, `secure` are read-only or absent on `Client` in the specification (spec 4: `readonly fullscreen`); a write needs **NEW** writable properties | the same four, inside the helper |

<!-- prettier-ignore-end -->

Recommendation: keep `WindowRules` (effort S, behaviour preserved), mark it **NEW** to the owner as a library helper and not core
API, and delete it later if the owner prefers recipes. The decision is the owner's because deleting it loses matching and ordering
features, not just a key.

### 3.4 The zero-config default and the starter

The specification requires the stock shell to function without a user `init.js` (section 18, last bullet). The shell today already
treats a file with no `defineConfig` call as a program that "runs as written and gets no stock keys" (design/js-runtime.md), so the
two cases are already the same shape.

- The stock default is library code: `data/js/starter/init.ts`, written in the owner's style as in 3.2 (imports, then imperative
  statements). It is embedded as the module `gnoblin/starter`. There is one source and no `defaultConfig` object.
- With no `init.ts` or `init.js`, or when the entry fails at startup with no last-good generation, the shell evaluates
  `gnoblin/starter` as the generation's entry. `loadDefault` (gnoblin.ts:4670) becomes `import "gnoblin/starter"`.
- `gnoblinctl config init` writes that same file's text (plus the editor files). The starter is therefore always what runs with no
  config; there is no drift to test for. A test evaluates the embedded starter and the file `config init` writes and compares the
  resulting object state (workspace ids, bindings, listeners, settings ledger).
- A user's `init.ts` replaces the default. A user who wants the stock keys copies the starter or calls its exported helpers:
  `import { stockKeys } from "gnoblin/starter"; stockKeys(gnoblin.input.primary)`. This changes one behaviour: today a
  `defineConfig` file is layered over the stock keys and workspaces; after the change a file that sets only a style has no `Super+1`.
  That is the specification's model (the reference `init.js` binds its own keys) and the migration tool writes the full set (3.8).
- The runtime keeps what a locked or empty session needs and no user file may remove: the lock screen (`attachLock`), the
  `ensureDefaultWorkspace` rule (a script that declares no workspace gets one floating `desktop`) and the focus policy.

### 3.5 The document commit without a config object

The compositor takes configuration as a document (`gnoblin-config`), validates it, keeps the last good one, and the guardian reads
its `autostart` section. That does not change; what changes is where the shell gets the patch it commits.

**The settings ledger.** `gnoblin.settings` and `gnoblin.session.autostart` are host objects whose setters validate against the schema
and record `(path, value)` in the generation's ledger, last write wins. After the module graph has evaluated and its jobs have drained,
the shell asks the generation for its ledger. The entry exports `document()` and `autostart()` (gnoblin.ts:4644-4645) already return
the two strings the shell parses; they now return the ledger's patch and the autostart list. Everything past them is unchanged:
`parse_patch`, `ConfigState::apply(patch, retract)`, the commit, the revision, `autostart_of`, the guardian lines and
`config-accepted` (`runner.rs`), the recording link used by tests.

<!-- prettier-ignore-start -->

| Moment | Today | Spec-native |
| --- | --- | --- |
| Role start (`JsRoleRunner`, `role.rs:246`) | evaluate `init.ts`; `__register` fills `documentPatch`; commit | evaluate `init.ts` in the real generation; the ledger is the patch; commit |
| `config.reload` (`runner.rs`) | stage a new generation, commit its patch, answer the held caller | the same; the patch is the new generation's ledger |
| Last good | the document store keeps the last accepted revision; a failed stage keeps the old generation | unchanged |
| Guardian autostart | `autostart_of(&last.document)` after commit | unchanged; the entries come from `session.autostart` through the document's `autostart` section |
| Keyring first boot (`gnoblin-services`, `keyring.rs:174`) | `evaluate_document`: a throwaway generation whose `PROBE` host captures `__register` | `evaluate_document` runs the file in a throwaway generation over the **dry-run host** and reads the ledger |

<!-- prettier-ignore-end -->

**The dry-run host.** The role needs the effective settings before the compositor has committed anything, and a spec-style file
touches host objects at its top level (`gnoblin.workspaces.add`, `input.bind`), which `defineConfig` files did not. The proposal: a
throwaway generation (no tree, a 1 s budget, 32 MiB, as now) whose `gnoblin` global has two real parts and an inert rest.
`gnoblin.settings` and `gnoblin.session.autostart` are the real recording objects; every other member is the inert stub that
`gnoblinctl check` already uses (`js.rs:57-62`: a proxy whose get, call and construct return itself), extended with an empty
`Symbol.iterator` because the present stub reads symbols as `undefined` and a `for (const c of gnoblin.clients)` would throw.
The file runs to the end, nothing starts, nothing binds, and the ledger is read.

Trade-off: a file that decides a setting from live state (`if (gnoblin.outputs.length > 1) gnoblin.settings.x = ...`) reads the
stub's empty answers in the dry run, so the dry-run ledger can differ from the committed one. Two consequences. The dry-run result is
only a first-boot hint for one question (the keyring) and the document the runtime commits remains authoritative, as the comment in
`document.rs` already says; if the dry run throws, the caller waits for the commit as it does today. And the schema marks which
settings are decided at first boot (today only `keyring.enabled`), so the documentation can say which settings must not depend on live
state. The alternatives are worse. Putting early settings in a second declarative file is a second API the owner did not ask
for; making the services role wait for the runtime's commit reintroduces the reported bug (a keyring daemon started although the
user disabled it).

### 3.6 Reload and removal for imperative settings

A reload stages the whole new module graph in a fresh generation, validates, commits atomically and keeps the previous generation on
failure (spec 18, steps 1-6). For imperative code the problem is: a setting the old file set and the new file does not, must return
to its default, and no declarative object exists to diff. The answer is that nothing needs a diff: every settable object is a layer
with a base, and a generation owns one layer.

- **Cascaded objects** (`gnoblin.clients.style`, `workspace.clients.style`, `where(...).style`): the shell already holds the
  generation's style as a layer in the cascade and swaps it (`replaceStyle(scopeStyle(clientRoot).style, ...)`); a property the new
  generation does not write is absent from the new layer and resolves to the inherited default.
- **The document** (`gnoblin.settings`, `session.autostart`): `ConfigState::apply(patch, retract = true)` already removes the leaves
  of the previous generation's patch that the new patch does not name (`config.rs`: `leaves`, `candidate`, `remove`), so a key a reload
  dropped returns to its default. The ledger's patch is that patch.
- **Properties of long-lived host objects** (`input.primary.settings.focusMode`, `Keyboard.repeatRate`, `Output` properties, `Workspace.gaps`): each setter, on its first write in a generation, records `(object, key, base value)` in the generation's journal, where the base is the value before any generation wrote it. When a generation commits, the shell compares the new journal with the old: a key only in the old journal is set back to its recorded base; a key in both takes the new value; a key only in the new is applied. This is the same rule as `retract`, one level up.
- **Objects the generation creates** (workspaces, layouts, bindings, listeners, profiles, displays): reconciled by stable `id` (spec 18, step 3). `snapshot`/`restore` already carry window membership across generations; bindings and listeners are detached from the old generation at the commit boundary (step 4).

Failure keeps everything: staging runs the new file in the throwaway generation and then in the real one before commit; a throw, a
refused setter or a refused document leaves the old generation's layers, journal and document in place and reports the error (3.7).
The journal is generation-scoped, so a stored reference from a discarded generation cannot write into the new one (spec 18, step 6).

### 3.7 Errors, `check`, `validate`, `init`, `migrate`

**Errors.** `defineConfig` reported a list of key paths (`config.rules[3].appId must be ...`, `GnoblinError("invalid-config",
"defineConfig", problems)`). An imperative file reports the statement that failed, which is more precise: setters and constructors
throw an error that names the object path and the value, and the runtime adds the author's `file:line:column` (it already does for
every throw, in `.ts` coordinates). Examples: `init.ts:12:5: gnoblin.settings.windowManagement.focusMode must be one of click, hover,
hover-strict, got "foo"`; `init.ts:7:9: Workspace#web.layout: expected a Layout, got "grid"`. The style classes already speak this
way (`View#...input: must be a boolean`). For several problems at once, the dry-run host has a collect mode: setters record the
problem, drop the value and continue, so `check` lists every bad write with its line, and the live shell stays strict (first throw).

**`gnoblinctl check FILE`.** For a module it runs the dry-run host in collect mode (replacing `load_graph`'s bare stub and
`validate_define_config`), reports every problem with `file:line:column`, then optionally `tsc` over the new declarations. It
still stubs modules that are not entries. The Lua/document path (`config validate FILE`, a file that is not a module) is unchanged:
it validates the document against the schema, and the document remains the wire format.

**`gnoblinctl config validate`.** Unchanged. It validates a configuration document file, which is not `init.ts`. After the change
the ledger's patch can be validated with the same call (`host.config.check`), which the shell already does before a commit.

**`gnoblinctl config init`.** Writes the starter of 3.4 and the editor files. Same flags (`--force`, `DIR`), same refusal to shadow an
existing `init.js`. The inspector's effective config (`__effectiveConfig`) becomes `gnoblin.inspect.config()`: the ledger, the
journal's keys and the layers the generation holds, as plain data.

**`gnoblinctl config migrate`.** The converter's output target changes from `export default defineConfig({ ... })` to the
statements of 3.8. It validates its own output by loading it through the dry-run host in collect mode, as it loads it through
`defineConfig` today (`migrate.rs:225`). The refusals the converter makes for "something `defineConfig` would refuse" are the
setter errors of the new objects.

### 3.8 What the converter emits

For the Lua sections `config migrate` already reads: workspaces become `new Workspace(...)` and `gnoblin.workspaces.add`; shortcuts
become `input.bind` with `() => gnoblin.spawn([...])`; window rules become `new WindowRules([...])` (or listener functions if the owner
deletes `rules.ts`); outputs and monitors become `OutputProfile`s; autostart becomes `gnoblin.session.autostart.add`; every other
document section becomes `gnoblin.settings.<section>.<name> = value`. The converter writes the stock keys explicitly (3.4) when the
Lua file had them. Golden files and the 42 converter and CLI tests change with it.

### 3.9 Ordered, independently landable removal plan

Eight steps. Each leaves the tree green; a step that adds does not remove, and the last step is the only one that deletes
`defineConfig`. After step 8 `git grep -nE "defineConfig|desugarConfig|mergeConfig|defaultConfig" -- . ':!design/archive' ':!CHANGELOG.md'
':!infra/site/vite.config.ts'` is empty.

<!-- prettier-ignore-start -->

| # | Step | Files touched | Tests that must stay green | Acceptance |
| --- | --- | --- | --- | --- |
| 1 | **Add the missing objects of the specification** that the plan needs (all additive): `OutputProfile`, `Display`, `Viewport`, `Camera2D`/`Camera3D`, `gnoblin.displays`, `input.activeDisplay`, `ArcLayout`, device classes, `InputContext` constructor, `DragBinding`/`NativeObject` exports, `ObservableCollection.add` on style lists. Reverse the design/js-outputs.md decision. Each is its own landable change inside the step | `data/js/gnoblin/{scene,layouts,input,style}.ts`, `crates/gnoblin-js/js/gnoblin.ts`, output module, `gnoblin-stock.d.ts`, `design/js-outputs.md` | all of `crates/gnoblin-js/tests/*`, `crates/gnoblin/tests/js_shell.rs`, `dts_match.rs`, golden | `design/js-spec-conformance.md` regenerated; the `fails` count only goes down; `init.js` of the owner loads |
| 2 | **Add the NEW objects** the key table needs: `gnoblin.settings` (schema-generated tree), `gnoblin.session.autostart`, writable `InputContext.settings`, `PointerState.cursorTheme`/`cursorSize`, `DialogManager.style`, `OutputProfile.enabled`; library helpers `Decorations`, `WindowMenu`, `NotRespondingAlert`, `WindowRules` (the `RuleEngine` exported under its new name), `bindWorkspaceKeys`, `WorkspaceSwitchBinding` (rename) | `data/js/gnoblin/{settings,session,input,dialogs,menus,alerts,titlebar,rules}.ts`, `gnoblin.ts`, `gnoblin-config` schema export for codegen, d.ts | as step 1; new tests for each object that write a setting and check the document the shell commits | each `defineConfig` key of 3.1 has a spec-native equivalent proven by a test that sets it both ways and compares the world |
| 3 | **Ledger, journal and dry-run host**: the generation records settings writes and host-property writes; `document()` and `autostart()` read the ledger and merge it with `plan.document` (a key set by both is an error); `evaluate_document` and `gnoblinctl check` run over the dry-run host and still answer for `defineConfig` files through a compatibility `__register` in that host | `crates/gnoblin-js/{js/gnoblin.ts,src/document.rs,src/shell.rs,src/config.rs}`, `crates/gnoblinctl/src/js.rs`, `gnoblin-services/src/keyring.rs` (callers only) | `document.rs` (9), `config.rs` (10), `keyring.rs` (17), `js_check.rs` (26), `js_runtime_role.rs`, `js_guardian.rs`, `config_slots.rs` | a spec-style `init.ts` that sets `gnoblin.settings.keyring.enabled = false` is read by the first-boot path; a reload that drops a settings line reverts it (new tests for 3.5 and 3.6) |
| 4 | **Flip the zero-config default and the starter** to spec-style imperative code (`gnoblin/starter`); `loadDefault` and `config init` use it; a parity test asserts the starter's world equals what `defaultConfig` produced (workspaces, keys, listeners, the three services). `defaultConfig` stays for `defineConfig` files only | `data/js/starter/init.ts`, `crates/gnoblin-js/{js/gnoblin.ts,src/stock.rs}`, `crates/gnoblinctl/src/js.rs`, `docs/recipes/complete-starter-config.md` | `stock_ui.rs`, `stock_layouts*.rs`, `shell.rs`, `js_shell.rs` (the no-init cases), `js_check.rs` (`config init`) | with no `init.ts` the shell has the same workspaces, keys, titlebars, menu and alert as before |
| 5 | **Retarget the converter**: `config migrate` emits spec-style TypeScript; goldens and tests regenerated; output loaded through the dry-run host | `crates/gnoblin-config-migrate/src/*`, `tests/convert.rs`, `tests/fixtures/**`, `crates/gnoblinctl/src/migrate.rs`, `crates/gnoblinctl/tests/config_migrate.rs` | converter tests (27), `config_migrate.rs` (15) | the migrated starter golden equals the step 4 starter in behaviour; no converter output contains the removed names |
| 6 | **Rewrite the tests that use `defineConfig` as a fixture**, in batches that each stay green: (a) `stock_layouts*.rs`, `stock_ui.rs`; (b) `stock_focus.rs`, `window_policy.rs`, `input.rs`, `ui.rs`, `outputs.rs`, `style.rs`, `workspaces.rs`, `shell.rs`, `tiling_bindings.rs`, `client_requests.rs`, `common/policy.rs`; (c) `crates/gnoblin/tests/js_{shell,runtime_role,guardian}.rs`, `scripts/js-shell-screenshots.sh`; (d) `config_slots.rs` and `config_keys.rs` become per-object tests; `dts_match.rs` and `gnoblin-js-types/tests/config_keys.rs` check `gnoblin.settings` against the schema | the files listed | each batch runs its own files and the batch before it | `git grep -c defineConfig -- '*.rs' '*.sh'` falls to zero outside the compatibility shim |
| 7 | **Rewrite the documentation and the site**: `docs/typescript-config.md` becomes `docs/js-settings.md`; the 85 other pages' examples; `infra/site/src/lib/nav.ts`; `docs_examples.rs` loads the new examples; `docs/` pass `scripts/markdown-style.py` | `docs/**`, `infra/site/src/lib/nav.ts`, `crates/gnoblinctl/tests/docs_examples.rs` | `docs_examples.rs`, `xtask docs-parity`, the markdown hook | `git grep -l defineConfig -- docs infra` lists only `infra/site/vite.config.ts` |
| 8 | **Remove the compatibility path**: delete `defineConfig`, `mergeConfig`, `desugarConfig`, `defaultConfig`, `GnoblinConfig` and its helpers, `__register`, `__effectiveConfig`, `PROBE`, the `__register` stub in the dry-run host, `sets.ts`, `CONFIG_KEYS`; regenerate `gnoblin.d.ts` and the golden; move `outputRequest` to the output module; rewrite design/js-runtime.md and design/js-parity.md; move `design/js-research/spec-review.md`, `popups-portals.md` and this document to `design/archive/`; add the changelog entry | `data/js/gnoblin/{config,sets,index}.ts`, `crates/gnoblin-js/{js/gnoblin.ts,src/*}`, `gnoblin-stock.d.ts`, golden, `design/**`, `CHANGELOG.md` | the whole workspace gate (`cargo fmt`, `clippy -D warnings`, `cargo test -p gnoblin-js -p gnoblin-js-types`, `cargo check --workspace --locked`, xtask dupes, logging, deps, layout, docs-parity) and `spec_conformance.rs` | the grep in the opening paragraph of 3.9 is empty, `design/js-spec-conformance.md` is current, and the owner's `init.js` loads and runs |

<!-- prettier-ignore-end -->

Order matters in three places. Steps 1 and 2 come first because the replacement must exist before anything is removed. Step 3
precedes step 4 because the starter's settings (`gnoblin.settings`) must reach the document before the default can be imperative.
Step 5 can run in parallel with step 6 once step 3 lands. Steps 6 and 7 are large but mechanical and can be split by file as listed.

### 3.10 Open decisions for the owner

1. Is `gnoblin.settings` the home of compositor-wide settings (3.1, **NEW**)? The alternative is to split its 18 sections across the specification's objects, which fits only `input`, `cursor`, `monitors` and `dialogs`.
2. Keep `WindowRules` as a helper or delete the data-only rules engine (3.3)?
3. May a user file replace the stock default entirely (3.4), so a file that sets only a style has no workspace keys?
4. `add` of a binding whose id exists: replace by id, or refuse (fences 066, 068, 117)?
5. Reverse the "Display, Viewport and Camera are not built" decision (2.4)? Without it the reference `init.js` cannot load.

## Appendix: docs pages that mention the identifiers

The count is mentions of `defineConfig`, `desugarConfig`, `mergeConfig` or `defaultConfig` per page.

- `docs/` (15 pages, 58 mentions): `typescript-config.md` 13, `migrating-from-lua.md` 6, `typescript-shell.md` 6, `js-style.md` 5, `js-ui.md` 5, `user-shell.md` 5, `config.md` 4, `background-effects.md` 2, `bring-your-own-shell.md` 2, `build-a-desktop.md` 2, `js-window-policy.md` 2, `session-lock.md` 2, `wayland-protocols.md` 2, `gnoblinctl.md` 1, `js-console.md` 1.
- `docs/config/` (2 pages, 8 mentions): `configure.md` 5, `window_rule.md` 3.
- `docs/config/configure/` (18 pages, 66 mentions): `input.md` 9, `input_sources.md` 8, `window_management.md` 8, `dialogs.md` 6, `touchpad_gestures.md` 6, `appearance.md` 4, `monitors.md` 3, `protocols.md` 3, `accessibility.md` 2, `auth.md` 2, `autostart.md` 2, `compositor.md` 2, `cursor.md` 2, `keyring.md` 2, `layer_shell.md` 2, `location.md` 2, `xwayland.md` 2, `portals.md` 1.
- `docs/config/configure/input/` (4 pages, 10 mentions): `keyboard.md` 4, `mouse.md` 2, `touchpad.md` 2, `trackball-pointing-stick.md` 2.
- `docs/events/` (1 pages, 2 mentions): `index.md` 2.
- `docs/guides/` (15 pages, 65 mentions): `window_rules.md` 11, `shortcuts.md` 10, `session_settings.md` 7, `cursors.md` 6, `files_and_load_order.md` 6, `animations.md` 4, `autostart.md` 4, `replace-built-in-dialogs.md` 3, `stock-window-menu.md` 3, `theming.md` 2, `wallpapers.md` 2, `window_effects.md` 2, `window_snapping.md` 2, `window_state_shortcuts.md` 2, `editor_support.md` 1.
- `docs/recipes/` (13 pages, 31 mentions): `add-a-shortcut.md` 4, `complete-starter-config.md` 3, `organize-config-files.md` 3, `split-config-into-files.md` 3, `combine-window-rules.md` 2, `dim-unfocused-windows.md` 2, `index.md` 2, `laptop-input.md` 2, `pen-display-workflow.md` 2, `pointer-feedback.md` 2, `put-apps-on-named-workspaces.md` 2, `small-desktop.md` 2, `writing-workspace.md` 2.
- `docs/shell-api/` (18 pages, 57 mentions): `workspaces.md` 12, `input-handlers.md` 5, `focus-activation.md` 4, `keyboard-shortcuts.md` 4, `outputs.md` 4, `index.md` 3, `window-snapping.md` 3, `authentication.md` 2, `errors.md` 2, `find.md` 2, `idle.md` 2, `input.md` 2, `launch-feedback.md` 2, `processes.md` 2, `requests.md` 2, `session-runtime.md` 2, `tweens.md` 2, `windows.md` 2.
