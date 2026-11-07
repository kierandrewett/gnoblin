# Store

Status: **Proposed 2026-11-06** (D30 in `rust-compositor.md`). The API and the
decided items are Kieran's, and sit under "Decided". Where a fact was checked
in the tree, the file and line are given.

## Summary

Gnoblin keeps data in stores. You open a store by name, then use the same few
verbs on it: `put`, `get`, `delete`, `list`, `search`, `watch` and `batch`. A
document is just a value in a `kv` store. A log is a `log` store, and a secret
is a value that only the keyring hands out.

You never ask what a store can do. The verbs are the same everywhere. Providers
decide how a store works inside, and that is described in "Provider side". The
rest of this file is for consumers first.

## The API

Every store has these verbs. The kind decides what a verb does on that store. For
example, `delete` is refused on a log, where retention removes records instead.
The kind's rules are under "Kinds" in "Provider side".

| Verb              | What it does                                                                         |
| ----------------- | ------------------------------------------------------------------------------------ |
| `open(name)`      | Returns a handle. The owner creates the store on first open.                         |
| `put(key, value)` | Adds or replaces a value and returns its revision. On `log`, appends a record.       |
| `get(key)`        | Returns the value and revision, or nothing. On `log`, the newest record for the key. |
| `delete(key)`     | Removes a value. Refused on `log`, where retention removes records instead.          |
| `list(prefix)`    | Returns keys under a prefix, whole segments only, in pages.                          |
| `search(query)`   | Finds values with the operators under "Query". Never raw SQL.                        |
| `watch(prefix)`   | Streams changes under a prefix, in commit order, with key and revision.              |
| `batch(ops)`      | Runs puts and deletes together. All of them apply, or none do.                       |

`put` and `delete` take an optional `if_revision`. The write applies only if the
stored revision still matches. Otherwise it fails with `revision-conflict`.

The errors are the same in every front end:

| Error                  | Meaning                                                              |
| ---------------------- | -------------------------------------------------------------------- |
| `store-locked`         | A `secret` store is locked. See "Secret values".                     |
| `store-unavailable`    | The store could not open. Its values cannot be read or written.      |
| `read-only`            | The store opened with `writable = false`.                            |
| `quota-exceeded`       | The store is over its quota. The whole batch is refused.             |
| `value-too-large`      | The value is over 64 KiB. Nothing is written.                        |
| `value-not-storable`   | The value has a type the store does not keep. See "Values and keys". |
| `revision-conflict`    | The `if_revision` did not match the stored revision.                 |
| `not-allowed`          | The verb does not apply to this kind, such as `delete` on `log`.     |
| `grant-missing`        | The caller has no grant for another owner's store.                   |
| `secret-value-refused` | A generic front end asked for a `secret` value. See "Secret values". |

### Five common tasks

Each task uses the same verbs in Rust, Lua and `gnoblinctl`. Lua writes and
reads return an `Operation`, as in `design/lua-api.md:204`. The examples leave
out the wait.

**Save settings.**

```rust
let settings = gnoblin_store::open("settings")?;
settings.put("theme", "dark")?;
let theme = settings.get("theme")?;
```

```lua
local settings = gnoblin.store.open("settings")
settings:put("theme", "dark")
local theme = settings:get("theme")
```

```sh
gnoblinctl store put settings theme dark
gnoblinctl store get settings theme
```

**Keep a document.** The document is a value. Its key is its path.

```rust
let documents = gnoblin_store::open("documents")?;
documents.put("notes/today", text)?;
```

```lua
gnoblin.store.open("documents"):put("notes/today", text)
```

```sh
gnoblinctl store put documents notes/today --file notes.md
```

**Remember a choice.** This example keeps the app that opens each file type.

```rust
let choices = gnoblin_store::open("app-choices")?;
choices.put("image/png", "org.example.Viewer.desktop")?;
```

```lua
gnoblin.store.open("app-choices"):put("image/png", "org.example.Viewer.desktop")
```

```sh
gnoblinctl store put app-choices image/png org.example.Viewer.desktop
```

