# Constants audit: ipc group

Read-only audit of the IPC crates: `gnoblin-client`, `gnoblin-proto`, `gnoblin-types`, `gnoblin-model`, `xdg-desktop-portal-gnoblin`, `gnoblinctl` and `gnoblin-cli-style`. No Rust was changed or built. Every row cites `crates/<crate>/src/<file>.rs:<line>` at the time of the audit.

Method: item-level `const` and `static` lines outside test modules, found with grep, plus a read of the config schema (`crates/gnoblin-schema/schema/*.toml`), `design/rust-compositor.md` (D3, D16, D19, D22, D23) and the portal specification XML. Items inside `#[cfg(test)]` modules and `const fn` constructors are out of scope, except where a constructor value is a named constant (`ZERO`, `ONE`, `ALL`).

Spec sources checked (raw XML, `flatpak/xdg-desktop-portal` main branch):

- https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/main/data/org.freedesktop.portal.Screenshot.xml
- https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/main/data/org.freedesktop.portal.ScreenCast.xml
- https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/main/data/org.freedesktop.portal.RemoteDesktop.xml
- https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/main/data/org.freedesktop.portal.InputCapture.xml
- https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/main/data/org.freedesktop.portal.Settings.xml
- https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/main/data/org.freedesktop.portal.GlobalShortcuts.xml
- https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/main/data/org.freedesktop.portal.Clipboard.xml
- https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/main/data/org.freedesktop.portal.Inhibit.xml

## 1. Summary per crate

Counts are item-level `const` and `static` items outside test modules. The brief's figures (35, 25, 113, 13, 39, 16, 7) appear to include `const fn` and test items [INFERENCE]; the figures below are the ones this audit covers.

| Crate                      | Items | Covered by                        | Notes                                                          |
| -------------------------- | ----: | --------------------------------- | -------------------------------------------------------------- |
| gnoblin-client             |    33 | 9 grouped rows, all items listed  | Wire member and code strings are hand-written here             |
| gnoblin-proto              |    22 | 9 grouped rows, all items listed  | Wire framing; protobuf via prost                               |
| gnoblin-types              |    33 | 8 grouped rows, all items listed  | Plus 81 `const fn` constructors, out of scope                  |
| gnoblin-model              |     4 | 3 grouped rows, all items listed  | Slot count duplicates `ALL`                                    |
| xdg-desktop-portal-gnoblin |    29 | 11 grouped rows, all items listed | Settings `KEY` table holds 13 rows, counted as one item        |
| gnoblinctl                 |    13 | 6 grouped rows, all items listed  | Includes five CLI argument tables in `args.rs`                 |
| gnoblin-cli-style          |     6 | 4 grouped rows, all items listed  | ANSI colour codes sit inline in `theme.rs:96-104`, not counted |

## 2. Findings

### 2.1 gnoblin-client

| Item (evidence)                                                                                       | Value                                                       | Q1 user | Q2 upstream                                                                                             | Q3 needed | Verdict | Risk   | Effort |
| ----------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------- | --------- | ------- | ------ | ------ |
| `SOCKET_VARIABLE`, `RUNTIME_VARIABLE`, `DEFAULT_SOCKET` (`unix.rs:45`, `:48`, `:51`)                  | `GNOBLIN_IPC_SOCKET`, `XDG_RUNTIME_DIR`, `gnoblin/ipc.sock` | No      | XDG base dir spec: `XDG_RUNTIME_DIR` is the variable; the `gnoblin/` subdir is ours                     | No        | KEEP    | Low    | S      |
| `NAMESPACE` = `gnoblin` (`wire.rs:54`); same string in `generate.rs:36` as `CORE_NAMESPACE`           | `gnoblin`                                                   | No      | Ours; no upstream                                                                                       | Partly    | DERIVE  | Low    | S      |
| Member strings `object.*`, `request.*`, `slot.*`, `config.*`, `portal.*` (`wire.rs:59-102`, 20 items) | e.g. `object.subscribe`                                     | No      | Ours. Schema has `since` fields but no member names; one copy in client `wire.rs`, reused by gnoblinctl | Yes       | DERIVE  | Medium | M      |
| Code strings `request-taken` ... `no-provider` (`wire.rs:108-132`, 9 items)                           | e.g. `denied`, `no-provider`                                | No      | Ours; schema has no error table                                                                         | Yes       | DERIVE  | Medium | M      |
| `UNKNOWN_MEMBER` re-exported from proto (`wire.rs:119`)                                               | `unknown-member`                                            | No      | Proto owns it (`envelope.rs:32`)                                                                        | No        | REMOVE  | Low    | S      |
| Offer timeout `Duration` to `u64` ms, fails open to `u64::MAX` (`client.rs:295-306`)                  | not a literal                                               | No      | Ours                                                                                                    | No        | KEEP    | Low    | S      |

