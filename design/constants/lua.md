# Constants audit: lua group

Read-only audit. No Rust was changed or built. Paths are relative to the worktree root. Line numbers come from the detached worktree at `03c93fe67`.

Method: `grep -rnE '^\s*(pub(\([a-z]+\))? )?(const|static) [A-Z_0-9]+'` per crate, then a read of each hit and of the inline limits in non-test code. Test modules and `tests/` were excluded where the grep made that clear. Counts below are the grep hits, not a full inventory, so a few inline magic numbers are grouped rather than listed one by one.

## 1. Summary per crate

| Crate                  | Grep hits | Listed individually | Grouped | Notes                                                             |
| ---------------------- | --------- | ------------------- | ------- | ----------------------------------------------------------------- |
| gnoblin-lua            | 16        | 9                   | 7       | Layout limits, role pump, registry keys, build-time schema paths  |
| gnoblin-lua-sandbox    | 8         | 6                   | 2       | Instruction, memory and time budgets, pattern limits, Lua wrapper |
| gnoblin-config         | 13        | 8                   | 5       | Search paths, last-good file, depth limits, accent table          |
| gnoblin-config-codegen | 3         | 2                   | 1       | Hook list and namespace names                                     |
| gnoblin-schema         | 2         | 2                   | 0       | Schema directory path                                             |
| gnoblin-testkit        | 4         | 3                   | 1       | Clock start, runtime directory mode, test secret                  |

Total grep hits: 54. Schema leaves in `crates/gnoblin-config/schema/config.toml` were counted separately: 103 `[[record.field]]` entries and 90 section headers, using `grep -c` (2137 lines).

## 2. Findings

### 2.1 Sandbox budgets (gnoblin-lua-sandbox)

| Item                             | Location                                          | Value             | Q1 user wants it       | Q2 upstream                                                                                 | Q3 needed                               | Verdict                                  | Risk   | Effort |
| -------------------------------- | ------------------------------------------------- | ----------------- | ---------------------- | ------------------------------------------------------------------------------------------- | --------------------------------------- | ---------------------------------------- | ------ | ------ |
| `HOOK_INSTRUCTIONS`              | `crates/gnoblin-lua-sandbox/src/lib.rs:63`        | 1000              | No, a per-hook guard   | No                                                                                          | Yes, stops runaway hooks                | KEEP, but see note                       | low    | S      |
| `FORMAT_LIMIT`                   | `crates/gnoblin-lua-sandbox/src/lib.rs:66`        | 99                | No                     | No, Lua `%99` format width is a Lua limit [INFERENCE]                                       | Yes                                     | KEEP                                     | low    | S      |
| `CONFIG_RANDOM_SEED`             | `crates/gnoblin-lua-sandbox/src/lib.rs:216`       | `0x6f6d7073`      | No                     | No                                                                                          | Yes, deterministic `math.random`        | KEEP                                     | low    | S      |
| `Limits::default` instructions   | `crates/gnoblin-lua-sandbox/src/lib.rs:85`        | 10,000,000        | Yes, for heavy configs | No                                                                                          | Yes                                     | CONFIGURE (see 3.1)                      | medium | M      |
| `Limits::default` memory         | `crates/gnoblin-lua-sandbox/src/lib.rs:86`        | 32 MiB            | Yes                    | No                                                                                          | Yes                                     | CONFIGURE (see 3.1)                      | medium | M      |
| `Limits::default` handler ticks  | `crates/gnoblin-lua-sandbox/src/lib.rs:88`        | 16 ms deadline    | Maybe                  | No                                                                                          | Yes, a frame budget                     | CONFIGURE, read-only to users            | medium | M      |
| `MAX_CAPTURES`                   | `crates/gnoblin-lua-sandbox/src/pattern.rs:19`    | 32                | No                     | Yes, matches Lua's `LUA_MAXCAPTURES` default [INFERENCE: lua-src 551.0.2 in cargo registry] | Yes                                     | KEEP, derive from the C value by comment | low    | S      |
| `MAX_DEPTH` (pattern)            | `crates/gnoblin-lua-sandbox/src/pattern.rs:23`    | 200               | No                     | Yes, matches `MAXCCALLS` in the C core [INFERENCE]                                          | Yes                                     | KEEP                                     | low    | S      |
| `CAP_UNFINISHED`, `CAP_POSITION` | `crates/gnoblin-lua-sandbox/src/pattern.rs:26,29` | sentinels         | No                     | Yes, C-library sentinels                                                                    | Yes                                     | KEEP                                     | low    | S      |
| `WRAPPER`                        | `crates/gnoblin-lua-sandbox/src/strlib.rs:42`     | Lua source string | No                     | No                                                                                          | Yes, carries caller position for errors | KEEP                                     | low    | S      |

