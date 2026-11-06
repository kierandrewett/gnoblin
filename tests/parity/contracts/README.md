# Contract inventory

The parity harness has to hold three implementations to one list of names: Gnoblin on Mutter, stock
Mutter, and the Rust candidate. This directory is that list, read out of the C sources that own it.
Nothing here is hand-written: every entry records the file and line it was read from, so a
disagreement can be settled by opening the source at that line.

The inventory covers API 1.93: every wire operation of the native control socket, every event
name the compositor emits, the `gnoblinctl` command surface and the top-level keys of
`gnoblin.configure`.

## Files

| File               | Contents                                                                          |
| ------------------ | --------------------------------------------------------------------------------- |
| `operations.json`  | Every row of the wire operation table, one per socket method                      |
| `events.json`      | Every event name the compositor emits                                             |
| `cli.json`         | Every `gnoblinctl` command and action, with the help summary                      |
| `config-keys.json` | Every top-level `gnoblin.configure` key, plus the keys that can change at runtime |

Each file is a JSON object:

```json
{
    "format": 1,
    "kind": "operations",
    "generated_by": "scripts/contract-inventory.py",
    "count": 136,
    "entries": []
}
```

`format` is the schema version of the file itself. `kind` names the category, so a consumer that is
handed the wrong file fails instead of misreading it. Entries are sorted by their name, so a
regenerated file differs only where the sources differ.

## Regenerating

```console
$ python3 scripts/contract-inventory.py           # rewrite this directory
$ python3 scripts/contract-inventory.py --check   # exit 1 when a file is stale
```

The script needs Python 3.11 or later and nothing else. It does not build, run or connect to a
compositor, so it works on a machine with no session and in any checkout. `--check` compares the
checked-in files, including the counts list below, with what the sources say and exits non-zero
with the stale paths, which is what `tests/contract-inventory.test.py` runs, the same way the other
contract tests run: `python3 tests/contract-inventory.test.py`. Run the write form after changing
the C tables; a review then sees the contract change as a diff instead of reading prose.

The script reads only these sources:

| Source                                 | Read for                                                                                |
| -------------------------------------- | --------------------------------------------------------------------------------------- |
| `src/control/gnoblin-api-operations.h` | The `GNOBLIN_API_OPERATIONS(X)` rows and their `MSG_*` gate messages                    |
| `src/control/gnoblin-native-control.h` | The API major and minor                                                                 |
| `src/control/gnoblin-native-control.c` | `native_socket_events[]`, the names a socket client may subscribe to                    |
| `src/**/*.c`, `src/**/*.h`             | Event names passed to a publish or dispatch function, and the helpers that forward them |
| `src/config/lua/gnoblin-lua.c`         | `lua_api[]` and `bridge_operations[]`, which map a wire operation to a Lua entry        |
| `src/config/lua/gnoblin-lua-input.c`   | `known_configure_sections[]` and `known_protocols[]`                                    |
| `src/control/gnoblin-overrides.h`      | The sections and keys that may change at runtime                                        |
| `src/cli/gnoblinctl-cli.c`             | `commands[]` and `help_entries[]`                                                       |

A construct the script cannot read raises instead of being skipped, so a table that grows a new
shape fails the check rather than losing rows quietly.

## Entry fields

`operations.json` holds one entry per row of the operation table:

| Field                              | Meaning                                                                                                                           |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `name`, `id`, `table_index`        | The wire name, the `GNOBLIN_OP_*` id, and the row's position in the table                                                         |
| `is_family`                        | The row is a prefix family such as `window.`, not one method                                                                      |
| `gate_minor`, `gate_message`       | The first API minor that offers the operation, and the text an older client gets                                                  |
| `effective_gate_minor`             | What a client really meets: the family gate when the row declares none (`gnoblin_api_operation_gate_error`)                       |
| `cli_minor`, `effective_cli_minor` | The minor `gnoblinctl` asks for, and the value after the family fallback, which defaults to 8 (`gnoblin_api_operation_cli_minor`) |
| `flags`                            | The `GN_OP_*` flag names without their prefix                                                                                     |
| `hello`                            | The name is listed in the `hello` event's `methods`                                                                               |
| `dispatch`                         | The compositor routine the row names                                                                                              |
| `lua`                              | The Lua entries that send this operation, with their positional argument declarations where the entry declares any                |
| `lua_bridge`                       | `gnoblin_config_call_api` accepts the wire name itself (`bridge_operations[]`)                                                    |
| `source`, `line`                   | Where the row is                                                                                                                  |

`events.json` entries carry `channels` (`socket`, `runtime`, or both), `subscription_table` (the
name is in `native_socket_events[]`, so a socket client can subscribe to it) and `source`/`line`.
`windows` and `monitors` are listed because they are in the same table: they are the legacy
snapshot streams, not `gnoblin.` names.

