# Store

Status: **Proposed 2026-11-06** (D30 in `rust-compositor.md`). The model, the
capability split and the provider rules are proposed. The choices under "Needs
Kieran" are open. Where a fact was checked in the tree, the file and line are
given.

## Summary

A store is the one abstraction. The keyring (D27), the log (D23), extension
state and any later storage are kinds of store. Every store has the same common
core. Behaviour comes from capabilities, and a kind is a named bundle of
capabilities plus policy.

A provider registers a store the way a layer registers (D28). The compositor
owns names, attribution, grants and lifecycle. Consumers use one API, in Lua,
IPC, `gnoblinctl` and wasm.

## Model

### Common core

Every store has these parts. No kind may leave one out.

| Part          | Contents                                                     |
| ------------- | ------------------------------------------------------------ |
| Identity      | `name`, `kind`, and `provider`, which the compositor sets    |
| State         | One lifecycle state, from the table below                    |
| Usage         | Bytes, record count, quota and retention for the kind        |
| Capabilities  | The set reported at open, from the capability list           |
| Change signal | One `changed` event per commit, with revisions               |
| Export policy | What `export` and `import` may do, set by the kind           |
| Access        | Grants per consumer (D22 powers), held in host configuration |

### Lifecycle

The states are `opening`, `ready`, `locked`, `unavailable`, `quarantined` and
`closing`. Only the registry moves a store between them. A provider reports a
cause, and the registry decides the transition.

| From          | To            | When                                                  |
| ------------- | ------------- | ----------------------------------------------------- |
| `opening`     | `ready`       | Backend opened and its integrity check passed         |
| `opening`     | `unavailable` | Backend cannot open, such as a missing directory      |
| `opening`     | `quarantined` | Integrity check failed; the file is moved aside       |
| `ready`       | `locked`      | Lock requested, or screen locked (`Lock` stores only) |
| `locked`      | `ready`       | Unlock accepted with a valid credential               |
| `ready`       | `unavailable` | Backend lost: disk gone, or provider disconnected     |
| `unavailable` | `opening`     | Retry on the next open                                |
| `quarantined` | `closing`     | Purge, or the user restores a copy                    |
| any           | `closing`     | Provider removed, purged, or compositor shutdown      |

A store that opens but cannot write is `ready` with `writable = false`. That is a
flag, not a seventh state. It is an option under "Needs Kieran".

### Capabilities

A capability is a trait. A store reports its set at open, and the set may
shrink later, never grow. A consumer may call a method only when the capability
is in the set and the grant allows it. Lua omits the method. IPC returns
`capability-not-supported`.

| Capability | Operations                                                           | Kinds                 |
| ---------- | -------------------------------------------------------------------- | --------------------- |
| `Items`    | `get`, `put` with optional revision, `delete` with optional revision | `kv`, `secret`        |
| `Search`   | Typed query: key prefix, attribute equality, time range              | `kv`, `secret`, `log` |
| `Append`   | Append a record; read from a sequence; retention by age or size      | `log`                 |
| `Lock`     | `lock`; `unlock` with a credential or a D28 prompt layer             | `secret`              |
| `Watch`    | Change signal per commit, in commit order                            | `kv`, `secret`        |
| `Transact` | Atomic batch of `Items` operations                                   | `kv`, `secret`        |

`Search` is one query language for every kind. It has no raw SQL, and no
consumer gets a SQL handle.

### Secret gating

Generic frontends must not bypass the Secret Service authorization path (D27).
The frontends are Lua, IPC, `gnoblinctl` and WASM. For a `secret` store they
expose metadata only: names, counts, usage, state, lock state and, if Needs
Kieran 3 allows it, attribute keys. They never return a secret value.

A secret value comes only from the secret store's own path. That path needs the
keyring unlocked, and it applies the consent and prompt policy of D27: confirm
first retrieval per application, wherever that policy applies.

A `secret` store is never exported in plaintext. The only export form is an
encrypted backup of the whole store file. `gnoblinctl store get` on a secret
item returns `secret-value-refused`, which names the keyring tooling
(`secret-tool`, `design/keyring.md`).

### One item model, and where it breaks

An item is a key, a set of attributes, a value and a revision. Each attribute
is a name and a `Value`, and so is the item value (`gnoblin-model/src/value.rs:19`).

Key-value and the Secret Service both fit this model. A `kv` item has a key and
no attributes. A secret item has attributes, such as `app` or `label`, and a
secret value in `Bytes`.