Why the member and code strings should be derived: `gnoblinctl` uses them through `gnoblin_client::code` and they are wire contract. D16 says the wire is generated from `gnoblin-schema` [design/rust-compositor.md, D16]. Generating them from the schema removes the hand-written table in `wire.rs`. The schema has `since` fields (`crates/gnoblin-schema/schema/session.toml:12`) so the generator has a place to hang them.

### 2.2 gnoblin-proto

| Item (evidence)                                                                                             | Value                                                           | Q1 user | Q2 upstream                                                                                               | Q3 needed | Verdict       | Risk   | Effort |
| ----------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- | ------- | --------------------------------------------------------------------------------------------------------- | --------- | ------------- | ------ | ------ |
| `Capability::ALL` (`capability.rs:82`) and `Capabilities::NONE` (`capability.rs:142`)                       | 9 items; `0`                                                    | No      | Ours                                                                                                      | Yes       | KEEP          | Low    | S      |
| Capability bits and names, all nine (`capability.rs:82-120`)                                                | bits `1<<0`..`1<<8` in a `u16` (grant bytes 5-6, little endian) | No      | Ours; wire ABI. A tenth capability needs another width or a new field                                     | Yes       | KEEP          | Medium | S      |
| `FLAG_SENSITIVE = 1 << 0` (`envelope.rs:27`)                                                                | `1`                                                             | No      | Ours; wire ABI; redaction rule per `request.toml:7`                                                       | Yes       | KEEP          | High   | S      |
| `UNKNOWN_MEMBER = "unknown-member"` (`envelope.rs:32`)                                                      | string                                                          | No      | Ours; wire ABI                                                                                            | Yes       | KEEP          | Medium | S      |
| `MAX_FRAME_LEN = 16 MiB` (`frame.rs:21`)                                                                    | 16,777,216 bytes                                                | No      | Ours; DoS bound. Doc says largest real frame is a frame export                                            | Yes       | KEEP          | Medium | S      |
| `LENGTH_PREFIX_LEN = 4` (`frame.rs:24`)                                                                     | 4                                                               | No      | Ours; wire ABI                                                                                            | Yes       | KEEP          | High   | S      |
| `MAX_GRANTS = 64`, `MAX_NAMESPACE_LEN = 128`, `VERSION_LEN = 12`, `MAX_GRANTS_BODY_LEN` (`grants.rs:28-39`) | 64 / 128 / 12 / derived                                         | No      | Ours; protocol bounds                                                                                     | Yes       | KEEP          | Medium | S      |
| `LENGTH_PREFIX_LEN = 4` duplicated (`grants.rs:42`)                                                         | 4                                                               | No      | Duplicate of `frame.rs:24`                                                                                | No        | REMOVE        | Low    | S      |
| `MAGIC = *b"GNBL"`, `HANDSHAKE_LEN = 20` (`handshake.rs:27`, `:31`)                                         | bytes                                                           | No      | Ours; wire ABI                                                                                            | Yes       | KEEP          | High   | S      |
| `IPC_VERSION` from `env!("GNOBLIN_IPC_VERSION")` (`lib.rs:72`)                                              | build-time version                                              | No      | Already derived from the Cargo version; session schema says it is the Gnoblin version (`session.toml:14`) | No        | DERIVE (done) | Low    | S      |
| `FD_VARIABLE`, `CONTROL_VARIABLE`, `OWNER_PREFIX` (`privilege.rs:41`, `:48`, `:54`)                         | `GNOBLIN_IPC_FD`, `GNOBLIN_IPC_CONTROL_FD`, `GNOBLIN_IPC_FD_`   | No      | Ours: env handover between roles                                                                          | Yes       | KEEP          | Medium | S      |
| `OWNER_PREFIX` is used at `privilege.rs:62` and `:68`, so it stays                                          | `GNOBLIN_IPC_FD_`                                               | No      | Ours; live env handover prefix                                                                            | Yes       | KEEP          | Low    | S      |
| `MAX_HANDOVER_PAYLOAD = 64` (`privilege.rs:95`)                                                             | 64                                                              | No      | Ours; bound on descriptor payload                                                                         | Yes       | KEEP          | Low    | S      |
| `READY_VARIABLE`, `READY`, `CONFIG_ACCEPTED`, `CONFIG_REJECTED` (`report.rs:32`, `:35`, `:38`, `:41`)       | `GNOBLIN_READY_FD` and three strings                            | No      | Ours; read by the service supervisor                                                                      | Yes       | KEEP          | Medium | S      |

