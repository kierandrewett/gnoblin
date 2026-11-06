# Keyring service

Status: **Decided 2026-11-06** (D27 in `rust-compositor.md`). The key source is
an open problem, see "Key source: open problem". Kieran decided the service
and its store. The key source and the three points under "Needs Kieran" are
still to be chosen.

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

## Key source: open problem

The SQLCipher key must come from somewhere that survives reboot. Until one is
chosen, the store cannot unlock at boot, and phases 3 and later cannot land.
This is a risk to the schedule, not a reason to change the decision above.

Facts checked for this section:

- `pam_gnome_keyring` reads `PAM_AUTHTOK` in the auth stage. It starts
  `gnome-keyring-daemon --login` and writes the password on stdin. Its control
  socket is `$XDG_RUNTIME_DIR/keyring/control`, with operations INITIALIZE,
  UNLOCK, CHANGE and QUIT (`daemon/control/gkd-control-codes.h`). The client
  checks the socket owner and the peer uid with `SO_PEERCRED`.
- The host PAM stacks for `gdm-password`, greetd and cosmic-greeter run
  `pam_gnome_keyring.so` in auth (optional) and session (`auto_start`).
- The compositor is not a PAM peer. A keyring service started by PAM would be
  one, as the gnome-keyring daemon is. The earlier objection applied to the
  compositor process only.
- The host runs systemd 258 with TPM2 support. `/dev/tpm0` and `/dev/tpmrm0`
  are present. The tree has no TPM code.

| Option                               | How the key is obtained                                                                             | Survives reboot | Input needed                    |
| ------------------------------------ | --------------------------------------------------------------------------------------------------- | --------------- | ------------------------------- |
| 1. Login passphrase via PAM hand-off | A PAM module starts the service with the login password on stdin, as `pam_gnome_keyring` does.      | Yes             | Login, already typed            |
| 2. Separate passphrase at first use  | The service raises a `Prompt`. Gnoblin's prompt UI asks for a second passphrase once per session.   | Yes             | Second passphrase, each session |
| 3. `systemd-creds` / TPM2 sealed key | A random key is sealed with `systemd-creds encrypt` and loaded by `LoadCredentialEncrypted=`.       | Yes             | None                            |
| 4. Kernel session keyring cache      | The key lives only in the session keyring (`keyrings(7)`), for the life of the session.             | No              | Depends on another option       |
| 5. Coexistence (bus rule, not a key) | Gnoblin's service does not take `org.freedesktop.secrets` when another provider owns it. See below. | Not applicable  | Not applicable                  |

Notes on each option:

1. **PAM hand-off.** Gives automatic unlock with no second password. It needs a
   new PAM module, `pam_gnoblin_keyring`, in each greeter stack. A password
   change needs a hook in the PAM password stage, or the store stays locked.
   A mismatch leaves the store locked. [INFERENCE] The module design follows
   `pam_gnome_keyring`; it is not yet tested on greetd or cosmic-greeter.
2. **Second passphrase.** Needs no PAM work and no TPM. The store is locked
   until the user types the passphrase after each login. It needs the
   compositor running, so it cannot serve a boot-time unlock.
3. **`systemd-creds`.** The `systemd-creds` man page says a `tpm2` key cannot
   be used without the original TPM2 chip. The `host+tpm2` key also needs
   `/var/lib/systemd/credential.secret`, which only root can read. `--user`
   (systemd 256 and later) binds the credential to one user. The man page says
   systemd 262 pins the credential to the TPM2 SRK, so credentials made with
   262 may not decrypt on the host's 258. The protection is against offline
   disk theft. A process running as the user can read the secret.
4. **Kernel session keyring.** Never a root of trust. It lasts until logout or
   reboot, so every secret in the store is lost with it. Use it only as a cache,
   so a service restart in one session does not ask again. [INFERENCE] The
   semantics come from `keyrings(7)`, not re-read for this document.
5. **Coexistence.** Not a key source. See the next section.

Options 1 to 3 can be combined. For example, option 3 could hold the key and
option 4 could cache it. The choice is recorded under "Needs Kieran".

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

Kieran chooses whether that default stands ("Needs Kieran").

## Secret Service API

The service implements these methods on the session bus at
`org.freedesktop.secrets`, as the specification defines them.

- `Service.OpenSession(algorithm, input)`: accepts `dh-ietf1024-sha256-aes128-cbc-pkcs7`
  and refuses `plain`. The DH group is the 1024-bit Oakley group 2 (RFC 2409),
  the same arithmetic the gcr adapter uses, with a different constant.