The Secret Service `label` and `content-type` become reserved attribute names.
Both cases are "find by identity, then read or write a value", so one model
serves both.

The model breaks in three places:

- **Logs.** A log record has no key and is never updated. It is a sequence, so
  a log uses `Append`, not `Items`.
- **Secret Service collections.** A collection is not an item. It is a store of
  kind `secret`. The `default` alias is a name that points at one store.
- **Nested values.** `Value` has no map (`value.rs:19`). An attribute or value
  that needs a map is refused with `value-not-storable`.

### Kinds

A kind is a name, a capability set and a policy. The kind sets the policy, and
a provider may set options only inside the bounds the kind gives.

| Kind     | Capabilities                           | Policy                                                                                         |
| -------- | -------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `kv`     | `Items`, `Search`, `Transact`, `Watch` | Plain values. Byte quota per store. Optional TTL.                                              |
| `log`    | `Append`, `Search`                     | Plain records. Retention by age and size. Written only through the `gnoblin-log` facade (D23). |
| `secret` | `Items`, `Search`, `Lock`, `Watch`     | Encrypted (D27). Never exported in plaintext. Locked on screen lock. Never logged.             |

Kinds are a closed list in `gnoblin-store` for v1. Whether an extension may
define a kind is open (see "Needs Kieran").

## Provider side

### Who provides

A provider is a Gnoblin crate, the keyring service process, an extension with
the `store` power, or a wasm component later. An out-of-process provider speaks
the same provider interface over IPC. The keyring is one such provider.

### Registration rules

These rules copy the shape of the layer registry (`design/layers.md`,
"Registration").

- **Unique names.** A store name belongs to its first registrant. Built-in
  names are reserved for their providers. A refused name gets
  `namespace-invalid` or `namespace-taken`.
- **Attribution.** The compositor sets `provider`. A caller never sets it.
- **Kind by name.** The provider names a kind from the closed list.
- **Options within bounds.** The provider sets quota, TTL or retention, but only
  inside the kind's bounds.
- **Limits.** Each provider has a maximum store count and a maximum total of
  bytes. The numbers are proposed.

### Platform pattern: registry, provider, grants, lifecycle

Layers (D28) and stores (D30) use one shape. Any later registry should use it
too. This section names the shape, so a new registry does not invent its own.

| Part         | Layers (D28)                          | Stores (D30)                                    |
| ------------ | ------------------------------------- | ----------------------------------------------- |
| Registry     | `LayerRegistry` in `gnoblin-layers`   | `StoreRegistry` in `gnoblin-store`              |
| Providers    | Gnoblin, extensions, Wayland surfaces | Gnoblin crates, keyring, extensions             |
| Names        | Owned by the first registrant         | Owned by the first registrant                   |
| Closed set   | Bands, for ordering                   | Kinds, for policy                               |
| Capabilities | Scope, input, `secure`, content kind  | `Items`, `Search`, `Append`, `Lock`, and others |
| Lifecycle    | Live until target or provider ends    | The six states above                            |
| Invariants   | Compositor-enforced                   | Compositor-enforced                             |

A closed set fits an enum, so bands and kinds are enums. Capabilities are not
closed in that way. A kind is a bundle of them, and extensions may need new ones
later. So capabilities are trait objects, discovered at runtime. D28 and D30
point at each other, and each keeps its own invariants.

### Rust sketch

Signatures only. The bodies are in the phase 2 commit.

