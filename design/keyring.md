# Keyring service

Status: **Decided 2026-11-06** (D27 in `rust-compositor.md`). The key source
is decided: the login password, handed over by a PAM module, with one unlock
prompt when there is no password (see "Key source"). Two choices remain
under "Needs Kieran".

## Summary

Gnoblin runs its own keyring service, `gnoblin-service-keyring`, in the
services role (D4). It implements the Secret Service API on the session bus,
so existing clients (libsecret, `secret-tool`, browsers, mail clients) use it
without change. Its items live in one SQLCipher-encrypted file. The Secret
portal serves per-app secrets from the same store.

Gnoblin does not depend on gnome-keyring, KWallet or KeePassXC. If another
provider already owns the Secret Service name, Gnoblin's service does not take
it (see "Coexistence").

## Decision

- Gnoblin owns the store. No secret is kept by another provider on Gnoblin's
  behalf.
- The store is a SQLCipher database. The `rusqlite` crate (0.41.1) has the
  `bundled-sqlcipher` feature. The workspace `Cargo.toml` (line 297) enables
  only `bundled`, so enabling SQLCipher is part of phase 1.
- Compatibility target: the Secret Service specification, version 0.2 draft
  (dated 2026-10-19). The service implements the `Service`, `Collection`,
  `Item`, `Session` and `Prompt` interfaces.
- Gnoblin's prompt UI draws the `Prompt` objects, through the `prompt.secret`
  slot.
- The key source is the login password, as in "Key source" below.

## Key source

**Decided 2026-11-06 by Kieran.** The store is unlocked by the login password,
handed over by a PAM module. This is what GNOME and KDE do. TPM sealing is not
the default. It is an optional later add-on, with the facts in the appendix.

How it works:

1. A PAM module, `pam_gnoblin_keyring`, runs in the login stack of each
   greeter. In the auth stage it reads `PAM_AUTHTOK` and holds the password in
   a zeroizing buffer. In the session stage, once `XDG_RUNTIME_DIR` exists, it
   sends the password to the keyring service over the unix socket
   `$XDG_RUNTIME_DIR/gnoblin/keyring-unlock.sock`. The service derives the
   SQLCipher key from it and unlocks the store. The service is a long-lived
   process, not a child of the PAM caller, so stdin cannot carry the password.
   The password is passed to SQLCipher as the database key. SQLCipher stretches
   it at `kdf_iter` 256000, set explicitly in `gnoblin-keyring-store` (not left
   to the library default). [INFERENCE] The hash is PBKDF2-HMAC-SHA512, from the
   SQLCipher 4 defaults; confirm on Agent37 before relying on it.
2. **No password (fingerprint login, autologin).** The PAM module gets no
   password, so it cannot unlock. The keyring then offers **one** `prompt.secret`
   request per login session. A shell claims that slot and draws the prompt; the
   keyring never draws UI itself. If no shell claims the slot, the request is
   refused at once and the store stays locked. The store stays unlocked until
   logout, so the user is never asked twice in one session.
3. **Password change.** A hook in the PAM password stage re-keys the store. If
   the hook is missing, the store stays locked after a change. [INFERENCE] This
   follows `pam_gnome_keyring`; it is not yet tested on greetd or
   cosmic-greeter.
4. **Screen lock does not lock the store by default.** Locking it on every
   screen lock would ask the user again. The optional leaf
   `keyring.lock-on-screen-lock` (default `false`, **PROPOSED**) would lock it.
   The name follows the schema's kebab-case section and field pattern.
   `crates/gnoblin-config/schema/config.toml` has no
   `keyring` section yet; add one when the feature is built.

The service holds the derived key in process memory for the session. A
restart of the service asks for the key again. That cost is accepted for now.

### Why not the other options

| Option                            | Why it is not the default                                                                                      |
| --------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| Second passphrase at every login  | Asks the user twice: at login and at the store. The no-password prompt is asked once per session.              |
| `systemd-creds` / TPM2 sealed key | Needs the same machine and TPM, and root-only host key data. Not used by Linux desktop keyrings. See appendix. |
| Kernel session keyring cache      | Lost at logout. Not a root of trust. The service's memory already holds the key for the session.               |
| Coexistence (bus rule, not a key) | Not a key source. See "Coexistence".                                                                           |

The table is the rationale. The chosen design is in "How it works" above.

## Prior art: how other services handle a locked store

The facts below were read from upstream source, unless marked otherwise. The
design follows the results, so a client that works with GNOME, KWallet and
KeePassXC also works with Gnoblin.