**Record history.** The `history` store is a log. Each `put` adds a record, and
the store stamps its time.

```rust
let history = gnoblin_store::open("history")?;
history.put("launch", "firefox")?;
```

```lua
gnoblin.store.open("history"):put("launch", "firefox")
```

```sh
gnoblinctl store put history launch firefox
```

**Look something up.**

```rust
let hits = history.search(Query::new()
    .key_prefix("launch")
    .since(Time::now() - Duration::days(7))
    .contains("fire")
    .newest_first()
    .limit(20))?;
```

```lua
local hits = gnoblin.store.open("history"):search({
  key_prefix = "launch", since = "7d", contains = "fire", order = "newest", limit = 20,
})
```

```sh
gnoblinctl store search history --key-prefix launch --since 7d --contains fire --limit 20
```

### Query

`search` takes these operators. Any mix of them works, and none of them is SQL.

- Equality on the key.
- Key prefix, on whole segments.
- Time range, on the time the store stamped at `put`.
- Number range, on a number value.
- Contains text, using a full-text index where the backend has one.
- Order by time or key, newest or oldest first.
- Limit, with paging.
- Count, and group-by-count.

No consumer gets a database handle, and no query string is parsed as SQL.

### Values and keys

A value is a `gnoblin_model::Value`, the same enum the object model uses for
properties (`value.rs:19`). The store accepts a subset: `Bool`, `Int`, `Uint`,
`Double` (finite only), `Text`, `Bytes`, and a `List` of accepted values. It
refuses `Object(ObjectId)`, because object ids are runtime identities
[INFERENCE: they do not survive a restart]. Refusals return `value-not-storable`.

Keys are UTF-8, 1 to 256 bytes, with no NUL and no leading or trailing `/`. A
prefix listing matches whole segments only, so `window/1` does not match
`window/10`.

A key may contain `/`, so `mime/image/png` is one key. `kv` and `log` encode
values canonically. The encoding is owned by `gnoblin-store` and carries a version
[INFERENCE: checked against the IPC payload encoding in phase 4].

### Names and grants

A store name is 1 to 64 bytes: a lowercase ASCII letter or digit, then
lowercase letters, digits, `.`, `-` or `_`. `..` is refused, and `/`, NUL and
upper case are refused. An extension id may not start with `gnoblin-` or
`gnoblin.`.

- **`store`** (D22) gives a package its own store. It needs `[powers.store]`,
  and host configuration must grant it.
- **`store.read:<name>` and `store.write:<name>`** name one other store each.
  There are no globs. A cross-store grant is never implicit.
- **Crate stores** are readable by an extension only under a
  `store.read:gnoblin-<crate>` grant. An extension can never write one.
- **Grants live in host configuration**, not in the store, so the user can read
  them in the config file.
- **`gnoblinctl store usage`** lists each store with its owner, kind, bytes,
  quota, state and the grants that reach it.
- **Uninstall.** A removed package's stores are marked `orphaned` in `usage`.
  That is a flag, not a lifecycle state. The data is kept. A reinstall with the
  same id attaches to it again. `gnoblinctl store purge <name> --yes` removes
  the data. Crate stores are never orphaned.

### Secret values

A `secret` store is the keyring (D27). Generic front ends never return a secret
value. They are Lua, IPC, `gnoblinctl` and WASM. For a `secret` store they show
metadata only: names, counts, usage, state and lock state.

A secret value comes only from the keyring's own path. That path applies the
consent policy of D27: confirm the first retrieval per application.

A `secret` store is never exported in plaintext. The only export is an encrypted
backup of the whole store file. `gnoblinctl store get` on a secret item returns
`secret-value-refused`, and names the keyring tooling (`secret-tool`,
`design/keyring.md`).

Decided in principle: follow gnome-keyring and KDE wallet. The exact behaviour
is being verified by the KeyringResearch lane, and this file does not state it.

## Provider side

This part is for people who write a provider. Consumers can skip it.