```rust
/// Common core. Every store implements this. Object-safe.
pub trait Store: Send + Sync {
    fn name(&self) -> &StoreName;
    fn kind(&self) -> KindId;
    fn state(&self) -> LifecycleState;
    fn usage(&self) -> Usage;
    fn capabilities(&self) -> CapabilitySet;

    // One accessor per capability. None when the capability is absent.
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
    pub attributes: BTreeMap<AttrName, Value>,
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

**Dyn or enum.** The kinds are open, because extensions may supply them later.
The store also has no hot path: calls cross IPC or a bounded write. Capability
discovery needs trait objects. An enum would close the set and force a
compositor change for each new capability. So the store uses `dyn Store`, not
an enum.

## Consumers

One API has four front ends and two compatibility adapters. Each shows the same
properties: `name`, `kind`, `provider`, `state`, `capabilities`, `usage`.

- **Lua: `gnoblin.store.open(name)`.** Returns a handle. The handle carries
  only the methods its capabilities allow. Async calls return `Operation<Result>`,
  as in `design/lua-api.md:204`. Opening another owner's store needs a grant.
  For `secret`, the handle exposes metadata only; see Secret gating.
- **IPC: `store` object.** Properties as above. Methods are gated by
  capability and grant. The signal `changed` carries keys and revisions. For
  `secret`, metadata only; see Secret gating.
- **`gnoblinctl store`.** Subcommands `list`, `usage`, `get`, `search`, `export`
  and `purge`. The output is the same for every kind, except that `secret` is
  metadata only; see Secret gating. Writes need the compositor.
- **WASM.** A host import in the extension world (D22, `gnoblin:ext@0.2.0`,
  `rust-compositor.md:2700`). The same functions, with values as WIT variants.
  A `secret` value is not exposed here either; see Secret gating.
- **Secret Service adapter.** The D-Bus Secret Service API (keyring, D27) is an
  adapter over a `secret` store. The keyring service runs it.
- **Secret portal adapter.** The Secret portal serves per-app secrets from the
  same `secret` store. Per-app consent is unchanged (D27).

The keyring stays a separate service process with its own SQLCipher file (D27).
The change here is only that it speaks the provider interface. So
`gnoblinctl store list` shows it, and the Secret Service API keeps its own
D-Bus surface.

## Backends

A backend is an implementation detail of a provider. The contract is the store
and its name, not the file.

| Kind     | Default backend                                                                         |
| -------- | --------------------------------------------------------------------------------------- |
| `kv`     | SQLite, one file per store: `$XDG_DATA_HOME/gnoblin/store/<name>.sqlite3` (proposed)    |
| `log`    | The existing `$XDG_STATE_HOME/gnoblin/logs.sqlite3`, `gnoblin-log/src/store.rs:103-113` |
| `secret` | SQLCipher file owned by the keyring service (D27, `design/keyring.md:13`)               |

One file per store is the default. It is not a v1 contract. Whether files group
by kind is a backend choice (see "Needs Kieran").

## Rules

Each rule is tagged. An **invariant** is compositor policy, and no provider or
extension may change it. An **option** is a value a provider sets within its
grant, and its choice is open.

| Rule                                                  | Tag       |
| ----------------------------------------------------- | --------- |
| Extensions never see a file path or open a file       | invariant |
| One writer per backend: its owning process            | invariant |
| Names are owned by the first registrant               | invariant |
| The compositor sets `provider`                        | invariant |
| Lifecycle moves only through the registry             | invariant |
| No raw SQL reaches any consumer                       | invariant |
| `secret` never returns plaintext while locked         | invariant |
| Generic frontends never return a `secret` value       | invariant |
| `secret` export is encrypted whole-file only          | invariant |
| Secrets never enter a `kv` or `log` store (D27)       | invariant |
| Grants and capability sets live in host configuration | invariant |
| Uninstalled data is kept until `purge`                | invariant |
| Capability set of each kind, within its bounds        | option    |
| Quota, TTL and retention values                       | option    |
| `writable` flag for a store that cannot write         | option    |
| Backend file layout                                   | option    |

## Values and keys

A value is a `gnoblin_model::Value`, the same enum the object model uses for
properties (`value.rs:19`). The store accepts a subset: `Bool`, `Int`, `Uint`,
`Double` (finite only), `Text`, `Bytes`, and a `List` of accepted values. It
refuses `Object(ObjectId)`, because object ids are runtime identities
[INFERENCE: they do not survive a restart]. Refusals return `value-not-storable`.

Keys are UTF-8, 1 to 256 bytes, with no NUL and no leading or trailing `/`. A
prefix listing matches whole segments only, so `window/1` does not match
`window/10`. `kv` and `log` encode values canonically. The encoding is owned by
`gnoblin-store` and carries a version [INFERENCE: checked against the IPC
payload encoding in phase 3].

## Names and grants

A store name is 1 to 64 bytes: a lowercase ASCII letter or digit, then
lowercase letters, digits, `.`, `-` or `_`. `..` is refused, and `/`, NUL and
upper case are refused. An extension id may not start with `gnoblin-` or
`gnoblin.`.

- **`store`** (D22) reads and writes the package's own store. A package gets it
  only when it declares `[powers.store]` and host configuration grants it.
- **`store.read:<name>` and `store.write:<name>`** name one other store each.
  There are no globs. A cross-store grant is never implicit.
- **Crate stores** are readable by an extension only under a
  `store.read:gnoblin-<crate>` grant. An extension can never write one.
- **Grants live in host configuration**, not in the store, so the user can read
  them in the config file.
- **`gnoblinctl store usage`** lists each store with its owner, kind, bytes,
  quota, state and the grants that reach it.
- **Uninstall.** A removed package's stores are marked `orphaned` in `usage`.
  That is a flag, not a lifecycle state. The data is kept and listed there. A
  reinstall with the same id attaches to it again.
  `gnoblinctl store purge <name> --yes` removes the data. Crate stores are never
  orphaned.

## Today

Every persisted location under `crates` on origin is listed below. The last
column says whether it moves to a store.

| What                    | Where                                                                                    | Decision                                        |
| ----------------------- | ---------------------------------------------------------------------------------------- | ----------------------------------------------- |
| Log records             | `$XDG_STATE_HOME/gnoblin/logs.sqlite3`, `gnoblin-log/src/store.rs:103-113`               | Stays until adopted as `log` (Needs Kieran 5).  |
| Session recovery        | `$XDG_STATE_HOME/gnoblin/recovery`, `gnoblin-session/src/supervisor.rs:136`              | Stays. Read before any runtime starts.          |
| Last accepted config    | `$XDG_STATE_HOME/gnoblin/last-good-config.json`, `gnoblin-config/src/last_good.rs:45-57` | Stays. Fallback must not depend on a store.     |
| Spent activation tokens | `gnoblin-authority/src/authority.rs:288-292`, in memory                                  | Stays in memory. Never persisted.               |
| AppChooser "remember"   | `gnoblin-portal-core/src/model/interface.rs:123`; no write code exists                   | Stays a freedesktop file (`mimeapps.list`).     |
| Layout `ctx.state`      | `gnoblin-lua/src/layout/host.rs:94`, in memory per worker                                | Stays in memory.                                |
| Pre-snap window frame   | `design/lua-api.md:515`; no disk write found                                             | Stays in memory [INFERENCE].                    |
| Session dconf keyfile   | `compat/gnoblin-compat-gtk-dconf-settings/src/lib.rs:249`                                | Stays. Written into the caller's profile.       |
| Extension state         | None. The wasm host has no filesystem (`gnoblin-wasm/src/host.rs:457`)                   | New. A `kv` store is its first home.            |
| Secrets                 | None in the tree. Keyring is planned (`design/keyring.md:10`)                            | A `secret` store (D27). Never in `kv` or `log`. |

Nothing persisted today has to move, so the first phase needs no data migration.
The log keeps its own file until it is adopted (D23).

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
- **Locked.** A `secret` read or write while `locked` fails with `store-locked`.
  The file stays closed.
- **Secret value asked of a generic frontend.** `get` on a `secret` item returns
  `secret-value-refused` in every frontend. No value is read from the file.
- **Plaintext export asked for.** `export` of a `secret` store returns
  `secret-export-refused`, and names the encrypted whole-file backup.
- **Migration fails.** The step's transaction rolls back and the version stays.
  The store opens with `writable = false` if that option is chosen, and the
  failure is logged.
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
  store is refused here; see Secret gating. `import` runs in one transaction and
  refuses an owner mismatch unless `--as` names another owner.

## Interactions

- **Config and store.** Configuration is declarative and hand-edited. Stores
  hold runtime state.
- **Never moves to a store:** hand-edited settings, grants and capability sets,
  the recovery state, the last accepted configuration, and capture, input and
  secret consent.
- **Layers (D28) and window rules.** Window rules are configuration. A remembered
  placement, if wanted, is a key in the owning crate's `kv` store, off by default.
- **Portals.** AppChooser's "remember" stays a freedesktop file. The store may
  hold transient portal state.
- **Keyring (D27).** The keyring is a `secret` provider. Its decisions are in
  `design/keyring.md` and are unchanged.
- **`gnoblin.configure`.** Unaffected. `gnoblin.settings` stays a read-only
  snapshot (`design/lua-api.md:188`).

## Phased implementation

Each phase is one reviewable commit. Its tests are named here but not run.

1. **Decision.** This file, D30, the D28 pointer and the D27 provider note.
   Verify: prettier and the review hook. No code.
2. **`gnoblin-store` core.** `Store`, the lifecycle, `StoreRegistry`, the `Items`
   capability and an in-process `kv` provider. File layout, lock, WAL and
   quarantine. Verify: one unit test per failure mode on temp directories, and
   the foundation rule in `xtask/src/deps.rs` (`FOUNDATION`, line 34).
3. **IPC `store` object.** Core properties and `Items` methods, gated by
   capability and grant. Verify: a round trip over IPC. Regenerate
   `gnoblin-proto`, `gnoblin-model` and `gnoblin-client`.
4. **`gnoblinctl store`.** `list`, `usage` and `get` first, then `put` and
   `delete` through IPC. Verify: golden output from `gnoblin-cli-style`.
5. **Lua `gnoblin.store.open`.** `kv` only, behind power `store`. Verify: a
   missing grant is refused, and a missing capability leaves the method nil.
6. **`Search`, `Transact` and `Watch` for `kv`.** Verify: commit-order delivery,
   batch coalescing per key, and no raw SQL path.
7. **`log` adopter.** `Append` and `Search`, once Needs Kieran 5 and 4 are
   decided. Verify: retention by age and size, and writes only through the
   facade.
8. **`secret` kind and the keyring provider.** `Lock`, after the D27 key source
   is chosen. Verify: the Secret Service and portal adapters, `gnoblinctl store
