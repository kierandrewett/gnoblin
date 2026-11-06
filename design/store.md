# Store

Status: **Proposed 2026-11-06** (D30 in `rust-compositor.md`). The invariants
are compositor policy. The options, and the items under "Needs Kieran", are
still open.

## Summary

Gnoblin gets one storage API, `gnoblin-store`. Each owner keeps its own SQLite
file behind that API. An owner is a Gnoblin crate or an extension.

Extensions never open a SQLite file. The compositor names each owner, checks
each call, and enforces the quota and the grants.

The store holds runtime state that does not belong in the hand-edited
configuration. It never holds secrets (D27), the recovery state, or the last
accepted configuration.

## Rules

Each rule is tagged. An **invariant** is compositor policy. An **option** is
open, and its choice is listed under "Needs Kieran".

| Rule                                           | Tag       |
| ---------------------------------------------- | --------- |
| Extensions never open SQLite files             | invariant |
| One writer: the compositor process             | invariant |
| One SQLite file per owner, under XDG data      | invariant |
| Namespace is the owner id, validated           | invariant |
| Values are a subset of `gnoblin_model::Value`  | invariant |
| No raw SQL for extensions                      | invariant |
| Secrets never enter the store                  | invariant |
| Configuration and grants stay in configuration | invariant |
| Uninstalled data is kept until `purge`         | invariant |
| Collections (indexed records) in v1            | option    |
| One file per namespace, or namespaced tables   | option    |
| Logs exposed through the API                   | option    |
| Default quota and value size                   | option    |
| Read-only query for an owner's own namespace   | option    |

## Today

Every persisted location found under `crates` on origin is listed below. The
last column says whether it moves to the store.

| What                    | Where                                                                                    | Decision                                               |
| ----------------------- | ---------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| Log records             | `$XDG_STATE_HOME/gnoblin/logs.sqlite3`, `gnoblin-log/src/store.rs:103-113`               | Stays. High-write sink (D23, line 2890).               |
| Session recovery        | `$XDG_STATE_HOME/gnoblin/recovery`, `gnoblin-session/src/supervisor.rs:136`              | Stays. Read before any runtime starts.                 |
| Last accepted config    | `$XDG_STATE_HOME/gnoblin/last-good-config.json`, `gnoblin-config/src/last_good.rs:45-57` | Stays. Fallback must not depend on the store.          |
| Spent activation tokens | `gnoblin-authority/src/authority.rs:288-292`, in memory                                  | Stays in memory. Never persisted.                      |
| AppChooser "remember"   | `gnoblin-portal-core/src/model/interface.rs:123`; no write code exists                   | Stays a freedesktop file (`mimeapps.list`, line 2457). |
| Layout `ctx.state`      | `gnoblin-lua/src/layout/host.rs:94`, in memory per worker                                | Stays in memory.                                       |
| Pre-snap window frame   | `design/lua-api.md:515`; no disk write found                                             | Stays in memory [INFERENCE].                           |
| Session dconf keyfile   | `compat/gnoblin-compat-gtk-dconf-settings/src/lib.rs:249`                                | Stays. Written into the caller's profile.              |
| Extension state         | None. The wasm host has no filesystem (`gnoblin-wasm/src/host.rs:457`)                   | New. The store is its first home.                      |
| Secrets                 | None in the tree. Keyring is planned (`design/keyring.md:10`)                            | Keyring (D27). Never here.                             |

Nothing persisted today has to move, so phase 1 needs no data migration. The
log keeps its own file (D23).

The consent rule is already decided: capture, input and secrets are never
remembered (`rust-compositor.md:2481`). AppChooser's "remember" is the only
remember, and it is the user's explicit choice.

Grep limits: the search covered `XDG_STATE_HOME`, `XDG_DATA_HOME`, `rusqlite`,
`.sqlite` and `recovery` only. Other persistence would not appear here.

## Owner, namespace and file

- **Owner.** A Gnoblin crate is owned by its package name, such as
  `gnoblin-lua`. An extension is owned by the id in its `extension.toml`
  (D22, `rust-compositor.md:2682`). The compositor assigns the owner at
  registration. A package cannot choose another owner's name.
- **Namespace.** The namespace is the owner id. It is 1 to 64 bytes: a lowercase
  ASCII letter or digit, then lowercase letters, digits, `.`, `-` or `_`. `..`
  is refused. `/`, NUL and upper case are refused, so a name cannot traverse a
  path or collide with another by case.
- **Reserved.** Extension ids may not start with `gnoblin-` or `gnoblin.`. A
  refused name gets `namespace-invalid`, or `namespace-taken` when another owner
  holds it.