Note on `HOOK_INSTRUCTIONS`: the value is an instruction-count granularity for the hook, not a user setting. Keep it, but name the unit in a comment so nobody mistakes it for a time budget.

### 2.2 Lua host and layout (gnoblin-lua)

| Item                        | Location                                       | Value                                       | Q1                       | Q2                                             | Q3                                          | Verdict                                          | Risk   | Effort |
| --------------------------- | ---------------------------------------------- | ------------------------------------------- | ------------------------ | ---------------------------------------------- | ------------------------------------------- | ------------------------------------------------ | ------ | ------ |
| `MAX_LOADS`                 | `crates/gnoblin-lua/src/host.rs:48`            | 32                                          | No                       | Matches the C core's nesting limit [INFERENCE] | Yes, guards `gnoblin.load` recursion        | KEEP                                             | low    | S      |
| `hard_limit` (layout host)  | `crates/gnoblin-lua/src/layout/host.rs:50`     | 100 ms                                      | Maybe, for slow machines | No                                             | Yes, a runaway layout callback needs a stop | CONFIGURE, Lua API only (see 3.2)                | medium | M      |
| `leak_cap`                  | `crates/gnoblin-lua/src/layout/host.rs:51`     | 3                                           | No                       | No                                             | Yes, bounds leaked callbacks                | KEEP, document why 3                             | low    | S      |
| `MAX_DEPTH` (layout data)   | `crates/gnoblin-lua/src/layout/data.rs:10`     | 32                                          | No                       | No                                             | Yes, stops self-referential tables          | KEEP                                             | low    | S      |
| `MAX_ENTRIES` (layout data) | `crates/gnoblin-lua/src/layout/data.rs:14`     | 65,536                                      | No                       | No                                             | Yes, bounds table size crossing into Rust   | KEEP                                             | low    | S      |
| `MODULE_BYTES`              | `crates/gnoblin-lua/src/layout/require.rs:17`  | 256 KiB                                     | No                       | No                                             | Yes, a module size cap                      | KEEP                                             | low    | S      |
| FNV-1a offset and prime     | `crates/gnoblin-lua/src/layout/require.rs:161` | `0xcbf2_9ce4_8422_2325`, `0x0100_0000_01b3` | No                       | Yes, FNV spec constants                        | Yes                                         | KEEP, name them `FNV1A_OFFSET` and `FNV1A_PRIME` | low    | S      |
| `PUMP_TIMEOUT`              | `crates/gnoblin-lua/src/role.rs:62`            | 100 ms                                      | Maybe                    | No                                             | Yes, bounds how long one pump blocks        | CONFIGURE, Lua API only (see 3.2)                | medium | M      |
| `TEST_CRASH_VARIABLE`       | `crates/gnoblin-lua/src/role.rs:70`            | env var name                                | No                       | No                                             | Test hook only                              | KEEP, confirm not reachable in release builds    | low    | S      |
| `PUMP_HANDLERS`             | `crates/gnoblin-lua/src/role.rs:76`            | 256                                         | No                       | No                                             | Yes, per-pump batch cap                     | KEEP                                             | low    | S      |
| Registry keys               | `crates/gnoblin-lua/src/bindings.rs:55-65`     | string keys                                 | No                       | No                                             | Yes, Lua registry identity                  | KEEP                                             | low    | S      |
| `SCHEMA_DIR`                | `crates/gnoblin-lua/build.rs:37`               | path                                        | No                       | No                                             | Yes, build input                            | MOVE to a shared path helper with gnoblin-schema | low    | S      |
| `OWN_NAMESPACE`             | `crates/gnoblin-lua/build.rs:43`               | `gnoblin`                                   | No                       | No                                             | Yes                                         | KEEP                                             | low    | S      |
| `RESERVED`                  | `crates/gnoblin-lua/build.rs:49`               | name list                                   | No                       | No                                             | Yes, reserved identifiers                   | KEEP                                             | low    | S      |

### 2.4 Config (gnoblin-config and codegen)