### 2.3 gnoblin-types

| Item (evidence)                                                                                              | Value                 | Q1 user | Q2 upstream                                                                                                 | Q3 needed | Verdict       | Risk   | Effort |
| ------------------------------------------------------------------------------------------------------------ | --------------------- | ------- | ----------------------------------------------------------------------------------------------------------- | --------- | ------------- | ------ | ------ |
| `Direction::ALL`, `Side::ALL`, `Corner::ALL` (`direction.rs:37`, `:98`, `:175`)                              | 4-element arrays      | No      | Ours                                                                                                        | No        | KEEP          | Low    | S      |
| `Orientation::ALL` (`orientation.rs:40`)                                                                     | 8-element array       | No      | Ours                                                                                                        | No        | KEEP          | Low    | S      |
| `ZERO`, `ONE`, `EMPTY` in geometry (`geometry.rs:27`, `:82`, `:133`, `:382`, `:450`, `:501`, `:603`, `:606`) | 0 and 1 values        | No      | Math identities                                                                                             | No        | KEEP          | Low    | S      |
| `Scale::IDENTITY` (`scale.rs:21`), `Transform::IDENTITY` (`transform.rs:111`)                                | 1 and identity        | No      | Ours                                                                                                        | No        | KEEP          | Low    | S      |
| `Tick::ZERO` (`tick.rs:22`, `:79`)                                                                           | 0                     | No      | Ours                                                                                                        | No        | KEEP          | Low    | S      |
| Modifier bits `SHIFT`..`MOD5` = `1<<0`..`1<<7` (`keyboard.rs:124-142`)                                       | X11 core state bits   | No      | Match the X11 core state mask and XKB real modifiers [INFERENCE: xkbcommon docs not fetched]                | Yes       | KEEP          | Medium | S      |
| `SUPER` `1<<26`, `HYPER` `1<<27`, `META` `1<<28` (`keyboard.rs:144-152`)                                     | virtual modifier bits | No      | [INFERENCE] believed to match Clutter's `ClutterModifierType`; the Clutter header fetch failed this session | Yes       | KEEP (verify) | Medium | S      |
| `ALT = MOD1` (`keyboard.rs:154`)                                                                             | alias                 | No      | Ours                                                                                                        | No        | KEEP          | Low    | S      |
| `MAX_HANDLE_BYTES = 64` (`parent_window.rs:25`)                                                              | 64                    | No      | Portal spec gives no length [INFERENCE]; bound on untrusted input                                           | Yes       | KEEP          | Medium | S      |
| `MAX_XID_DIGITS = 8` (`parent_window.rs:28`)                                                                 | 8 hex digits          | No      | X11 window IDs are 32-bit, so eight hex digits is the full width                                            | Yes       | KEEP          | Low    | S      |
| `transform.rs:56` `CODE = "unsupported-3d"`                                                                  | string                | No      | Ours; error code                                                                                            | Yes       | KEEP          | Low    | S      |

### 2.4 gnoblin-model