list` shows the keyring store, and every generic frontend refuses a `secret`
   value.
9. **Export, import, purge and orphaning.** Verify: an export and import round
   trip, and `purge` refuses without `--yes`.
10. **WASM host import.** After the D22 wasm gate. Verify: a component test.
11. **Fault injection.** Quarantine, disk full, lock, migration failure. Verify:
    one test per failure mode above. Also: a plaintext `secret` export is
    refused, and a `secret` value read through each generic frontend (IPC, Lua,
    `gnoblinctl`, WASM) returns `secret-value-refused`.

## Needs Kieran

Each item is tagged. Each lists the real choice, with pros and cons. None is
decided here.

1. **Which capabilities ship first.** Option.
    - Proposed: `kv` gets `Items`, `Search`, `Transact` and `Watch` in phases 2
      to 6. `log` adopts `Append` and `Search` later. `secret` adopts `Lock`
      with the keyring.
    - Pros: a small first step, and each kind arrives with its own tests.
    - Cons: an owner that needs `Watch` or `Transact` waits longer.
2. **May an extension define a new kind.** Option.
    - Proposed: no. Kinds are compiled into `gnoblin-store`, and a new kind is a
      Gnoblin change.
    - Pros: each kind has one reviewed policy.
    - Cons: every new kind needs a release, which an extension author cannot do.
3. **Are `secret` attribute keys readable while locked.** Option. Values never are.
    - Pros of yes: an app can find its item without unlocking.
    - Cons of yes: a locked store reveals which apps hold secrets and under which keys.
    - Pros of no: nothing about the store leaks while locked.
    - Cons of no: every lookup needs an unlock first.
    - Item names and counts while locked are a separate question. This item does not settle them.
4. **A `writable` flag, or a seventh lifecycle state.** Option.
    - Pros of the flag: the state enum stays at six.
    - Cons of the flag: consumers must read two fields to know if a write works.
5. **Logs through the store API (`log` kind), or keep `logs.sqlite3`.** Option.
    - Pros of adopting: one query surface for tools.
    - Cons of adopting: a second reader on a high-write file (D23). A read view
      must not hold the WAL open for long.
6. **Default quota, TTL and retention.** Option.
    - Example: 16 MiB per `kv` store, 64 KiB per value, and 30 days of log.
    - Pros of small numbers: bounded disk use.
    - Cons of small numbers: a real cache fails sooner, and quietly in the field.
7. **Query operators.** Option.
    - Proposed: key prefix, attribute equality and time range only. Raw SQL is
      refused by invariant.
    - Pros: indexes stay simple and the surface stays small.
    - Cons: owners will ask for ordering and ranges. Each addition moves the
      query language towards SQL.
8. **Backend layout: one file per store, or one file per kind.** Option. This
   is a backend detail, not a contract.
    - Pros of one file per store: a failure stays local to one store.
    - Cons of one file per store: more files to list and back up.
9. **The `secret` unlock credential.** Option. Depends on the key source that
   `design/keyring.md` leaves open.
    - Pros of a D28 prompt layer: the user unlocks where they already are.
    - Cons of a prompt layer: a prompt is a second path for a credential.

## Change log

- 2026-11-06: rewritten as a store abstraction. Keyring, log and extension state
  are kinds. The v1 key-value design is now the `kv` kind. The earlier open
  items are folded into items 1 to 9 above. Generic frontends are metadata
  only for `secret`; see Secret gating.