- **GNOME (gnome-keyring).** `SearchItems` returns locked items with a
  `locked` out-parameter (`daemon/secret-service/gkd-secret-service.c`
  452-458; `gkd-secret-objects.c` 1277, 1346-1349, 619-650). The label and the
  fields are readable while locked (`gkm-secret-object.c` 138-139;
  `gkm-secret-item.c` 232-237). The secret value is lock-checked
  (`gkm-secret-item.c` 217-219). Attributes are stored hashed on disk
  (`gkm-secret-fields.c` 445-451).
- **KWallet.** `SearchItems` works while locked. It reads a plain JSON file,
  `<wallet>_attributes.json`, beside the encrypted wallet
  (`kwalletfreedesktopattributes.cpp` 19-20, 31-50, 62-70; `collection.cpp`
  240-252). An item reports locked when its collection is locked
  (`item.cpp` 81-84). `GetSecret` needs the wallet open (`item.cpp` 116-135).
  `CreateItem` is refused while locked (`collection.cpp` 107-111). The
  attributes are plain text on disk.
- **KeePassXC.** `SearchItems` covers only unlocked collections
  (`kp-Service.cpp` 266-327). `Collection::searchItems` returns nothing while
  the database is locked (`kp-Collection.cpp` 213-225). Attributes and the
  label report `IS_LOCKED` (`kp-Item.cpp` 95-100, 175-187).
- **PAM, GNOME.** `gkr-pam-module.c` reads `PAM_AUTHTOK` in the auth stage
  (lines 851-890). With no password it returns `PAM_SUCCESS` and does not
  unlock (lines 879-886). So a login without a password leaves the store
  locked.
- **PAM, KWallet.** `pam_kwallet.c` prompts for a password when no `authtok`
  is set (lines 282-290). `open_session` needs the key data (lines 567-583).
- **Arch Wiki, GNOME/Keyring.** `pam_fprintd` does not set `PAM_AUTHTOK`, so
  a fingerprint login does not unlock the keyring. The page's workaround is a
  blank keyring password, which leaves the store unencrypted. The Arch Wiki
  fprint page says `pam_fprintd` is `sufficient`, and warns about CVE-2024-37408.
  I did not read the advisory.

Consequences for the design:

- **Locked search.** The store is one SQLCipher file, so `SearchItems` cannot
  read attributes while locked. Until unlock, a search returns the collection
  as locked and no items. GNOME keeps hashed attributes outside its cipher so
  search works while locked. That is a choice for Kieran (see "Needs Kieran").
- **Plain-text attributes are a trap.** KWallet's attribute file is readable
  without the wallet key. Gnoblin keeps attributes inside the cipher.
- **Fingerprint login.** The no-password prompt path in "Key source" covers
  this. The fingerprint finding is part of the decision, not a side note.

## Coexistence

The bus rules below come from the D-Bus specification, the `RequestName`
section.

- The service requests `org.freedesktop.secrets` with `DO_NOT_QUEUE` (0x4) and
  without `REPLACE_EXISTING` (0x2) and without `ALLOW_REPLACEMENT` (0x1).
- If another connection owns the name and did not set `ALLOW_REPLACEMENT`, the
  reply is `DBUS_REQUEST_NAME_REPLY_EXISTS` (3). The service then does not
  take the name.
- Without `ALLOW_REPLACEMENT`, nothing can replace Gnoblin's service once it
  owns the name.
- Safe default: when another provider owns the name, Gnoblin's service does not
  take it. The Secret portal still serves from Gnoblin's store, under a private
  name. Secret Service clients keep talking to the other provider. The two
  stores are then separate, and the consent text must say so.

Decided 2026-11-07 (item 35, revised): a config leaf, `keyring.enabled`, a bool that defaults to `true`, decides whether Gnoblin runs its built-in keyring. The doc line reads: "Run Gnoblin's built-in keyring. Turn it off to use a keyring from another program, or none." Gnoblin does not need to know which other keyring exists, so the earlier provider-name design (`keyring.provider`) is removed with no alias. With `keyring.enabled = false` the services role does not request `org.freedesktop.secrets`, creates no store file or directory, runs no keyring code, and the portal does not export the Secret object, so the system's keyring answers Secret requests. The unlock prompt is not a config switch: the keyring asks for its password through the `prompt.secret` slot. The config page is `docs/config/configure/keyring.md`.

## Secret Service API

The service implements these methods on the session bus at
`org.freedesktop.secrets`, as the specification defines them.

- `Service.OpenSession(algorithm, input)`: accepts `dh-ietf1024-sha256-aes128-cbc-pkcs7`
  and refuses `plain`. The DH group is the 1024-bit Oakley group 2 (RFC 2409).
- `Service.SearchItems`, `Service.Unlock`, `Service.Lock`, `Service.GetSecrets`:
  the store's lock state is per collection. `Unlock` returns a `Prompt` path,
  or `/` when no prompt is needed.
- `Service.ReadAlias`: the `default` alias points at one collection, created on
  first run. `SetAlias` and `CreateCollection` are refused over the bus.