| Item (evidence)                                                          | Value         | Q1 user | Q2 upstream                 | Q3 needed | Verdict | Risk | Effort |
| ------------------------------------------------------------------------ | ------------- | ------- | --------------------------- | --------- | ------- | ---- | ------ |
| `REMEMBERED_FINISHED = 256` (`request.rs:41`, used at `:316`)            | tombstone cap | No      | Ours                        | Yes       | KEEP    | Low  | S      |
| `COUNT = 4` (`slot.rs:58`) used for `CapabilitySlot::ALL` (`slot.rs:80`) | 4             | No      | Ours                        | No        | DERIVE  | Low  | S      |
| `OBJECT_GONE = "object-gone"` (`graph.rs:631`)                           | string        | No      | Ours; code in `wire.rs` set | Yes       | KEEP    | Low  | S      |

`COUNT` duplicates the array length. Use `ALL.len()` in a const context, or drop `COUNT` and let `providers` size from `ALL`. Shared with the model rows for slot names (`slot.rs:89-113`, `auth.polkit`, `prompt.secret`, `location.agent`, `idle.policy`): these are D22-era slot names. KEEP, they are the wire contract.

### 2.5 xdg-desktop-portal-gnoblin

#### 2.5.1 Interface versions

Compared against the specification XML. Per-method version deltas (which methods were added in which version) were not verified line by line [INFERENCE]. The table compares Gnoblin's constant with the spec's declared version.

| Item (evidence)                                      | Gnoblin | Spec | Q2 finding                                                                            | Verdict         | Risk   | Effort |
| ---------------------------------------------------- | ------- | ---- | ------------------------------------------------------------------------------------- | --------------- | ------ | ------ |
| `SCREENSHOT_VERSION` (`iface/capture.rs:57`)         | 2       | 3    | Spec is 3. Gnoblin reports 2 [INFERENCE: which methods differ not checked]            | DERIVE or fix   | Medium | M      |
| `SCREENCAST_VERSION` (`iface/capture.rs:60`)         | 3       | 6    | Spec is 6. Comment says Gnoblin reports 3 deliberately; delta not checked [INFERENCE] | KEEP (document) | High   | S      |
| `REMOTE_DESKTOP_VERSION` (`iface/remote.rs:42`)      | 2       | 2    | Matches                                                                               | KEEP            | Low    | S      |
| `INPUT_CAPTURE_VERSION` (`iface/remote.rs:44`)       | 2       | 2    | Matches                                                                               | KEEP            | Low    | S      |
| `VERSION` in `iface/settings.rs:26`                  | 2       | 2    | Matches                                                                               | KEEP            | Low    | S      |
| `GLOBAL_SHORTCUTS_VERSION` (`iface/shortcuts.rs:39`) | 1       | 2    | Spec is 2. Check served methods before any bump                                       | Verify (check)  | Medium | S      |
| `CLIPBOARD_VERSION` (`iface/shortcuts.rs:41`)        | 1       | 1    | Matches                                                                               | KEEP            | Low    | S      |
| `VERSION` in `iface/inhibit.rs:31`                   | 1       | 3    | Spec is 3. Check served methods before any bump                                       | Verify (check)  | Medium | S      |

The right long-term fix is to attach `since` to each method in the portal table and derive the reported version as the maximum served, rather than hand-typing it. Bumping versions changes what clients see, so it needs Kieran's decision (section 5).

#### 2.5.2 Bus, path, timing and ids

