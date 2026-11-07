# Storage and secrets: one design

Status: **Proposal 2026-11-07. Replaces the split between `design/store.md` and `design/keyring.md` once Kieran
picks the options under "Needs Kieran".** Nothing in this file is built yet. Kieran's verdict on the current
work: not planned as one thing, and too much of it. This file starts from what Gnoblin needs, not from what was
built.

## What Gnoblin actually needs

| Need                              | Who asks                              | Today                            |
| --------------------------------- | ------------------------------------- | -------------------------------- |
| Keep logs and read them back      | `gnoblinctl log`, the overlay         | Built (`gnoblin-store` log kind) |
| Small persistent state            | Compositor, runtime, later extensions | Built (`gnoblin-store` kv kind)  |
| Passwords for apps (libsecret)    | Browsers, chat and mail clients       | Not usable (see problems)        |
| Per-app secret for sandboxed apps | The Secret portal                     | Partly built                     |
| Ask the user to unlock            | The keyring                           | Not built                        |

Everything else in the two design files (watch, search operators, batch, export, import, purge, cross-store
grants, `[powers.store]`) serves no caller today.

## What is wrong with the current shape

Facts, with where to check them.

- **Two storage engines.** `design/store.md` says a secret store is a kind of store. The code has
  `gnoblin-store` (4149 lines) and a separate `gnoblin-keyring-store` (738 lines) with its own schema.
- **Five crates for one feature.** Keyring is `gnoblin-keyring-store`, `gnoblin-service-keyring` (1583 lines),
  `gnoblin-keyring-handoff`, `pam-gnoblin-keyring` and a relay interface. Each has its own xtask rules.
- **The compositor carries secrets.** The portal path sends a request to the compositor, which holds a pending
  envelope, asks the services role over a new internal slot (`secret.derive`), and relays the bytes back. That
  made zeroising wire buffers and broker tombstones a project of its own. The compositor needs to know nothing
  about secrets.
- **The Secret Service design is read only.** `design/keyring.md` lines 162 to 165 refuse create, update and
  delete over the bus, but the Secret Service specification defines `CreateItem`, `SetSecret` and `Delete`, and
  the acceptance test on line 243 expects `secret-tool store` to work. The design contradicts its own test.
- **SQLCipher changes the whole workspace.** It pins SQLite 3.53.4 instead of 3.54.1 for every crate, and adds
  a runtime dependency on libcrypto, for one feature.
- **Open questions block the rest.** Locked search, cross-store grants, `[powers.store]`, and Lua store access
  (blocked on per-extension Lua environments) are all unresolved, so `gnoblinctl store put` ships refused.

## Principles for the redesign

1. One storage engine. Logs, state and secrets differ by kind, not by crate.
2. A foreign protocol is an adapter. The Secret Service is a compat adapter like the other D-Bus ones.
3. Secrets never pass through the compositor. It draws consent and nothing else.
4. Build what a caller needs. Nothing without a consumer.
5. No new heavy native dependency for one feature.

## Proposal A: one engine, three kinds, one adapter

### Engine

`gnoblin-store` stays the only storage crate. One SQLite file per store, as decided. Three kinds:

| Kind    | What it holds                          | Verbs kept now                         |
| ------- | -------------------------------------- | -------------------------------------- |
| `kv`    | Values by key                          | `open`, `put`, `get`, `delete`, `list` |
| `log`   | Appended records, retention            | `put` (append), `get`, `list`          |
| `vault` | Sealed items with attributes (secrets) | Item verbs, plus `lock` and `unlock`   |

`watch`, `search` operators, `batch`, export, import and purge move to "when a caller exists". The verbs we keep
are the ones the log reader, the portal remember-choice store and the keyring use.

### Vault without SQLCipher

A `vault` is a plain SQLite store whose values are sealed with an AEAD cipher (XChaCha20-Poly1305 from pure Rust
crates) under a key held in memory while unlocked. Attribute values are stored as keyed hashes under a second key
derived from the same key, so a search needs the vault unlocked and the file reveals no attribute text.