A provider says how its store works inside. The compositor turns each verb into
one of a few internal operations, and it enforces the rules of the kind. A
capability is the internal name for one of those operations. A provider reports
its set at open. Consumers never see the set, and they never check it.

| Capability (internal) | Verbs it serves                                 | Kinds                 |
| --------------------- | ----------------------------------------------- | --------------------- |
| `Items`               | `get`, `put`, `delete`, `list`, `if_revision`   | `kv`, `secret`        |
| `Append`              | `put` on a log, which appends                   | `log`                 |
| `Search`              | `search`, with the operators under "Query"      | `kv`, `log`, `secret` |
| `Watch`               | `watch`, one change per commit, in commit order | `kv`, `secret`        |
| `Transact`            | `batch`, all or nothing                         | `kv`, `secret`        |
| `Lock`                | Lock state and unlock, on the keyring's path.   | `secret`              |

A capability may be dropped later, but never added after open. A verb that a
store cannot serve fails with its error from the table above. Lua hides a
method only when the store kind never has it.

### Kinds

A store has one kind. The kind sets the rules that a consumer sees.

| Kind     | Holds                    | `put`                               | `delete`          | Lifetime                        |
| -------- | ------------------------ | ----------------------------------- | ----------------- | ------------------------------- |
| `kv`     | Plain values, by key     | Adds or replaces. 64 KiB per value. | Removes the value | Until deleted, or purged        |
| `log`    | Records, in time order   | Appends a record. Never replaces.   | `not-allowed`     | Retention by age and by size    |
| `secret` | Secrets from the keyring | Keyring path only                   | Keyring path only | Until deleted. Encrypted (D27). |

Only Gnoblin defines kinds, in `gnoblin-store`. An extension can open a `kv`
store with the `store` power. It cannot define a new kind.

A store that opens but cannot write has `writable = false`. Its writes return
`read-only`. Reads still work. The lifecycle keeps its six states. See
"Provider side".

Each `kv` store has a quota of 16 MiB, and each value has a limit of 64 KiB.
Host configuration can override the store quota. Logs use retention instead of
a quota.

### Lifecycle

The states are `opening`, `ready`, `locked`, `unavailable`, `quarantined` and
`closing`. Only the registry moves a store between them. A provider reports a
cause, and the registry decides the transition.

| From          | To            | When                                               |
| ------------- | ------------- | -------------------------------------------------- |
| `opening`     | `ready`       | Backend opened and its integrity check passed      |
| `opening`     | `unavailable` | Backend cannot open, such as a missing directory   |
| `opening`     | `quarantined` | Integrity check failed; the file is moved aside    |
| `ready`       | `locked`      | Lock requested, or screen locked (`secret` stores) |
| `locked`      | `ready`       | Unlock accepted with a valid credential            |
| `ready`       | `unavailable` | Backend lost: disk gone, or provider disconnected  |
| `unavailable` | `opening`     | Retry on the next open                             |
| `quarantined` | `closing`     | Purge, or the user restores a copy                 |
| any           | `closing`     | Provider removed, purged, or compositor shutdown   |

`writable = false` is a flag on a `ready` store, not a seventh state. Kieran
chose the flag.

### Who provides

A provider is a Gnoblin crate, the keyring service process, an extension with
the `store` power, or a wasm component later. An out-of-process provider speaks
the same provider interface over IPC. The keyring is one such provider. An
extension may provide a store of an existing kind. It may not add a kind.

### Registration rules

These rules copy the shape of the layer registry (`design/layers.md`,
"Registration").

- **Unique names.** A store name belongs to its first registrant. Built-in
  names are reserved for their providers. A refused name gets
  `namespace-invalid` or `namespace-taken`.
- **Attribution.** The compositor sets `provider`. A caller never sets it.
- **Kind by name.** The provider names a kind from the closed list.
- **Options within bounds.** The provider sets retention, but only inside the
  kind's bounds. Quota comes from host configuration.
- **Limits.** Each provider has a maximum store count and a maximum total of
  bytes. The numbers are proposed.

### Platform pattern: registry, provider, grants, lifecycle