| Item                                   | Location                                       | Value                                                | Q1                              | Q2                                           | Q3                          | Verdict                                                                 | Risk   | Effort |
| -------------------------------------- | ---------------------------------------------- | ---------------------------------------------------- | ------------------------------- | -------------------------------------------- | --------------------------- | ----------------------------------------------------------------------- | ------ | ------ |
| `MAX_DEPTH` (value)                    | `crates/gnoblin-config/src/value.rs:26`        | 64                                                   | No                              | No                                           | Yes                         | KEEP                                                                    | low    | S      |
| `MAX_SALVAGE_ROUNDS`                   | `crates/gnoblin-config/src/document.rs:289`    | 32                                                   | No                              | No                                           | Safety net for salvage loop | KEEP, document as a bound not a setting                                 | low    | S      |
| `FORMAT` (last good)                   | `crates/gnoblin-config/src/last_good.rs:42`    | 1                                                    | No                              | No                                           | Yes, file version           | KEEP                                                                    | low    | S      |
| `FILE` (last good)                     | `crates/gnoblin-config/src/last_good.rs:45`    | `last-good-config.json`                              | No                              | XDG-relative, see 2.5                        | Yes                         | KEEP name, path via XDG helper                                          | low    | S      |
| State directory                        | `crates/gnoblin-config/src/last_good.rs:48`    | `gnoblin` under `XDG_STATE_HOME` or `~/.local/state` | No                              | Yes, XDG Base Directory spec                 | Yes                         | DERIVE from XDG helper, already done                                    | low    | S      |
| Init file name                         | `crates/gnoblin-config/src/locate.rs:138`      | `init.lua`                                           | No                              | No                                           | Yes                         | KEEP                                                                    | low    | S      |
| Config directory name                  | `crates/gnoblin-config/src/locate.rs:141`      | `gnoblin`                                            | No                              | Yes, XDG                                     | Yes                         | KEEP                                                                    | low    | S      |
| `NAMED_ACCENTS`                        | `crates/gnoblin-config/src/appearance.rs:60`   | 9 colours                                            | Yes, but already an enum choice | Yes, libadwaita accent palette [see 4]       | No, derive                  | DERIVE from libadwaita `Adw.AccentColor` values, or re-check on upgrade | medium | M      |
| `CORE_NAMESPACE`                       | `crates/gnoblin-config/src/extensions.rs:35`   | `gnoblin`                                            | No                              | No                                           | Yes                         | KEEP                                                                    | low    | S      |
| `DECLARED_HOOKS`                       | `crates/gnoblin-config/src/hooks.rs:34`        | generated list                                       | No                              | Generated                                    | Yes                         | DERIVE, already generated from codegen                                  | low    | S      |
| `HOOKS`                                | `crates/gnoblin-config-codegen/src/lib.rs:393` | 13 hook names                                        | No                              | Repeated in `hooks.rs`, a test compares them | Yes                         | KEEP, single source would remove the copy                               | low    | S      |
| Codegen schema directory and namespace | `crates/gnoblin-config-codegen/src/*.rs`       | paths and names                                      | No                              | No                                           | Yes                         | MOVE to shared path helper, see 3.4                                     | low    | S      |

### 2.5 Config search paths

Config is located in this order (`crates/gnoblin-config/src/locate.rs`):

1. `GNOBLIN_CONFIG`, used only when set and not empty.
2. `$XDG_CONFIG_HOME/gnoblin`, used only when `XDG_CONFIG_HOME` is absolute.
3. `$HOME/.config/gnoblin`.

The file is `init.lua`, and an override must end in `.lua`. These are already the correct sources, so they are KEEP with DERIVE for the fallback. No user-facing leaf is needed: the environment is the right interface.

### 2.6 Schema (gnoblin-schema, gnoblin-config schema)

| Item                            | Location                                       | Value                                                                    | Q1    | Q2                                               | Q3              | Verdict                                                                               | Risk   | Effort |
| ------------------------------- | ---------------------------------------------- | ------------------------------------------------------------------------ | ----- | ------------------------------------------------ | --------------- | ------------------------------------------------------------------------------------- | ------ | ------ |
| `SCHEMA_DIR`                    | `crates/gnoblin-schema/src/proto.rs:35`        | path                                                                     | No    | No                                               | Yes             | MOVE to a shared helper, see 3.4                                                      | low    | S      |
| Effect `MAX_PASSES` (schema)    | `crates/gnoblin-config/schema/effect.toml:54`  | 32                                                                       | Maybe | Mirrors `crates/gnoblin-render/src/effect.rs:30` | Yes, duplicated | DERIVE: schema text should read the Rust constant or a generated value, not repeat 32 | medium | S      |
| Too-many-passes doc             | `crates/gnoblin-config/schema/window.toml:266` | text                                                                     | No    | Repeats the limit                                | No              | REMOVE duplicate text, reference the field                                            | low    | S      |
| Effect defaults in doc comments | `crates/gnoblin-config/schema/effect.toml`     | downscale 2, passes 3, blur kawase, shadow `#00000080`, shadow blur 12.0 | Yes   | No                                               | Yes             | CONFIGURE, already configurable; the doc text must match the default value            | low    | S      |