- No SQLCipher, no libcrypto, no SQLite downgrade. `gnoblin-store` stays on the current SQLite.
- Locked search returns nothing. That is the behaviour already chosen; it falls out of the design.
- The key comes from a `KeySource`. Today's choices are a prompt and, later, the login password from PAM.
- Wrong key is detected by a sealed canary item, not by the file failing to open.

[INFERENCE] The cipher crates are not yet checked against the workspace's dependency policy.

### The adapter

`gnoblin-compat-dbus-freedesktop-secrets` is one new crate in the compat set. It serves
`org.freedesktop.secrets` in the services role and talks to the vault in process. It supports the full
Secret Service: create, update, delete, search, lock and unlock, so `secret-tool` works. Prompts for unlock use
the stock built-in dialog. The bus coexistence rule and `keyring.provider` stay as decided.

### The portal

The relay shows consent through the compositor, as every portal does today. After consent it asks the
services role directly on the session bus. The method is private, accepts only the relay's bus name (checked by
sender credentials), and returns the per-app secret derived with HKDF from the master item. The compositor never
sees a secret. The `secret.derive` slot, the retrieve envelope, and the tombstone work are deleted.

### Unlock

First cut: one prompt per login, drawn by the built-in dialog. The vault unlocks until logout. No PAM module, no
hand-off socket.

Later, behind the same `KeySource` trait: a PAM module that hands over the login password. It adds a native
module, a socket and a password-change hook, so it is a separate step with its own decision.

### What happens to the built code

| Piece                                            | Action                                                   |
| ------------------------------------------------ | -------------------------------------------------------- |
| `gnoblin-store` kv and log                       | Keep. Unused verbs stay off the IPC and CLI surface.     |
| `gnoblin-keyring-store`                          | Delete. Its tests move to the `vault` kind.              |
| `gnoblin-service-keyring`                        | Delete. Logic moves into the compat adapter.             |
| `gnoblin-keyring-handoff`, `pam-gnoblin-keyring` | Park until the PAM step. Not in the first cut.           |
| `secret.derive` slot and compositor routing      | Delete. Portal uses the private bus method.              |
| Relay `iface/secret.rs`                          | Keep. It calls the bus method instead of the compositor. |
| Zeroising work in client, proto, loopback        | Keep. It is correct, only no longer on the secret path.  |
| SQLCipher workspace feature                      | Revert to plain `bundled`.                               |
| `gnoblinctl store list`, `usage`, `get`          | Keep. `get` refuses vault values.                        |

### Key custody and isolation

Who holds what, in Proposal A:

| Item                         | Held by                       | Where it lives                               |
| ---------------------------- | ----------------------------- | -------------------------------------------- |
| Vault key and index key      | Services role process only    | Memory, while unlocked. Never on disk        |
| Vault files                  | On disk, readable by the user | Their own directory, mode 0600 (same UID)    |
| `kv` and `log` files         | Compositor, runtime, services | The store directory, per owner               |
| Unlock input (prompt or PAM) | Services role                 | Never in argv, environment or the compositor |

The engine crate is shared code. It is not a shared process. The compositor links `gnoblin-store` for `kv` and
`log`, and it should never open a vault. A rule in `xtask/src/deps.rs` can stop any role but services from
calling the vault constructors. That is a build-time rule, not a runtime boundary.

**What separates them today: nothing at runtime.** Mode 0600 does not. The compositor, the runtime and the
services role all run as the same user, so any of them, or any other process of that user, can open the vault
file and can read the memory of the others unless the kernel stops it. The threat model has to include a same-user
attacker.

What the proposal relies on, none of it built or tested yet:

1. The file is sealed. Without the key it is useless, and the key exists only in the services process's memory
   while unlocked. This holds against file theft and backup leaks, not against code running as the user.