Layers (D28) and stores (D30) use one shape. Any later registry should use it
too. This section names the shape, so a new registry does not invent its own.

| Part         | Layers (D28)                          | Stores (D30)                        |
| ------------ | ------------------------------------- | ----------------------------------- |
| Registry     | `LayerRegistry` in `gnoblin-layers`   | `StoreRegistry` in `gnoblin-store`  |
| Providers    | Gnoblin, extensions, Wayland surfaces | Gnoblin crates, keyring, extensions |
| Names        | Owned by the first registrant         | Owned by the first registrant       |
| Closed set   | Bands, for ordering                   | Kinds, for policy                   |
| Capabilities | Scope, input, `secure`, content kind  | Internal operations, listed above   |
| Lifecycle    | Live until target or provider ends    | The six states above                |
| Invariants   | Compositor-enforced                   | Compositor-enforced                 |

A closed set fits an enum, so bands and kinds are enums. Internal operations
are trait objects, discovered at runtime, so a provider reports only what it
has. D28 and D30 point at each other, and each keeps its own invariants.

### Rust sketch

Signatures only. The bodies are in the phase 2 commit. Consumers use the
`Store` verbs, not these traits.

```rust
/// Common core. Every store implements this. Object-safe.
pub trait Store: Send + Sync {
    fn name(&self) -> &StoreName;
    fn kind(&self) -> KindId;
    fn state(&self) -> LifecycleState;
    fn usage(&self) -> Usage;
    fn writable(&self) -> bool;

    // One accessor per internal operation. None when the provider lacks it.
    fn as_items(&self) -> Option<&dyn Items> { None }
    fn as_search(&self) -> Option<&dyn Search> { None }
    fn as_append(&self) -> Option<&dyn Append> { None }
    fn as_lock(&self) -> Option<&dyn Lock> { None }
    fn as_watch(&self) -> Option<&dyn Watch> { None }
    fn as_transact(&self) -> Option<&dyn Transact> { None }
}

pub enum LifecycleState { Opening, Ready, Locked, Unavailable, Quarantined, Closing }

pub trait Items {
    fn get(&self, key: &ItemKey) -> Result<Option<Item>, StoreError>;
    fn put(&self, item: Item, if_revision: Option<Revision>) -> Result<Revision, StoreError>;
    fn delete(&self, key: &ItemKey, if_revision: Option<Revision>) -> Result<(), StoreError>;
}

pub struct Item {
    pub key: ItemKey,
    pub value: Value,
    pub revision: Revision,
}

pub trait Search {
    fn search(&self, query: &Query, page: Page) -> Result<Vec<ItemRef>, StoreError>;
}

pub trait Append {
    fn append(&self, record: Record) -> Result<Seq, StoreError>;
    fn read_from(&self, seq: Seq, limit: u32) -> Result<Vec<Record>, StoreError>;
}

pub trait Lock {
    fn lock(&self) -> Result<(), StoreError>;
    fn unlock(&self, credential: Credential) -> Result<(), StoreError>;
}

pub trait Watch {
    fn subscribe(&self, filter: &WatchFilter) -> Result<Subscription, StoreError>;
}

pub trait Transact {
    fn batch(&self, ops: Vec<Op>) -> Result<Vec<Revision>, StoreError>;
}

/// Provider side. The registry calls `open` to start, unlock or retry.
pub trait StoreProvider: Send + Sync {
    fn open(&self, spec: &StoreSpec) -> Result<Arc<dyn Store>, StoreError>;
}

pub struct StoreSpec {
    pub name: StoreName,
    pub kind: KindId,
    pub options: KindOptions, // bounded by the kind
}

impl StoreRegistry {
    pub fn register(&mut self, provider: ProviderId, spec: StoreSpec)
        -> Result<StoreHandle, RegisterError>;
}
```

**Dyn or enum.** Internal operations are trait objects, so a provider reports
only what it has. An enum would force a compositor change for each new
operation. Stores have no hot path: calls cross IPC or a bounded write.

## Consumers

