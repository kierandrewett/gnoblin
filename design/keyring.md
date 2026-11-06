# Keyring service

Status: **Proposed 2026-11-06** (D27 in `rust-compositor.md`). Kieran signs off
the choice in "Decision" before any code lands.

## Summary

Gnoblin stores no secrets. A Secret Service provider on the session bus holds
them: gnome-keyring, KWallet's bridge, KeePassXC or oo7. Gnoblin is a client of
that provider. It serves two things:

- the `org.freedesktop.impl.portal.Secret` portal, backed by one per-user
  master secret kept in the provider;
- the `prompt.secret` slot, so an unlock prompt the provider raises through the
  gcr names can be shown by Gnoblin's own consent and prompt code.

Gnoblin writes no secret file, holds no key of its own, and adds no new crypto
dependency. Without a provider the Secret portal is unavailable and falls
through to the next portal backend.

## Why Gnoblin does not own a store

A store needs a key, and the compositor has no source for one that survives
a reboot:

| Key source             | Problem                                                                               |
| ---------------------- | ------------------------------------------------------------------------------------- |
| Login passphrase       | The compositor cannot see it. PAM hands it to the keyring, not to Gnoblin.            |
| A second passphrase    | Asked at every session start. A second password for users who already have a keyring. |
| Kernel session keyring | Lost at logout or reboot. Every secret in the store is lost with it.                  |
| TPM                    | No TPM code in the tree. Not in scope for 0.2.0.                                      |

SQLCipher (`rusqlite` with `bundled-sqlcipher` in the workspace) would encrypt
the store, but it still needs the key above. It does not remove the problem.
A store with no key source is rejected. Option B in the open-decision table of
`rust-compositor.md` is therefore dropped.

## Options considered

| Option                                 | Trade-off                                                                                       |
| -------------------------------------- | ----------------------------------------------------------------------------------------------- |
| A. Client of `org.freedesktop.secrets` | Chosen. No store, no key to lose. Needs a provider on the bus, which every desktop session has. |
| B. Own file store                      | Rejected. Needs a key source that survives reboot, and there is none (table above).             |
| C. No Secret portal                    | Fallback when no provider runs. Same result as A without a provider: next backend answers.      |

## Decision

**Option A, with C as the fallback.** Gnoblin asks the running Secret Service
for the master secret and gives each app a secret derived from it. The portal
behaviour and consent rules stay as in `rust-compositor.md`, section "Secret".

## Component

- **`gnoblin-service-secret`** (services role, D4). It is a Secret Service
  client, so it runs with the other services and restarts without taking the
  compositor down. Its shape follows `gnoblin-service-location`. The crate does
  not exist yet.
- **Portal side:** `xdg-desktop-portal-gnoblin` routes `Secret` requests to the
  services role through the usual portal path (see "Portal" in
  `rust-compositor.md`). The compositor draws the consent step.
- **No new dependency:** the transfer below uses the crates the gcr adapter
  already needs (`sha2`, `hkdf`, `aes`, `cbc`, `num-bigint`, `getrandom` in
  the workspace `Cargo.toml`). The gcr protocol uses a 1536-bit MODP group;
  Secret Service `dh-ietf1024` uses the 1024-bit Oakley group 2 (RFC 2409).
  The same arithmetic serves both, with a different group constant.

## Secret Service use

Each step names the Secret Service method it uses.

1. **Session.** `OpenSession("dh-ietf1024-sha256-aes128-cbc-pkcs7", client_pub)`
   returns the service public key. The session key is HKDF-SHA256 with no salt
   and empty info, truncated to 128 bits. Secrets travel AES-128-CBC with PKCS7
   padding, with a fresh 16-byte IV per call.
2. **Find the master.** `SearchItems({"gnoblin-purpose": "portal-secret"})`.
   Unlocked items come back directly. Locked items need step 3.
3. **Unlock.** `Unlock(locked)` returns a prompt path, or `/` when no prompt
   is needed. A prompt is drawn by the provider. If the provider routes it
   through a gcr name Gnoblin serves, Gnoblin shows it through `prompt.secret`.
4. **Create, first run only.** When no item matches and the default collection
   is unlocked, create one item in the default collection (`ReadAlias("default")`)
   with 64 random bytes as its secret and the attribute above.
5. **Read.** `GetSecrets([master], session)` returns the secret for the session.

Plain transfer (`plain`) is never used. The spec allows it, but it sends the
master secret across the bus in clear. If the provider refuses the DH
algorithm, the request fails with `error { not-supported }`.

## Per-app secrets

The portal gives each app a secret that depends on its app id and the master
secret, so one app cannot read another app's secret. The derivation uses
HKDF-SHA256 with the app id as `info`. [INFERENCE] This follows the portal
model; check the Secret portal specification before the code is written.

Consent for each app follows the Secret row in `rust-compositor.md`: "confirm
first retrieval per app". The grant lasts for the session, is held in memory,
and is never written to disk. It is never offered as "remember". This reads
the existing no-remember rule as covering the dialog, not the session grant;
Kieran confirms that reading.

## Failure modes

| Event                                       | Result                                                                                        |
| ------------------------------------------- | --------------------------------------------------------------------------------------------- |
| No Secret Service on the bus                | `error { not-supported }`; portal falls through to the next backend                           |
| Provider refuses `dh-ietf1024`              | `error { not-supported }`. No `plain` fallback                                                |
| Provider restarts or bus name vanishes      | Session is dropped with the connection. Reopen the session and retry once, then `unavailable` |
| Master item locked and unlock is cancelled  | `cancelled { user }`                                                                          |
| Two or more master items found              | Use the oldest, log the others by id. Never create a second item                              |
| Default collection missing on first run     | Create the item in a new collection with the same attribute, if the provider allows it        |
| Provider changed (gnome-keyring to KWallet) | The master differs, so every app gets a new portal secret. Documented, not migrated           |
| Session locked                              | Refused: `error { locked }`, as for every consent request                                     |
| Services role down                          | `error { not-supported }` until services returns, as for other services-hosted portals        |

## Security

- The master secret lives in the services process only. It is never logged, never
  sent to the compositor, and never placed in argv or the environment.
- The compositor and the portal relay see only per-app secrets, and only after
  consent.
- Buffers holding the master or a session key are cleared after use. [INFERENCE]
  Confirm the chosen crates clear their buffers; if they do not, add zeroising
  at the call site.
- Switching the provider gives every app a new secret. Users are told this in
  the consent dialog text; no migration is offered.

## Out of scope

- A Gnoblin-owned keyring or secret file (option B).
- Migrating secrets between providers.
- TPM or smartcard storage.
- Keyring unlock at login. PAM and the provider own that.

## Open questions for Kieran

1. Accept option A, or keep the Secret portal out of 0.2.0 (option C).
2. Confirm the session-long consent reading in "Per-app secrets".
3. Confirm that switching provider giving every app a new secret is acceptable.