- **File.** `$XDG_DATA_HOME/gnoblin/store/<namespace>.sqlite3`. The fallback
  follows the rule in `last_good.rs:75` with `XDG_DATA_HOME` and
  `$HOME/.local/share` [INFERENCE: data for owner files, state for logs and
  recovery]. The directory is `0700` (`last_good.rs:362`) and the file `0600`
  (`last_good.rs:420`).
- **Writer.** The compositor holds an exclusive `flock` on `store/.lock`. A
  second process opens the files read-only.
- **Journal.** `journal_mode = WAL` and a busy timeout, as the log store sets
  them (`gnoblin-log/src/store.rs:223-224`).
- **Schema.** The store owns table `kv` and its version in `user_version`. The
  owner never sees the table.

```sql
CREATE TABLE kv (
  key        TEXT PRIMARY KEY,  -- 1 to 256 bytes, see Keys
  kind       INTEGER NOT NULL,  -- value tag, see Values
  value      BLOB NOT NULL,     -- canonical encoding, version in meta
  revision   INTEGER NOT NULL,  -- bumped by each commit to the key
  expires_ms INTEGER,           -- wall-clock expiry, NULL when none
  updated_ms INTEGER NOT NULL
);
CREATE TABLE meta (name TEXT PRIMARY KEY, value BLOB NOT NULL);
-- meta holds the encoding version, the namespace byte total and its
-- revision counter. The total is updated in the same transaction.
```

## Values

A value is `gnoblin_model::Value` (`crates/gnoblin-model/src/value.rs:19`), the
enum the object model already uses for properties. The store accepts a subset.

- **Accepted:** `Bool`, `Int`, `Uint`, `Double` (finite only), `Text`, `Bytes`,
  and a `List` of accepted values.
- **Refused:** `Object(ObjectId)`. Object ids are runtime identities
  [INFERENCE: they do not survive a restart]. A refused value gets
  `value-not-storable`.
- **No map.** The model has none. Keys are flat and `/`-separated, so a map is a
  prefix. The config `Value` (`gnoblin-config/src/value.rs:30`) has a `Map`, but
  configuration is not stored.

The canonical encoding is owned by `gnoblin-store` and carries a version in
`meta`. [INFERENCE: checked against the IPC payload encoding in phase 4.]

## Keys

Keys are UTF-8, 1 to 256 bytes, with no NUL and no leading or trailing `/`.
A prefix listing matches whole segments only, so `window/1` does not match
`window/10`.

## Value API (v1)

- `get(key)` returns the value and its revision, or absent.
- `set(key, value, ttl_ms?, if_revision?)` returns the new revision. With
  `if_revision` it is a compare-and-set, and a stale revision is refused.
- `delete(key, if_revision?)` applies the same check, then removes the key.
- `list(prefix, limit, after)` returns keys in pages, without values.
- `batch(ops)` runs every operation in one transaction, or none. A batch holds at
  most 256 operations [INFERENCE: keeps one commit short].
- `watch(prefix)` emits a change after each commit, in commit order, with the keys
  and new revisions. Changes in one batch are coalesced per key.
- **TTL** is optional, with a maximum of 30 days [INFERENCE]. Expiry is checked on
  read and swept on open and on a timer. TTL is advisory and never gates a
  security decision.

### Why not raw SQL for extensions

- The schema would become public. Any owner's migration would break every reader
  that wrote SQL against it. That is the coupling the constraints rule out.
- A quota cannot be enforced on arbitrary SQL. The store counts bytes per commit,
  and a statement cannot be counted before it runs.
- `ATTACH`, triggers and temporary tables reach other files. Stopping them needs
  an authoriser, which is a long list of sharp edges.
- SQLite allows one write lock. A long extension statement would stall the writer
  that serves every owner.

Option: a read-only `query` for an owner's own namespace only. An authoriser
would allow `SELECT` and `query_only`, with a row cap and a progress-handler CPU
cap. Crate owners could use it in process. Extensions could use it only under a
`store.query` grant. Its cost is a visible SQL surface for that owner.

## Surfaces