- `Collection` and `Item` interfaces: read and search only. `SearchItems`,
  `GetSecret` and property reads work. Create, update and delete are refused
  over the bus. Attributes are stored in an indexed table, so `SearchItems`
  does not decrypt every item.
- `Prompt` objects: `Prompt(window-id)` and `Dismiss`. The `Completed` signal
  carries the result. The shell answers `prompt.secret` requests; see
  `docs/shell-api/passphrase-prompts.md`.

The store holds no plain-text secret. Every value is inside the SQLCipher file.

## Secret portal

The `org.freedesktop.impl.portal.Secret` backend holds one per-user master
secret in the store. It creates the master on first run, with 64 random bytes
in the default collection and a fixed attribute `gnoblin-purpose=portal-secret`.

Each app gets a secret derived from the master with HKDF-SHA256, using the app
id as `info`. [INFERENCE] This follows the portal model; check the Secret portal
specification before the code is written.

Consent follows the Secret row in `rust-compositor.md`: confirm first retrieval
per app. The grant lasts for the session, is held in memory, and is never
written to disk.

## Component

- **`gnoblin-service-keyring`** (services role, D4). It restarts without taking
  the compositor down. It owns the SQLCipher file and the bus name, or none of
  the bus name under coexistence.
- **`pam_gnoblin_keyring`.** The PAM module that hands the login password to
  the service. It is the only part outside the services role.
- **Portal side:** `xdg-desktop-portal-gnoblin` routes `Secret` requests to the
  services role through the usual portal path. The compositor draws the consent
  step.
- **Dependencies:** `rusqlite` with `bundled-sqlcipher`, and the crates the DH
  session needs (`sha2`, `hkdf`, `aes`, `cbc`, `num-bigint`,
  `getrandom`). The crate does not exist yet.

## Failure modes

| Event                                   | Result                                                                                 |
| --------------------------------------- | -------------------------------------------------------------------------------------- |
| Store locked (no key yet)               | Collections report locked. `Unlock` returns a `Prompt`. Portal: `error { locked }`     |
| No password at login                    | One unlock prompt per login session. Unlocked until logout. Never asked twice          |
| Unlock cancelled                        | `cancelled { user }`. The store stays locked                                           |
| Wrong key                               | SQLCipher refuses to open. Store stays locked. No data is touched                      |
| Password changed, hook missing          | Store stays locked. The user unlocks with the old password, then re-keys               |
| Provider refuses `dh-ietf1024`          | `error { not-supported }`. No `plain` fallback                                         |
| Built-in keyring off                    | No name request, no store opened, no Secret object. The system keyring answers         |
| Services role down                      | `error { not-supported }` until services returns, as for other services-hosted portals |
| Two or more master items found          | Use the oldest and log the others by id. Never create a second master                  |
| Default collection missing on first run | Create it, with the same attribute, if the store allows it                             |
| Session locked                          | Refused: `error { locked }`, as for every consent request                              |

## Security

- The master secret and the SQLCipher key live in the service process only.
  They are never logged, never sent to the compositor, and never put in argv or
  the environment.
- The PAM module hands the password over once, on the unlock socket, and does
  not keep it. The socket is `0600` in a per-login directory (see
  `gnoblin-keyring-handoff`).
- The compositor and the portal relay see only per-app secrets, and only after
  consent.
- Buffers holding a key or a secret are cleared after use. [INFERENCE] Confirm
  the chosen crates clear their buffers; if not, zeroise at the call site.
- A wrong key must not reveal the data. SQLCipher gives page-level encryption
  and an authenticated check on open.
- The KDF cost for a key derived from a password must be set explicitly.

## Phased implementation

Each phase is one commit. Each commit has Agent37 verification before the next
phase starts.

1. **Store.** Enable `bundled-sqlcipher` in the workspace. Add the crate skeleton
   and the SQLCipher schema: collections, items, attributes, aliases. Tests:
   open with key, wrong key refused, rekey.
2. **Secret Service server.** The `Service`, `Collection`, `Item` and `Prompt`
   interfaces over zbus, with `dh-ietf1024` only. Bus name taken under the
   coexistence rule. Tests: method replies match the specification.
   Built in `gnoblin-service-keyring`. Serves `dh-ietf1024` sessions only and
   requests the bus name with `DO_NOT_QUEUE`, so gnome-keyring keeps its name.
   A locked store returns no items; that choice lives in `search_locked` and is
   still open under "Needs Kieran" item 1.
3. **Unlock path.** The PAM hand-off (`pam_gnoblin_keyring`), the password-change
   hook, and the one-per-session unlock prompt for the no-password case. Tests:
   login with a password unlocks; login without one prompts once.
4. **Secret portal.** `RetrieveSecret`, the per-app derivation, the session grant
   in memory, consent through the compositor.