Each front end offers the same verbs and the same errors. Each shows the
properties `name`, `kind`, `provider`, `state`, `writable` and `usage`.

- **Lua: `gnoblin.store.open(name)`.** Returns a handle with the verbs above.
  Async calls return `Operation<Result>`, as in `design/lua-api.md:204`.
  Opening another owner's store needs a grant.
- **IPC: `store` object.** The same verbs, gated by grant. The verb `list` is `keys`
  on the wire, because `list` is a WIT keyword. The signal `changed` carries keys and revisions.
- **`gnoblinctl store`.** Ships `list`, `usage` and `get`, read-only. They open
  the store file directly and refuse a store whose file does not exist, so the
  read verbs never create one. `put`, `delete`, `search`, `export` and `purge`
  are not offered. `put` and `delete` are blocked on the grant design (Needs
  Kieran): while the compositor holds a store, a second writer would break the
  one-writer WAL rule, so writes go through IPC once grants exist.
- **WASM.** A host import in the extension world (D22, `gnoblin:ext@0.2.0`,
  `rust-compositor.md:2700`). The same functions, with values as WIT variants.
- **Secret Service adapter.** The D-Bus Secret Service API (keyring, D27) is an
  adapter over a `secret` store. The keyring service runs it.
- **Secret portal adapter.** The Secret portal serves per-app secrets from the
  same `secret` store. Per-app consent is unchanged (D27).

The keyring stays a separate service process with its own SQLCipher file (D27).
It speaks the provider interface, so `gnoblinctl store list` shows it. The
Secret Service API keeps its own D-Bus surface.

## Backends

A backend is an implementation detail of a provider. The contract is the store
and its name, not the file.

| Kind     | Default backend                                                                                                   |
| -------- | ----------------------------------------------------------------------------------------------------------------- |
| `kv`     | SQLite, one file per store: `$XDG_DATA_HOME/gnoblin/store/<name>.sqlite3`                                         |
| `log`    | SQLite, `<root>/log.sqlite3` for the kind, `<root>/<name>.log.sqlite3` for named stores. Replaces `logs.sqlite3`. |
| `secret` | SQLCipher file owned by the keyring service (D27, `design/keyring.md:13`)                                         |

One file per store is decided. A failure stays local to one store.

## Rules

Each rule is tagged. An **invariant** is compositor policy, and no provider or
extension may change it. An **option** is a value a provider sets within its
grant.

| Rule                                                     | Tag       |
| -------------------------------------------------------- | --------- |
| Extensions never see a file path or open a file          | invariant |
| One writer per backend: its owning process               | invariant |
| Names are owned by the first registrant                  | invariant |
| The compositor sets `provider`                           | invariant |
| Lifecycle moves only through the registry                | invariant |
| No raw SQL reaches any consumer                          | invariant |
| Only Gnoblin defines kinds                               | invariant |
| Generic front ends never return a `secret` value         | invariant |
| `secret` export is encrypted whole-file only             | invariant |
| Secrets never enter a `kv` or `log` store (D27)          | invariant |
| Grants and provider options live in host configuration   | invariant |
| Uninstalled data is kept until `purge`                   | invariant |
| Capability set of each kind, within its bounds           | option    |
| Retention for `log`, within its bounds                   | option    |
| Quota of a `kv` store, overridable in host configuration | option    |

## Today

Every persisted location under `crates` on origin is listed below. The last
column says whether it moves to a store.