- `Service.SearchItems`, `Service.Unlock`, `Service.Lock`, `Service.GetSecrets`:
  the store's lock state is per collection. `Unlock` returns a `Prompt` path,
  or `/` when no prompt is needed.
- `Service.ReadAlias`, `Service.SetAlias`, `Service.CreateCollection`: the
  `default` alias points at one collection, created on first run.
- `Collection` and `Item` interfaces: create, read, update and delete items.
  Attributes are stored in an indexed table, so `SearchItems` does not decrypt
  every item.
- `Prompt` objects: `Prompt(window-id)` and `Dismiss`. The `Completed` signal
  carries the result. Gnoblin's prompt UI draws them.

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
- **Portal side:** `xdg-desktop-portal-gnoblin` routes `Secret` requests to the
  services role through the usual portal path. The compositor draws the consent
  step.
- **Dependencies:** `rusqlite` with `bundled-sqlcipher`, and the crates the gcr
  adapter already needs (`sha2`, `hkdf`, `aes`, `cbc`, `num-bigint`,
  `getrandom`). The crate does not exist yet.

## Failure modes

| Event                                   | Result                                                                                 |
| --------------------------------------- | -------------------------------------------------------------------------------------- |
| Store locked (no key yet)               | Collections report locked. `Unlock` returns a `Prompt`. Portal: `error { locked }`     |
| Unlock cancelled                        | `cancelled { user }`                                                                   |
| Wrong key                               | SQLCipher refuses to open. Store stays locked. No data is touched                      |
| Provider refuses `dh-ietf1024`          | `error { not-supported }`. No `plain` fallback                                         |
| Other provider owns the name            | Coexistence: the service does not take the name. Portal serves from the private name   |
| Services role down                      | `error { not-supported }` until services returns, as for other services-hosted portals |
| Two or more master items found          | Use the oldest and log the others by id. Never create a second master                  |
| Default collection missing on first run | Create it, with the same attribute, if the store allows it                             |
| Session locked                          | Refused: `error { locked }`, as for every consent request                              |

## Security

- The master secret and the SQLCipher key live in the service process only.
  They are never logged, never sent to the compositor, and never put in argv or
  the environment.
- The compositor and the portal relay see only per-app secrets, and only after
  consent.
- Buffers holding a key or a secret are cleared after use. [INFERENCE] Confirm
  the chosen crates clear their buffers; if not, zeroise at the call site.
- A wrong key must not reveal the data. SQLCipher gives page-level encryption
  and an authenticated check on open.
- If the key source is a passphrase, the KDF cost must be set explicitly. The
  choice is part of the key-source decision.

## Phased implementation

Each phase is one commit. Each commit has Agent37 verification before the next
phase starts. The phases after phase 2 depend on the key-source decision.

1. **Store.** Enable `bundled-sqlcipher` in the workspace. Add the crate skeleton
   and the SQLCipher schema: collections, items, attributes, aliases. Tests:
   open with key, wrong key refused, rekey.
2. **Secret Service server.** The `Service`, `Collection`, `Item` and `Prompt`
   interfaces over zbus, with `dh-ietf1024` only. Bus name taken under the
   coexistence rule. Tests: method replies match the specification.
3. **Key source.** Implement the option Kieran chooses. Blocked until then. It
   changes only the unlock path.
4. **Secret portal.** `RetrieveSecret`, the per-app derivation, the session grant
   in memory, consent through the compositor.
5. **Prompt UI.** Gnoblin's prompt UI draws `Prompt` objects (`prompt.secret`).
6. **Compat acceptance.** Run the acceptance checks below.

Acceptance is compatibility with existing software:

- `secret-tool store` and `secret-tool lookup` (libsecret) round trip a value
  through `org.freedesktop.secrets` without change to libsecret or the caller.
- The portal `Retrieve` call returns the per-app secret after consent.
- An existing Secret Service client works without any change to its code or
  configuration.

## Out of scope

- Migrating secrets from gnome-keyring or KWallet, unless Kieran says yes.
- TPM or smartcard storage in the tree. Option 3 needs it, and it is not in
  the tree today.
- Keyring unlock at login, unless option 1 is chosen.

## Needs Kieran

These are real choices. Each one changes the work.

1. **Key source:** option 1, 2 or 3, alone or combined, with option 4 as cache.
2. **Import of an existing gnome-keyring file:** yes or no.
3. **Coexistence default:** keep the safe default above, or take the name when
   free and fall back otherwise.