5. **Prompt UI.** Done for unlock (3fc351de6): the keyring offers `prompt.secret`
   requests and never draws UI. The shell answers them.
6. **Compat acceptance.** Run the acceptance checks below.

Phase 1 facts, measured on Agent37 (rusqlite 0.41.1, libsqlite3-sys 0.39.1):

- `bundled-sqlcipher` builds SQLCipher 4.19.0 from `sqlcipher-src` 419.0.6, which
  contains SQLite 3.53.4. Plain `bundled` builds SQLite 3.54.1, so the store moves
  back one SQLite minor version. The plain-file fixture opens unchanged with an
  empty key.
- The build compiles with `SQLITE_TEMP_STORE=2`, so temporary tables and indices
  default to memory. Plain `bundled` does not set it. `PRAGMA temp_store` can still override.
- The build links libcrypto dynamically (`dylib=crypto`) unless `OPENSSL_LIB_DIR` and
  `OPENSSL_INCLUDE_DIR` point elsewhere. It needs the OpenSSL 3 headers at build time.
  The runtime dependency is `libcrypto.so.3`, which `gnoblin` must declare (see
  `distribution.md`). `bundled-sqlcipher-vendored-openssl` would vendor it instead; not used.

Phase 3 facts, for `pam_gnoblin_keyring`:

- The module sits in the login stack after `pam_systemd`, so `XDG_RUNTIME_DIR` exists by the session stage. Without it, the module uses `/run/user/<uid>`.
- It never fails a login and never grants one. Every auth entry point returns `PAM_IGNORE`, and the session and password stages return `PAM_SUCCESS`.
- A hand-off that fails leaves the service to prompt once, as the no-password case does.
- The password-change stage re-keys the store only when both old and new passwords are present. If the service is down, the store stays locked with the old password.

Acceptance is compatibility with existing software:

- `secret-tool store` and `secret-tool lookup` (libsecret) round trip a value
  through `org.freedesktop.secrets` without change to libsecret or the caller.
- The portal `Retrieve` call returns the per-app secret after consent.
- An existing Secret Service client works without any change to its code or
  configuration.

## Out of scope

- Migrating secrets from gnome-keyring or KWallet, unless Kieran says yes.
- TPM sealing in the tree. It is an optional later add-on (appendix).
- Locking the store on screen lock, unless `keyring.lock-on-screen-lock` is
  built and turned on.

## Needs Kieran

These are real choices. Each one changes the work.

1. **Locked search:** a locked store returns no items (the current design, as
   KeePassXC does), or attributes are kept as hashed values outside the cipher
   so search works while locked (as GNOME does). The second weakens what an
   attacker learns from the file.
2. **Import of an existing gnome-keyring file:** yes or no.

## Appendix: TPM2 sealing (optional later add-on)

Not the default. Kept for when Kieran wants a disk-theft guard that needs no
password. The facts below were read from the sources named.

- **`systemd-creds` (man page).** A `tpm2` key cannot be used without the
  original TPM2 chip. The `host+tpm2` key also needs
  `/var/lib/systemd/credential.secret`, which only root can read. Systemd 256
  added `--user`, which binds a credential to one user. Systemd 262 pins the
  credential to the TPM2 SRK, so a credential made with 262 may not decrypt on
  the host's systemd 258. A null key is refused when TPM and Secure Boot are both
  in use.
- **Protection.** Against offline disk theft. A process running as the user can
  read the unsealed secret, so this does not protect against code already
  running as the user.
- **Host.** The host runs systemd 258 with TPM2 support. `/dev/tpm0` and
  `/dev/tpmrm0` are present. The tree has no TPM code.
- **Industry use.** TPM-bound keys are the norm for disks on Windows:
  BitLocker "provides maximum protection when used with a Trusted Platform
  Module (TPM)" ([Microsoft Learn: BitLocker](https://learn.microsoft.com/en-us/windows/security/operating-system-security/data-protection/bitlocker/)).
  For LUKS2 on Linux, `systemd-cryptenroll` enrols a TPM2 token alongside a
  passphrase or recovery key ([systemd-cryptenroll(1)](https://www.freedesktop.org/software/systemd/man/latest/systemd-cryptenroll.html)).
  Android Keystore keys can be bound to secure hardware, a Trusted Execution
  Environment (TEE) or a Secure Element (SE), not a TPM
  ([Android Keystore](https://developer.android.com/privacy-and-security/keystore)).
  Linux desktop keyrings (GNOME, KWallet) unlock from the login password, and
  none of the three sources above is a desktop keyring.
- **Answer to "is TPM the industry standard?"** It is the standard for disk
  encryption on Windows and an option on LUKS. It is not the standard for a
  desktop keyring, where the login password is the norm.