| What                    | Where                                                                                    | Decision                                        |
| ----------------------- | ---------------------------------------------------------------------------------------- | ----------------------------------------------- |
| Log records             | `$XDG_STATE_HOME/gnoblin/logs.sqlite3`, `gnoblin-log/src/store.rs:103-113`               | Moves to kind `log` now; migrated once.         |
| Session recovery        | `$XDG_STATE_HOME/gnoblin/recovery`, `gnoblin-session/src/supervisor.rs:136`              | Stays. Read before any runtime starts.          |
| Last accepted config    | `$XDG_STATE_HOME/gnoblin/last-good-config.json`, `gnoblin-config/src/last_good.rs:45-57` | Stays. Fallback must not depend on a store.     |
| Spent activation tokens | `gnoblin-authority/src/authority.rs:288-292`, in memory                                  | Stays in memory. Never persisted.               |
| AppChooser "remember"   | `gnoblin-portal-core/src/model/interface.rs:123`; no write code exists                   | Moves to a `kv` store (decided).                |
| Layout `ctx.state`      | `gnoblin-lua/src/layout/host.rs:94`, in memory per worker                                | Stays in memory.                                |
| Pre-snap window frame   | `design/lua-api.md:515`; no disk write found                                             | Stays in memory [INFERENCE].                    |
| Session dconf keyfile   | `compat/gnoblin-compat-gtk-dconf-settings/src/lib.rs:249`                                | Stays. Written into the caller's profile.       |
| Extension state         | None. The wasm host has no filesystem (`gnoblin-wasm/src/host.rs:457`)                   | New. A `kv` store is its first home.            |
| Secrets                 | None in the tree. Keyring is planned (`design/keyring.md:10`)                            | A `secret` store (D27). Never in `kv` or `log`. |

Logs move, once, in phase 3, by a one-off migration. The old file is renamed, not
deleted, so a failed migration leaves it as it was.

AppChooser's choice moves to a `kv` store when the AppChooser portal lands. No
phase below covers that; it needs phase 2 first.

Extension state moves to a `kv` store with the wasm import (phase 10).

The consent rule is already decided: capture, input and secrets are never
remembered (`rust-compositor.md:2481`). AppChooser's "remember" is the only
remember, and it is the user's explicit choice.

Grep limits: the search covered `XDG_STATE_HOME`, `XDG_DATA_HOME`, `rusqlite`,
`.sqlite` and `recovery` only. Other persistence would not appear here.

## Failure modes

- **Corrupt file.** `quick_check` or open fails. The file and its `-wal` and
  `-shm` are renamed `<name>.sqlite3.corrupt-<UTC>`. The store starts empty, logs
  `error!`, and moves to `quarantined`. Start never blocks.
- **Disk full.** The write fails with `disk-full`. Reads continue. One `warn!`
  is logged until the next success.
- **Quota exceeded.** Refused before a transaction opens, with `quota-exceeded`,
  the current total and the limit. A batch is refused whole.
- **Locked.** A `secret` store that is `locked` is handled as decided in
  principle (see "Secret values"). This file does not state the behaviour.
- **Secret value asked of a generic front end.** `get` on a `secret` item returns
  `secret-value-refused` in every front end. No value is read from the file.
- **Plaintext export asked for.** `export` of a `secret` store returns
  `secret-export-refused`, and names the encrypted whole-file backup.
- **Migration fails.** The transaction rolls back, and the old `logs.sqlite3` is
  untouched. The failure is logged, and the next start retries.
- **Huge value.** Sized before the transaction. `value-too-large` names the
  limit, and nothing is written.
- **Owner crashes mid-batch.** A transaction lives inside one store call. SQLite
  discards uncommitted WAL frames on the next open.
- **Clock changes with TTL.** Expiry uses wall-clock milliseconds. TTL gates no
  security decision.
- **Store unavailable at boot.** The directory cannot be made or is not writable.
  The compositor starts. The store is `unavailable`, owners get code defaults,
  and the cause is logged.
- **Export and backup.** `export` writes JSON Lines: a header (format 1, name,
  owner, kind, time), then one line per key, with bytes in base64. A `secret`
  store is refused here. `import` runs in one transaction and refuses an owner
  mismatch unless `--as` names another owner.

## Interactions

- **Config and store.** Configuration is declarative and hand-edited. Stores
  hold runtime state.
- **Never moves to a store:** hand-edited settings, grants and provider options,
  the recovery state, the last accepted configuration, and capture, input and
  secret consent.
- **Layers (D28) and window rules.** Window rules are configuration. A remembered
  placement, if wanted, is a key in the owning crate's `kv` store, off by default.