The IPC conventions follow the object model (`rust-compositor.md`, "Object model
and IPC").

### IPC: `store` object

- `store` (core object): properties `namespaces` (list of `namespace`) and
  `directory` (read-only).
- `namespace`: properties `name`, `owner`, `kind` (`crate` or `extension`),
  `path`, `bytes`, `quota`, `keys`, `schema_version` and `state`.
- `state` is one of `ok`, `read-only`, `quarantined` or `orphaned`. `read-only`
  means reads work and writes are refused.
- Methods on `namespace`: `get`, `set`, `delete`, `list`, `batch`, `export`,
  `import`, `vacuum` and `purge`. Each checks the caller's grant first.
- Signals: `namespace.changed` (keys and revisions) and `namespace.state`.

`path` is shown to the user through `gnoblinctl` only. A caller with a grant never
receives it, so an extension cannot learn where its file is.

### Lua: `gnoblin.store`

The same functions, under power `store` (D22). A Lua sequence becomes a `List`.
Any other table, function or userdata is refused with `value-not-storable`.
Async calls return `Operation<Result>`, as `gnoblin.shortcuts.bind` does
(`design/lua-api.md:204`). `gnoblin.configure` is unaffected.

### WASM

The wasm host has no filesystem (`gnoblin-wasm/src/host.rs:457`). The store is a
host import in the extension world named by D22 (`wit = "gnoblin:ext@0.2.0"`,
`rust-compositor.md:2700`), with the same functions and values as WIT variants.

### `gnoblinctl store`

Subcommands: `list`, `get`, `set`, `delete`, `export`, `import`, `purge`, `vacuum`
and `usage`. Output comes from `gnoblin-cli-style` (`Cargo.toml:3`). The shape
follows `gnoblinctl logs` (`gnoblinctl/src/logs.rs`).

- While the compositor runs, writes go through IPC, so the single writer holds.
- `list`, `get`, `usage` and `export` may read the file directly, because WAL
  allows readers [INFERENCE].
- `set`, `delete`, `import` and `purge` need the compositor. Without it they fail
  with `compositor-not-running`.

### Rust: `gnoblin-store`

A library crate in the foundation set of `xtask/src/deps.rs` (`FOUNDATION`,
line 34). It depends on `rusqlite` with `bundled`, on `gnoblin-model` for `Value`
and on `gnoblin-types`. It names no engine, compat or compositor crate. The
dependency rules are checked by `xtask`, and the crate must be added to
`FOUNDATION` in phase 2.

A crate never opens a path. The compositor hands it a `Namespace` handle for its
own owner at construction.

## Access control (D22)

- **`store`** reads and writes the package's own namespace. A package gets it only
  when it declares `[powers.store]` and host configuration grants it. D22 grants
  only what host configuration lists (`rust-compositor.md:2665`).
- **`store.read:<ns>`** and **`store.write:<ns>`** name one other namespace each.
  There are no globs. A cross-namespace grant is never implicit.
- **Crate namespaces** are readable by an extension only under a
  `store.read:gnoblin-<crate>` grant. An extension can never write one.
- **Grants live in host configuration**, not in the store, so the user can read
  them in the config file.
- **What the user sees.** `gnoblinctl store usage` lists each namespace with its
  owner, kind, bytes, quota, state and the grants that reach it.
- **Quota** is per namespace, in bytes. The default is open (see "Needs Kieran").
- **Uninstall.** When a package is removed its namespace becomes `orphaned`. The
  data is kept and listed in `usage`. A reinstall with the same id attaches to it
  again. `gnoblinctl store purge <ns> --yes` removes the file. Crate namespaces
  are never orphaned.

## Failure modes

- **Corrupt file.** `quick_check` or open fails. The file and its `-wal` and `-shm`
  are renamed `<name>.sqlite3.corrupt-<UTC>`. The store starts empty, logs
  `error!`, and the state is `quarantined`. Start never blocks.
- **Disk full.** The write fails with `disk-full`. Reads continue. One `warn!` is
  logged until the next success.
- **Quota exceeded.** Refused before the transaction opens, with `quota-exceeded`,
  the current total and the limit. A batch is refused whole.
- **Concurrent writers.** One writer, the compositor, holds `store/.lock`. A second
  process reads only.
- **Migration fails.** The step's transaction rolls back and `user_version` stays.
  The state is `read-only`, and the failure is logged.
- **Huge value.** Sized before the transaction. `value-too-large` names the limit,
  and nothing is written.
- **Owner crashes mid-batch.** A transaction lives inside one store call, and none
  spans a message. SQLite discards uncommitted WAL frames on the next open.
- **Clock changes with TTL.** Expiry uses wall-clock milliseconds. A step back
  keeps a value longer; a step forward expires it early. TTL gates no security
  decision.
- **Name collision or traversal.** Validated before a path is built. Refused as
  `namespace-invalid` or `namespace-taken`.
- **Store unavailable at boot.** The directory cannot be made or is not writable.
  The compositor starts. Every namespace is `read-only`, owners get code defaults,
  and the cause is logged.
- **Export and backup.** `export` writes JSON Lines: a header line (format 1,
  namespace, owner, schema version, time), then one line per key. Bytes are
  base64. `import` runs in one transaction and refuses an owner mismatch unless
  `--as` names another owner.

## Interactions

- **Config and store.** Configuration is declarative and hand-edited. The store
  is runtime state.
- **Never moves to the store:** hand-edited settings (`gnoblin.configure`,
  keybindings, workspaces, window rules); extension grants and quotas; the recovery
  state; the last accepted configuration; capability grants; capture, input and
  secret consent.
- **Optional later:** an owner's runtime memory that a user would not write by
  hand, as an opt-in.
- **Layers (D28) and window rules.** Window rules are configuration. A remembered
  placement, if wanted, is a key under the owning crate's namespace, off by
  default.
- **Portals.** AppChooser's "remember" stays a freedesktop file. Capture, input
  and secrets are never remembered. The store may hold transient portal state.
- **Keyring (D27).** `gnoblin-service-keyring` is planned with its own
  SQLCipher store (`design/keyring.md:13`). The store holds no secret. It is
  plain SQLite, so no secret may be written there. No API returns a secret, and a
  namespace named for the keyring is refused.
- **Extensions.** Only through grants, as above.
- **`gnoblin.configure`.** Unaffected. `gnoblin.settings` stays a read-only
  snapshot (`design/lua-api.md:188`).
- **Logs (D23).** A separate file and a separate writer thread. A read view is an
  option.

## Phased implementation

Each phase is one reviewable commit. Its headless tests run on Agent37.

1. **Decision.** This file, D30 and the D22 row. Verify: prettier and the review
   hook. No code changes.
2. **`gnoblin-store` library.** Name rules, file layout, lock, WAL, quarantine,
   `kv` operations, the value subset, size, quota and TTL. Verify: one unit test
   per failure mode, on temp directories, and the `xtask deps` foundation rule.
3. **Compositor registry.** Owners from crates and configured packages, grants,
   and the read-only fallback at boot. Verify: a headless boot with an unwritable
   directory starts with `read-only` namespaces.
4. **IPC `store` object.** Schema in `gnoblin-schema`; regenerate `gnoblin-proto`,
   `gnoblin-model` and `gnoblin-client`. Verify: a round trip over the IPC.
5. **`gnoblinctl store`.** `list`, `get` and `usage` first, offline. Then `set`
   and `delete`, through IPC. Verify: golden output from `gnoblin-cli-style`.
6. **Lua `gnoblin.store`.** Behind power `store`. Verify: a missing grant is
   refused; a cross-namespace call is refused; value refusals return the named
   errors.
7. **Export, import, purge, vacuum and orphaning.** Verify: an export and import
   round trip; `purge` refuses without `--yes`.
8. **Fault injection.** Quarantine, disk full and migration failure. Verify: one
   test per failure mode above.
9. **Change signals.** `namespace.changed` on IPC and `watch` in Lua. Verify:
   commit-order delivery and coalescing within a batch.
10. **WASM host import.** After the D22 wasm gate. Verify: a component test.

Collections and the log read view wait for the choices below.

## Needs Kieran

Each item is an **option**. None is decided here.

1. **Collections (indexed records) in v1 or later.**
    - Pros: owners can query by field without scanning prefixes.
    - Cons: a second API, with per-owner tables and migrations.
    - Risks: owners write their own schemas, which reopens the coupling the
      constraints rule out.
    - Choice: key-value only in v1, with collections later, or both in v1.
2. **One file per namespace, or one file with namespaced tables.**
    - Pros, one file per namespace: one lock per owner, so a failure stays local.
    - Pros, one file with tables: one backup and one file to inspect.
    - Cons, one file per namespace: more files to list.
    - Cons, one file with tables: one corruption affects every owner, and one
      writer lock is shared.
    - Risks: the constraints lean to per-owner files. This item is whether that
      lean holds.
3. **Logs exposed through the API.**
    - Pros: one query surface for tools.
    - Cons: a second reader path into a high-write file.
    - Risks: a read view must not hold the WAL open for long.
4. **Default quota and value size.**
    - Example: 16 MiB per namespace and 64 KiB per value, or 1 MiB and 4 KiB.
    - Pros of the smaller numbers: bounded disk use.
    - Cons of the smaller numbers: a legitimate cache fails sooner.
    - Risks: a quota that is too small breaks owners silently in the field.
5. **Read-only query for an owner's own namespace.**
    - Pros: owners keep SQL without a public schema.
    - Cons: a visible SQL surface for that owner, and an authoriser to maintain.
    - Risks: the authoriser must block `ATTACH` and every write. A bug there would
      breach the namespace boundary.