2. **Required controls, to implement and validate:** the services process sets `PR_SET_DUMPABLE` to 0, the
   target distributions restrict `ptrace` between peers (Yama `ptrace_scope`), and the key never appears in
   argv, environment or logs. Each needs a test on a target distribution before the design may say it holds.

Even with those controls, a same-user attacker with enough privilege can defeat them. A real boundary needs a
separate UID for the keyring service or a credentialed broker, which is a different design. This proposal does
not claim one and does not claim to match another keyring's protection. The process isolation is unresolved until
the controls above are validated.

### Shared API with separate sealed storage (variant A2)

The same consumer API, but the vault lives in its own crate with its own file format, and `gnoblin-store` only
routes the verbs to it.

| Question                        | A: vault is a kind in the one engine         | A2: separate sealed backend           |
| ------------------------------- | -------------------------------------------- | ------------------------------------- |
| Crates                          | One                                          | Two, plus the routing seam            |
| Compositor links vault code     | Yes, unless the kind is feature-gated        | No                                    |
| Key custody boundary            | Process and directory                        | Process, directory and crate          |
| Code shared with `kv` and `log` | File, WAL, quota and lifecycle code          | Little; the sealed format is its own  |
| Cost                            | Feature-gate the kind; deps rule on the kind | A trait seam and a second file format |

A is smaller. A2 gives a harder wall around key material at the price of a second engine, which is the split this
file argues against. If you want the compositor build to contain no vault code at all, choose A2 or gate the
`vault` kind behind a Cargo feature that only services enables.

## Alternatives

**B. No own keyring.** Keep `kv` and `log`. Ship no Secret Service. Document gnome-keyring or KeePassXC through
`keyring.provider`, and serve the Secret portal from the other provider when it can.

**C. Finish the current plan.** Keep the five crates, the slot and SQLCipher. The landed design refuses create,
update and delete on the Secret Service (`design/keyring.md` lines 162 to 165). The Secret Service specification
defines `CreateItem`, `SetSecret` and `Delete`, and the keyring acceptance test expects `secret-tool store`, so
they would have to be added. Then finish the prompt UI and decide the open items.

| Option | Pros                                                         | Cons                                                                  |
| ------ | ------------------------------------------------------------ | --------------------------------------------------------------------- |
| A      | One engine; fewer crates; no SQLCipher; compositor untouched | New cipher code to write and review; a sealed-value scheme of our own |
| B      | Least code; no secret handling to get wrong                  | A fresh Gnoblin desktop cannot save a browser password                |
| C      | Least rework of landed code                                  | Keeps every problem above; most dependencies and routing              |

Risks of A: a home-made sealing scheme needs an independent review before release, and the Secret Service has
to be complete or libsecret clients fail. Risks of B: it changes decision D27. Risks of C: it is the plan that
Kieran called unplanned.

## Needs Kieran

1. **Own keyring or delegate.** A and C build one. B does not. This reverses or keeps D27.
2. **Vault format.** Sealed values in plain SQLite (A), or SQLCipher for the whole file (C).
   A2 (separate sealed backend) is a variant of this choice: see "Shared API with separate sealed storage".
3. **Unlock.** Prompt once per login first and PAM later (A), or PAM from the start (C).
4. **Secret Service writes.** Full create, update and delete (needed for libsecret), or read only.
5. **Store verbs.** Keep only `open`, `put`, `get`, `delete`, `list` until a caller needs more, or keep the
   whole verb set in the design.

## Order of work if A is chosen

1. Revert the SQLCipher workspace feature and the keyring store crate dependency.
2. Add the `vault` kind to `gnoblin-store` with sealing, canary and lock state. Tests first: wrong key, tampered
   value, truncated file, concurrent open, locked search returns nothing, nothing readable from the file.
3. Add the compat adapter with full Secret Service and the stock unlock prompt.
4. Change the portal relay to the private bus method and delete the compositor secret path.
5. Delete the retired crates and rewrite `design/store.md` and `design/keyring.md` as short pages that point here.
6. PAM hand-off as its own proposal.