| Item (evidence)                                                                         | Value  | Q1 user | Q2 upstream                                                                                                                                   | Q3 needed | Verdict                                       | Risk   | Effort |
| --------------------------------------------------------------------------------------- | ------ | ------- | --------------------------------------------------------------------------------------------------------------------------------------------- | --------- | --------------------------------------------- | ------ | ------ |
| `BUS_NAME` `org.freedesktop.impl.portal.desktop.gnoblin` (`iface/mod.rs:58`)            | string | No      | Backend name convention, fixed by `portals.conf` (`lib.rs:109`) and portal dispatch                                                           | Yes       | KEEP                                          | High   | S      |
| `PATH` `/org/freedesktop/portal/desktop` (`iface/mod.rs:61`)                            | string | No      | Portal spec object path                                                                                                                       | Yes       | KEEP                                          | High   | S      |
| `TICK = 200 ms` (`iface/mod.rs:64`)                                                     | 200 ms | No      | Ours; polling cadence of the request loop                                                                                                     | Yes       | KEEP                                          | Low    | S      |
| `READ_TIMEOUT = 20 ms` (`host.rs:59`)                                                   | 20 ms  | No      | Ours; bounded blocking read                                                                                                                   | Partly    | REMOVE (use event loop readiness) [INFERENCE] | Medium | M      |
| `SLOT = consent::PORTAL_UI` (`host.rs:52`), `PORTAL_UI = "portal.ui"` (`consent.rs:85`) | string | No      | Ours; wire name, already in client `wire.rs:97`                                                                                               | Yes       | DERIVE                                        | Low    | S      |
| `OFFERER = ClientId::new(0)` (`consent.rs:96`)                                          | 0      | No      | Ours; the compositor is client 0                                                                                                              | No        | KEEP                                          | Low    | S      |
| `CONSENT_LIFETIME = 120_000` ms (`main.rs:47-49`)                                       | 120 s  | Yes     | Comment says 2 min is the frontend default. Not supported by the spec: async calls time out at 25 s; dialogs may stay open longer [INFERENCE] | Yes       | CONFIGURE                                     | Medium | S      |
| `PORTALS_CONF = include_str!("../data/portals.conf")` (`lib.rs:109`)                    | file   | No      | Ours; packaging contract                                                                                                                      | No        | KEEP                                          | Low    | S      |

Proposed leaf for `CONSENT_LIFETIME`: `portal.consent.timeout`, type duration, unit milliseconds, default `120000`, range `10000`-`600000`. Belongs in core config, because consent is compositor-drawn (D19). Fix the comment if the default stays, since the spec does not say 2 min.

#### 2.5.3 Capture, remote and settings bits

| Item (evidence)                                                                     | Value                                            | Q1 user | Q2 upstream                                                                                                             | Q3 needed | Verdict                   | Risk   | Effort |
| ----------------------------------------------------------------------------------- | ------------------------------------------------ | ------- | ----------------------------------------------------------------------------------------------------------------------- | --------- | ------------------------- | ------ | ------ |
| `SOURCE_MONITOR`, `SOURCE_WINDOW`, `SOURCE_VIRTUAL` (`iface/capture.rs:41-45`)      | 1, 2, 4                                          | No      | ScreenCast spec `AvailableSourceTypes` bits match                                                                       | Yes       | KEEP                      | High   | S      |
| `CURSOR_HIDDEN`, `CURSOR_EMBEDDED`, `CURSOR_METADATA` (`iface/capture.rs:47-51`)    | 1, 2, 4                                          | No      | ScreenCast spec cursor-mode bits match                                                                                  | Yes       | KEEP                      | High   | S      |
| `DEVICE_KEYBOARD`, `DEVICE_POINTER`, `DEVICE_TOUCHSCREEN` (`iface/remote.rs:35-39`) | 1, 2, 4                                          | No      | RemoteDesktop spec device-type bits match                                                                               | Yes       | KEEP                      | High   | S      |
| `VERSION` / keys in `settings.rs:97` `KEY` table (13 rows)                          | `org.freedesktop.appearance` keys and GNOME keys | No      | Appearance keys match the Settings spec. Remaining keys are GNOME GSettings names (`settings.rs:97-130`), not spec keys | Yes       | KEEP names, DERIVE values | Medium | M      |
| `ACCENT`, `ACCENT_NAMESPACE`, `ACCENT_KEY` (`settings.rs:170-174`)                  | `accent_red` ..., `accent-color`                 | No      | Matches the Settings spec key name `accent-color` [INFERENCE]. Component names are ours                                 | No        | KEEP                      | Low    | S      |

### 2.6 gnoblinctl