- **Portals.** AppChooser's "remember this choice" persists in a `kv` store
  (decided). Other apps read `mimeapps.list`, so whether the chooser still writes
  that file is for the portal lane to settle [INFERENCE].
- **Keyring (D27).** The keyring is a `secret` provider. Its decisions are in
  `design/keyring.md` and are unchanged.
- **`gnoblin.configure`.** Unaffected. `gnoblin.settings` stays a read-only
  snapshot (`design/lua-api.md:188`).

## Phased implementation

Each phase is one reviewable commit. Its tests are named here but not run.

1. **Decision.** This file, D30, the D28 pointer and the D27 provider note.
   Verify: prettier and the review hook. No code.
2. **`gnoblin-store` core.** `Store`, the lifecycle, `StoreRegistry`, and the
   `kv` provider with `open`, `put`, `get`, `delete` and `list`. File layout,
   lock, WAL, quota, `writable` and quarantine. Verify: one unit test per failure
   mode on temp directories, and the foundation rule in `xtask/src/deps.rs`
   (`FOUNDATION`, line 34).
3. **`log` kind and migration.** `put` appends, and `get`, `list` and `search`
   read. Retention by age and size. The one-off migration of `logs.sqlite3`, and
   the log writer moves behind the `gnoblin-log` facade. Verify: row counts match
   after migration, retention by age and size, and no second reader holds the WAL.
4. **IPC `store` object.** The verbs for `kv` and `log`, gated by grant. Verify:
   a round trip over IPC. Regenerate `gnoblin-proto`, `gnoblin-model` and
   `gnoblin-client`.
5. **`gnoblinctl store`.** `list`, `usage` and `get` are shipped, read-only.
   Golden output from `gnoblin-cli-style` is not yet checked in. `put` and
   `delete` are blocked on the grant design (Needs Kieran).
6. **Lua `gnoblin.store.open`.** Deferred with phase 9. Today one shared
   `gnoblin` global serves every script, so the compositor cannot attribute a
   store call to an extension (`design/layers.md`, scoping note 17). It
   returns `not-allowed` for any verb that does not apply to a kind once
   attribution exists.
7. **`search`, `batch` and `watch` for `kv`.** `search` takes the operators under
   "Query". Verify: commit-order delivery, batch coalescing per key, query operators
   against a fixture, and no raw SQL path.
8. **`secret` kind and the keyring provider.** `Lock`, after the D27 key source is
   chosen. Verify: the Secret Service and portal adapters, `gnoblinctl store list`
   shows the keyring store, and every generic front end refuses a `secret` value.
9. **Export, import, purge and orphaning.** Verify: an export and import round
   trip, and `purge` refuses without `--yes`.
10. **WASM host import.** After the D22 wasm gate. Verify: a component test.
11. **Fault injection.** Quarantine, disk full, lock, migration failure. Verify:
    one test per failure mode above. Also: a plaintext `secret` export is refused,
    and a `secret` value read through each generic front end (IPC, Lua,
    `gnoblinctl`, WASM) returns `secret-value-refused`.

## Decided

Decisions from Kieran. Each one stands unless he reopens it.

- **The API is the verbs above.** Kieran: "you don't have to worry about
  capabilities". Capabilities are an internal provider detail.
- **A document is a value in a `kv` store.** Kieran: "you store a document in the
  kv store".
- **Extensions do not define new kinds.** Kinds are compiled into `gnoblin-store`.
- **`kv` ships first.** Kieran: "uh sure".
- **`writable` is a flag.** It is not a seventh lifecycle state.
- **One file per store.**
- **`kv` quotas: 64 KiB per value, and 16 MiB per store.** Host configuration can
  override the store quota.
- **Logs move onto the store now, as kind `log`.** A one-off migration moves the
  existing `logs.sqlite3`. The log writer keeps its facade, and no second reader
  holds the WAL open.
- **Query operators:** equality, key prefix, time and number ranges, contains
  text (SQLite full-text where available), order by, limit, count, and
  group-by-count. Never raw SQL.