### 2.7 Testkit (gnoblin-testkit)

| Item                   | Location                                         | Value       | Q1  | Q2              | Q3                       | Verdict | Risk | Effort |
| ---------------------- | ------------------------------------------------ | ----------- | --- | --------------- | ------------------------ | ------- | ---- | ------ |
| `START_MILLIS`         | `crates/gnoblin-testkit/src/clock.rs:13`         | 2026-01-01  | No  | No              | Yes, deterministic clock | KEEP    | low  | S      |
| Runtime directory mode | `crates/gnoblin-testkit/src/runtime.rs:39,42,92` | `0o700`     | No  | Yes, POSIX mode | Yes                      | KEEP    | low  | S      |
| `COUNTER`              | `crates/gnoblin-testkit/src/runtime.rs:36`       | `AtomicU64` | No  | No              | Yes                      | KEEP    | low  | S      |
| `SECRET` (recorder)    | `crates/gnoblin-testkit/src/recorder.rs:147`     | test string | No  | No              | Test only                | KEEP    | low  | S      |

## 3. Top nine wins

Ranked by user value over effort.

1. **Make the Lua instruction and memory budgets configurable for a shell or the user** (`lua-sandbox` `Limits`, lib.rs:85-86). Heavy layouts currently hit a hard ceiling with no way out. Medium risk. Effort M.
2. **Expose the layout hard limit and pump timeout to Lua** (`layout/host.rs:50`, `role.rs:62`). Slow machines get a clear knob. Medium risk. Effort M.
3. **Derive the accent palette from libadwaita instead of repeating it** (`appearance.rs:60`). Keeps colours matching the toolkit on upgrade. Medium risk. Effort M.
4. **Remove the duplicated effect pass limit from the schema text** (`effect.toml:54`, `window.toml:266`). Stops docs drifting from `render/effect.rs:30`. Low risk. Effort S.
5. **Share the schema directory path between gnoblin-lua, gnoblin-schema and codegen** (`build.rs:37`, `proto.rs:35`). One source of truth. Low risk. Effort S.
6. **Share the hook list between codegen and hooks.rs** (`codegen lib.rs:393`, `hooks.rs:34`). A test already guards the copy; generating it removes the copy. Low risk. Effort S.
7. **Name the FNV constants** (`require.rs:161`). Readability only. Low risk. Effort S.
8. **Document `HOOK_INSTRUCTIONS` as an instruction count** (`lib.rs:63`). Stops misuse as a time budget. Low risk. Effort S.
9. **Document why `leak_cap` is 3** (`layout/host.rs:51`). Stops someone raising it blindly. Low risk. Effort S.

## 4. Constants to leave alone and why

- Lua pattern limits (`MAX_CAPTURES`, `MAX_DEPTH`, sentinels): they match the C library, changing them changes which patterns match.
- Registry keys (`bindings.rs:55-65`): internal identity, not configurable.
- `CONFIG_RANDOM_SEED`: a fixed seed keeps `math.random` reproducible. Making it user-facing would break determinism for no gain.
- Clock start and runtime directory mode in testkit: they exist to make tests deterministic and private.
- Config search names (`init.lua`, `gnoblin`, `last-good-config.json`): fixed by XDG convention and by users' expectations.

## 5. Needs Kieran

- Should the Lua instruction, memory and tick budgets be a shell setting (Bingux) or a core config leaf? A leaf would need a unit and a range. I recommend core config, because the sandbox is core.
- Should the libadwaita accent palette be derived at build time from a pinned libadwaita, or checked by a test on upgrade?
- Does the layout hard limit (100 ms) belong in the Lua API or the shell? I have assumed the Lua API.

## Sources

- libadwaita accent palette: https://gitlab.gnome.org/GNOME/libadwaita/-/blob/8c422b36bcc3efca6bbf63a3979bd53aededbdcd/src/stylesheet/_colors.scss
- Adw.AccentColor documentation: https://gnome.pages.gitlab.gnome.org/libadwaita/doc/1.8/enum.AccentColor.html
- Lua C limits: lua-src 551.0.2 in the local cargo registry [INFERENCE: values not re-read line by line]