| Item (evidence)                                                                             | Value               | Q1 user | Q2 upstream                                                                            | Q3 needed | Verdict                     | Risk   | Effort |
| ------------------------------------------------------------------------------------------- | ------------------- | ------- | -------------------------------------------------------------------------------------- | --------- | --------------------------- | ------ | ------ |
| `PARAMS`, `METHOD`, `SECRET`, `AUTH`, `BARE` (`args.rs:87`, `:100`, `:148`, `:153`, `:164`) | CLI argument tables | No      | Ours; per-method arg schema                                                            | Yes       | DERIVE                      | Medium | M      |
| `SHELLS = ["bash", "zsh", "fish"]` (`cli.rs:30`)                                            | 3 shells            | No      | Shell names; gnoblinctl already has a `Shell` enum (`cli.rs:80`, `:88`)                | No        | DERIVE                      | Low    | S      |
| `LEVELS = ["TRACE" ... "ERROR"]` (`cli.rs:33`)                                              | 5 log levels        | No      | Ours (`gnoblin-log`, D23)                                                              | No        | DERIVE                      | Low    | S      |
| `OBJECT_UNKNOWN`, `UNKNOWN_MEMBER` (`commands.rs:24`, `:27`)                                | string              | No      | Already `gnoblin_client::code::*`                                                      | No        | REMOVE                      | Low    | S      |
| `CORE_NAMESPACE = "gnoblin"` (`generate.rs:36`)                                             | string              | No      | Duplicate of `gnoblin-client` `NAMESPACE`                                              | No        | DERIVE                      | Low    | S      |
| `IPC_VERSION` emitted (`generate.rs:179`)                                                   | tuple format        | No      | Output template, not a value                                                           | No        | KEEP                        | Low    | S      |
| `DEFAULT_INTERVAL = 500 ms` (`logs.rs:27`)                                                  | 500 ms              | Yes     | Ours; poll interval for `logs --follow`. Already exposed as `--interval` (`logs.rs:9`) | Yes       | KEEP (already configurable) | Low    | S      |
| `BATCH = 1_000` (`logs/follow.rs:20`)                                                       | 1000 lines          | No      | Ours; page size                                                                        | Yes       | KEEP                        | Low    | S      |

### 2.7 gnoblin-cli-style

| Item (evidence)                                             | Value                                        | Q1 user | Q2 upstream                                                          | Q3 needed | Verdict                       | Risk | Effort |
| ----------------------------------------------------------- | -------------------------------------------- | ------- | -------------------------------------------------------------------- | --------- | ----------------------------- | ---- | ------ |
| `GAP = 2`, `LABEL_COLUMN = 7` (`components.rs:9`, `:11`)    | columns                                      | No      | Ours; layout                                                         | Partly    | KEEP                          | Low  | S      |
| `DEFAULT_WIDTH = 80` (`theme.rs:10`)                        | 80 columns                                   | No      | Fallback when no terminal; `COLUMNS` already read (`theme.rs:50-65`) | Yes       | DERIVE (ioctl, then fallback) | Low  | S      |
| `MIN_WIDTH = 40` (`theme.rs:12`)                            | 40 columns                                   | No      | Ours; floor                                                          | Yes       | KEEP                          | Low  | S      |
| `SPINNER_UNICODE`, `SPINNER_ASCII` (`theme.rs:148`, `:152`) | glyph arrays                                 | No      | Ours; ASCII fallback for non-UTF-8 terminals                         | Yes       | KEEP                          | Low  | S      |
| ANSI SGR codes `31`-`36`, `90` (`theme.rs:96-104`)          | basic 16-colour palette, incl. `34` and `90` | No      | Standard SGR codes; the terminal theme applies to palette colours    | Yes       | KEEP                          | Low  | S      |
| `NO_COLOR`, `CLICOLOR_FORCE`, `TERM` (`theme.rs:42-63`)     | env vars                                     | No      | Already honoured (no-color.org convention)                           | No        | KEEP                          | Low  | S      |

## 3. Top ten wins

Ranked by user value over effort.