- **AppChooser "remember this choice" persists in a `kv` store.**
- **`secret` locked behaviour: decided in principle.** Kieran: follow what GNOME
  and KDE do. The exact behaviour is being verified by the KeyringResearch lane,
  and this file does not state it.
- **Log key (decided by the implementing lane, 2026-11-07, reopen if wrong).**
  A `log` record carries an indexed `key` column. The `gnoblin-log` facade sets
  `key` to the record's area. `put(key, value)` appends, and `get(key)` returns
  the newest record for that key. A forward-only migration adds the column, with
  existing rows set to `key = area`, and the migration is idempotent.
- **Log file name (decided by the implementing lane, 2026-11-07, reopen if wrong).**
  A named `log` store is `<root>/<name>.log.sqlite3`, and `<root>` is the directory
  the `kv` files use. The `.log` part is needed because the store does not record
  each file's kind, so a `kv` and a `log` store with one name would share a file.
  A `kv` name has no `.`, so the two cannot collide.
- **Scope split (Main, 2026-11-07).** Phase 4 lands the `kv` verbs first as one
  commit. The `log` verbs follow as a second commit. Phases 5 and 6 build on both.
- **Grant finding (2026-11-07, blocks phase 4 powers, no schema added).** The
  extension grants frame cannot yet carry `store`, `store.read:<name>` or
  `store.write:<name>`. Today `gnoblin-config` `extensions` entries name a
  schema-backed namespace and the restricted socket roles that receive it
  (`grants = [roles]`), and `plan()` pushes grants only onto privileged
  connections. Accepted client sockets get an empty frame. Store grants are not
  schema-backed namespaces, so they need a host-config decision first: either a
  separate store-grant table, or per-role `store` entries. The `[powers.store]`
  manifest table does not exist in the parser yet (D22).
- **Phase 4 scope (Main, 2026-11-07).** Grants stay as they are: no schema is
  added. A connection reaches only its own store, where the owner is the role or
  extension id the compositor attributes to it. Cross-store `store.read:<name>`
  and `store.write:<name>` grants, and the `[powers.store]` manifest table, are
  deferred to "Needs Kieran". A test refuses namespace escape.

## Needs Kieran

- **Cross-store grants: where they are declared.** Two options. (1) A separate
  store-grant table in host configuration, naming the store, the connection and
  read or write. It is explicit and easy to audit, but it is a new config table
  with its own schema. (2) Per-role `store` entries on the existing extension
  `grants` shape. It reuses the plumbing, but a store name is not a schema-backed
  namespace, so the parser must accept a second kind of entry. Trade-off: option
  1 adds schema, option 2 bends an existing one.
- **`[powers.store]` manifest table (D22).** The manifest parser has no powers
  table yet. Decide whether manifests may request `store` at all, or whether only
  host configuration grants it.
- **Plain clients get no store yet.** A connection that no privileged role owns is
  refused with `no-store-owner`. Deciding how ordinary clients reach a store is open.

## Change log

- 2026-11-06: API-first rewrite. Consumers see one set of verbs and one set of
  errors. Capabilities and kinds move to "Provider side". Logs move to kind `log`
  now. Decisions from Kieran are under "Decided".
- 2026-11-07: Phase 1 done. `gnoblin-store` ships the consumer API for the
  in-process `kv` provider: typed query layer, WAL, quotas, quarantine, watch
  and batch. Tests cover the failure modes listed under "Failure modes".
- 2026-11-07: Phase 3 code landed. The `log` kind is in `gnoblin-store`, and
  the log store opener moved there from `gnoblin-log`, which no longer depends
  on the store. A one-off import copies an older `logs.sqlite3` into
  `log.sqlite3`, once, and renames the old file. The phase 3 verify items (row
  counts after migration, retention, and the single WAL reader) are not all run
  yet.
- 2026-11-07: Phase 4 landed. The `kv` verbs are served on `store`, and the `log`
  verbs (`put`, `get`) on `log`. Both serve the connection's owner's store and
  refuse with `no-store-owner` otherwise. The file name is in "Decided".