A `socket` channel is recorded for every name in the subscription table, and for a literal that
reaches a socket publisher. A `runtime` channel is recorded only when a literal name reaches a
dispatcher, either directly or through a helper whose definition forwards its `event` or `name`
parameter to one, followed to the end (`dispatch_output_event`, `gnoblin_control_publish_event`).
The asymmetry is deliberate: the subscription table proves socket delivery, and a literal proves
runtime delivery, while an absent `runtime` channel means the sources do not prove it either way
rather than that the runtime never sees the event.

`cli.json` entries carry `command`, `summary`, and an `actions` list; each action carries its own
summary and line, which is why `window focus` can be inventoried as a rejected command without a
special case.

`config-keys.json` entries carry `key`, `table_index`, `runtime_overridable` and
`runtime_override_keys` (the key names allowed inside that section, or empty when every key the
validator accepts may change). The `enumerations` array records the values of `protocols`, which
is the one `gnoblin.configure` key whose accepted values are a closed list in the C sources.

## Counts

<!-- contract-counts:start -->

- `operations.json`: 136 entries, from `src/control/gnoblin-api-operations.h`
- `events.json`: 76 entries, from `src/control/gnoblin-control-focus.c`, `src/control/gnoblin-control-grabs.c`, `src/control/gnoblin-control-overrides.c`, `src/control/gnoblin-control-process.c`, `src/control/gnoblin-control-tweens.c`, `src/control/gnoblin-native-control.c`, `src/portal/grants/gnoblin-control-session-activity.c`
- `cli.json`: 22 entries (77 actions), from `src/cli/gnoblinctl-cli.c`
- `config-keys.json`: 21 entries, from `src/config/lua/gnoblin-lua-input.c`

<!-- contract-counts:end -->

## What is not extracted, and why

Every gap below needs a decision before the harness can compare it, so it is a listed gap rather
than a guessed entry. The C sources build each of these in code, in a branch per operation, which
no text extraction can read reliably.

- **Wire argument schemas.** The fields a method accepts, their types and their defaults are
  assembled in `src/control/gnoblin-native-dispatch.c` and the `gnoblin-control-*.c` handlers with
  `json_object_get_*` calls, and there is no table that declares them. The positional arguments of
  Lua entries are recorded where `lua_api[]` declares them, and a `null` there means the entry
  packs its arguments in code.
- **`gnoblinctl` options and `--json` shapes.** Options are read by name at each call site
  (`option(cli, "app-id")`) and the JSON shapes come from `src/cli/gnoblinctl-render*.c`, both
  without a declaration. Commands and actions are complete; their options are not.
- **`gnoblinctl` action to wire operation mapping.** `src/cli/gnoblinctl-dispatch.c` assigns the
  method inside per-command branches, so the mapping is not a table. `tests/gnoblinctl-cli.test.py`
  and the recorded CLI output are the gate for that mapping.
- **Per-section configuration keys.** `gnoblin.configure` accepts the sections listed in
  `config-keys.json`; which keys are legal inside each section is checked by hand-written
  validators (`src/config/gnoblin-config-validate.c`, `gnoblin-config-windows.c` and others). Only
  the sections that may change at runtime name their keys in a table, and those are recorded.
- **The `runtime` channel of events named by a variable.** A name reaches the publisher as a
  local, a ternary or the payload of an event that arrived from Mutter: `gnoblin.session.lock-requested`
  through `event_name`, `gnoblin.animation.started` and `-finished` and `gnoblin.window.activation-denied`
  through the conditional in `native_native_event`, `gnoblin.input.gesture`, which is assembled into
  its JSON payload by hand, and `gnoblin.monitor.*` and `gnoblin.config.*`. Those names are
  inventoried from the subscription table with their socket channel, and no `runtime` channel is
  claimed for them, because no literal reaches a dispatcher.
- **`mutter.*` passthrough events.** The compositor also forwards Mutter's own signals under the
  `mutter.` prefix: `mutter.touchpad.gesture`, `mutter.display.restacked`,
  `mutter.monitor-manager.monitors-changed` and five others. They are the reference
  implementation's names rather than Gnoblin's, so they are out of scope here and belong in the
  divergence ledger when the candidate stops forwarding them.
- **The Lua API surface itself.** `gnoblin.d.lua` and the annotations under `src/config/lua/api`
  are already turned into `src/config/lua/gnoblin-lua-signatures.inc` by
  `scripts/gen-lua-api.py`; this inventory links operations to Lua entries rather than repeating
  that surface.
- **Wayland protocol globals.** The plan asks for reference `wayland-info` output as well. That has
  to come from a running session, so it cannot be extracted from the sources and is not here.

## Relationship to the rest of the harness

Scenarios, fixtures and the runner live beside this directory. A scenario names a method, an event
or a command, and that name is checked against these files, so a rename in the C sources shows up as
a diff here rather than as coverage that quietly stops matching anything.