1. **Consent timeout as a leaf.** `CONSENT_LIFETIME` (`main.rs:49`) becomes `portal.consent.timeout`, default 120 s; fix the comment that claims a spec basis. Risk low, effort S.
2. **Derive member and code strings from the schema.** Generate `wire.rs:54-132` from `gnoblin-schema`, removing the hand-kept table that `gnoblinctl` reads through `gnoblin_client::code`. Risk medium, effort M.
3. **Portal versions from per-method `since`.** Replace `SCREENSHOT_VERSION`, `GLOBAL_SHORTCUTS_VERSION`, `inhibit.rs:31` with a derived value, after checking served methods. Risk medium, effort M.
4. **Fix the Screenshot version gap.** Gnoblin reports 2; the spec is 3 (`capture.rs:57`). Confirm which methods are served at version 2, then decide whether to bump. Risk medium, effort S.
5. **Derive CLI shell and level lists.** `SHELLS` and `LEVELS` (`cli.rs:30`, `:33`) from clap `ValueEnum` and the existing `Shell` enum (`cli.rs:80`). Risk low, effort S.
6. **Terminal width from the terminal.** `DEFAULT_WIDTH` (`theme.rs:10`) becomes the fallback after `COLUMNS` and the ioctl query. Risk low, effort S.
7. **Remove duplicate constants.** `LENGTH_PREFIX_LEN` (`grants.rs:42`), `CORE_NAMESPACE` (`generate.rs:36`), `OBJECT_UNKNOWN` and `UNKNOWN_MEMBER` (`commands.rs:24-27`). Risk low, effort S.
8. **Derive slot count.** `COUNT` (`slot.rs:58`) becomes `ALL.len()`. Risk low, effort S.
9. **Replace the 20 ms read timeout.** `READ_TIMEOUT` (`host.rs:59`) with event-loop readiness, if the host runs on one [INFERENCE]. Risk medium, effort M.
10. **Record the ScreenCast version choice.** `SCREENCAST_VERSION` (`capture.rs:60`) reports 3 against spec 6 on purpose; write that decision next to the constant so it is not read as a bug. Risk low, effort S.

## 4. Constants to leave alone, and why

- **Wire framing and bounds:** `MAX_FRAME_LEN`, `LENGTH_PREFIX_LEN`, `MAGIC`, `HANDSHAKE_LEN`, `FLAG_SENSITIVE` (`frame.rs:21`, `:24`; `handshake.rs:27`, `:31`; `envelope.rs:27`). Wire ABI and denial-of-service bounds; changing them breaks peers or opens DoS.
- **Capability and device bits:** `capability.rs:82-120`, `capture.rs:41-51`, `remote.rs:35-39`. Fixed by the portal specification or our own wire ABI.
- **X11 and XKB modifier bits:** `keyboard.rs:124-142`. The X11 core state mask does not change.
- **Portal bus name and path:** `iface/mod.rs:58`, `:61`. Fixed by the portal dispatch contract.
- **Geometry and identity values:** `ZERO`, `ONE`, `EMPTY`, `IDENTITY`. Mathematical identities.
- **Spinner glyphs and SGR codes:** `theme.rs:96-104`, `:148`, `:152`. Terminal standards; the terminal theme already applies.
- **Environment variable names:** `GNOBLIN_IPC_SOCKET`, `XDG_RUNTIME_DIR`, `GNOBLIN_IPC_FD`, `GNOBLIN_READY_FD`, and the opt-in `GNOBLIN_SHELL_VM` (the shell VM's Lua directory; the VM opens its own owner-socket session by the `GNOBLIN_IPC_SOCKET` rule). Interface to launchers and supervisors.
- **Handle length bounds:** `MAX_HANDLE_BYTES`, `MAX_XID_DIGITS` (`parent_window.rs:25`, `:28`). Limits on untrusted input; the X11 width is fixed.

## 5. Needs Kieran

- **Consent timeout:** confirm `portal.consent.timeout` as a core config leaf (not Bingux-facing), default 120 s, range 10 s to 10 min.
- **Portal version reporting:** Screenshot is 2 but the spec is 3; ScreenCast is deliberately 3 against spec 6; Inhibit and GlobalShortcuts are at 1 against specs 3 and 2. Do you want full spec versions advertised, or are these deliberate lower caps? Bumping changes what clients negotiate.
- **Wire names from schema:** D16 says the wire is protobuf generated from the schema, but member and code strings are still hand-written. Confirm that generation should cover them.
- **Super, Hyper and Meta bits:** `keyboard.rs:144-152` should be checked against Clutter's header before any change. Confirm whether you want that check done.

## 6. Notes for the lead

- Counts differ from the brief because the brief appears to count `const fn` and test items; the figures here are item-level, outside test modules.
- Several verdicts are marked [INFERENCE] because the upstream source was not fetched in this session (Clutter header, xkbcommon docs, the 25 s portal timeout source).
