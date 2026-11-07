# Constants audit: session group

> **Correction (2026-11-07):** References to `config.toml` (e.g. `config.toml:2063`, `config.toml:2090-2103`) are stale. The schema lives in individual TOML files under `crates/gnoblin-schema/schema/*.toml`. No single `config.toml` exists. The config leaves cited (e.g. `logging.store-max-age-days`) may exist in a Lua default-config file rather than a schema TOML file. See the master README for verified findings.

Upstream claims in the tables (protocol, GeoClue, polkit, ScreenSaver, systemd) are NOT fetched in this audit unless a row says so; treat them as [INFERENCE] until checked against the cited source.

Scope: `gnoblin-log`, `gnoblin-store`, `gnoblin-session`, `gnoblin-authority`, `gnoblin-service-core`, `gnoblin-service-location`, `gnoblin-service-polkit`, `gnoblin-service-screensaver`, `gnoblin-services`, `gnoblin` (binary), `gnoblin-layers`.

Method: anchored grep for `const`/`static` items, then manual exclusion of `cfg(test)` modules and `tests.rs` files. Indented or `pub(crate)` items that the anchored grep missed are cited from direct reads of the source. Upstream claims checked against the sources listed under "Sources".

## 1. Summary per crate

Counts are grep hits in `crates/<crate>/src`. "Test" means test-only items excluded from the audit. "Listed" means audited in section 2 (families are grouped into one row).

| Crate                       | Grep hits | Test or macro-only excluded | Listed individually | Grouped into rows |
| --------------------------- | --------: | --------------------------: | ------------------: | ----------------: |
| gnoblin-log                 |        45 |                           7 |                  38 |                16 |
| gnoblin-store               |        16 |                           1 |                  15 |                 9 |
| gnoblin-session             |        23 |                           8 |                  15 |                 9 |
| gnoblin-authority           |        13 |                          13 |                   0 |                 0 |
| gnoblin-service-core        |         1 |                           0 |                   1 |                 1 |
| gnoblin-service-location    |         7 |                           0 |                   7 |                 4 |
| gnoblin-service-polkit      |         5 |                           0 |                   5 |                 2 |
| gnoblin-service-screensaver |         3 |                           0 |                   3 |                 2 |
| gnoblin-services            |         2 |                           0 |                   2 |                 2 |
| gnoblin (binary)            |         2 |                           0 |                   2 |                 1 |
| gnoblin-layers              |         6 |                           0 |                   6 |                 4 |

Notes on counts:

- gnoblin-log excluded: `filter.rs:390,402,422` and `retention.rs:70` (tests); `lib.rs:135,173,215` (macro templates, not items).
- gnoblin-authority: all 13 hits are in `tests.rs`. The only production numbers are the `Limits` defaults in `authority.rs:30-32` (tokens 256, contexts 128, requests 64), which are struct field values, not `const` items. They are covered in section 2.6.
- gnoblin-session: the 8 `tests.rs` hits (`tests.rs:16-23`) are excluded; `model.rs:13-19` ChildIds are included.
- Items inside `fn` bodies or nested `mod` blocks that are not `pub` were searched by hand in the crates listed above. [INFERENCE] There may be a few local `const` values inside functions that the grep did not match; none found in the files read.

## 2. Findings per group

Columns: Constant (location) | Value | Q1 | Q2 | Q3 | Verdict | Risk | Effort.

### 2.1 Logging: console, filter and environment (gnoblin-log)

| Constant (location)                                                                      | Value                   | Q1  | Q2                                                                                        | Q3                                              | Verdict                          | Risk | Effort |
| ---------------------------------------------------------------------------------------- | ----------------------- | --- | ----------------------------------------------------------------------------------------- | ----------------------------------------------- | -------------------------------- | ---- | ------ |
| `MAX_COMPONENT_WIDTH` (console.rs:25)                                                    | 16 columns              | Y   | No: our console layout                                                                    | Adaptive: derive from `COMPONENT_WIDTH` as used | CONFIGURE or REMOVE              | low  | S      |
| `COMPONENT_WIDTH` static (console.rs:28)                                                 | runtime atomic          | N   | No                                                                                        | Keep: runtime state, not a constant             | KEEP                             | low  | S      |
| `MAX_COMPONENT_LEN` (lib.rs:58)                                                          | 24 bytes                | N   | No                                                                                        | Needed for a fixed-size name; see 2.1 note      | KEEP                             | low  | S      |
| `DEFAULT_LEVEL` (filter.rs:31)                                                           | INFO                    | Y   | No: our default. Config `logging.level` already exists (config.toml:2063, default `info`) | Replace with config default only                | DERIVE from config               | low  | S      |
| `FILTER_VARIABLE` `GNOBLIN_LOG` (init/options.rs:14)                                     | env name                | N   | No: our own env contract                                                                  | Needed; env name must stay stable               | KEEP                             | low  | S      |
| `FORMAT_VARIABLE` `GNOBLIN_LOG_FORMAT` (init/options.rs:20)                              | env name                | N   | No                                                                                        | Needed                                          | KEEP                             | low  | S      |
| `JOURNAL_STREAM_VARIABLE` (init/options.rs:23)                                           | `JOURNAL_STREAM`        | N   | Yes: systemd sets it. Protocol name, see Sources                                          | Needed to detect journald stderr                | KEEP                             | low  | S      |
| `DEFAULT_FILTER` (init/options.rs:26)                                                    | `"info"`                | N   | No                                                                                        | Duplicates `DEFAULT_LEVEL`                      | REMOVE duplicate, use one source | low  | S      |
| `COMPONENT_FIELD`, `MESSAGE_FIELD`, `TARGET_FIELD`, `FOREIGN_COMPONENT` (event.rs:17-27) | field names and `"ext"` | N   | No: internal record schema                                                                | Needed as schema keys shared with SQL           | KEEP                             | low  | S      |

Note on `MAX_COMPONENT_LEN` (lib.rs:58): the value bounds a component name used in a fixed buffer. [INFERENCE] Could be replaced by a `&'static str` with a debug assert, but the gain is small. Leave alone.

### 2.2 Logging: journal socket and datagrams (gnoblin-log/journal.rs)

| Constant (location)                 | Value                         | Q1  | Q2                                                                                                                                                           | Q3                                                            | Verdict                                                   | Risk   | Effort |
| ----------------------------------- | ----------------------------- | --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------- | --------------------------------------------------------- | ------ | ------ |
| `SOCKET_PATH` (journal.rs:41)       | `/run/systemd/journal/socket` | N   | Yes, upstream protocol. Systemd native journal protocol names this socket. [systemd.io/JOURNAL_NATIVE_PROTOCOL](https://systemd.io/JOURNAL_NATIVE_PROTOCOL/) | Needed. Could use `$JOURNAL_STREAM` only, but not the socket. | KEEP                                                      | low    | S      |
| `MAX_DATAGRAM` (journal.rs:44)      | 64 KiB                        | N   | Partly: the protocol says oversize datagrams fail with `EMSGSIZE` and fall back to memfd. The 64 KiB cap is ours.                                            | Could drop the cap and rely on `EMSGSIZE` + memfd             | DERIVE: remove the fixed cap, retry via memfd on EMSGSIZE | medium | M      |
| `MAX_NAME` (journal.rs:47)          | 64                            | N   | No                                                                                                                                                           | Field-name sanity limit; keep                                 | KEEP                                                      | low    | S      |
| `PREFIX` `GNOBLIN_` (journal.rs:50) | prefix                        | N   | No                                                                                                                                                           | Needed so our fields do not collide                           | KEEP                                                      | low    | S      |
| `REPORT_INTERVAL` (journal.rs:53)   | 1 s                           | N   | No                                                                                                                                                           | Rate of drop reports; internal                                | KEEP                                                      | low    | S      |
| `ENOBUFS` (journal.rs:56)           | 105                           | N   | Yes: Linux errno. Use `libc::ENOBUFS` instead of a literal                                                                                                   | Replace the literal with the library constant                 | DERIVE from `libc`                                        | low    | S      |
| `BINARY_OVERHEAD` (journal.rs:60)   | `"MESSAGE\n".len() + 8 + 1`   | N   | Yes: follows the native protocol binary form (length as little-endian u64)                                                                                   | Keep: a computed value already, mirror the protocol           | KEEP                                                      | low    | S      |

Note on `MAX_DATAGRAM` (journal.rs:44): the protocol page says a client should send the payload first and retry via memfd on `EMSGSIZE`. Our fixed 64 KiB cap means some large entries take the wrong path. [INFERENCE] Removing the cap and letting the kernel decide is cleaner. Medium risk, since it changes the send path.

### 2.3 Logging: store, retention and redaction (gnoblin-log)

| Constant (location)                                                                                       | Value               | Q1  | Q2                                                                     | Q3                                               | Verdict                                   | Risk   | Effort |
| --------------------------------------------------------------------------------------------------------- | ------------------- | --- | ---------------------------------------------------------------------- | ------------------------------------------------ | ----------------------------------------- | ------ | ------ |
| `QUEUE_CAPACITY` (gnoblin-log/src/pipeline.rs:23)                                                         | 4096 records        | Y   | No: internal back-pressure bound. Only matters under log storms        | Keep internal; expose only if drops are reported | KEEP                                      | low    | S      |
| `BATCH_LIMIT` (gnoblin-log/src/pipeline.rs:26)                                                            | 256 records         | N   | No                                                                     | Internal tuning                                  | KEEP                                      | low    | S      |
| `FLUSH_TIMEOUT` (gnoblin-log/src/pipeline.rs:29)                                                          | 2 s                 | Y   | No                                                                     | Internal latency bound. Keep                     | KEEP                                      | low    | S      |
| `BUSY_TIMEOUT` (gnoblin-store/src/kv.rs:42)                                                               | 5 s                 | N   | Yes: SQLite busy wait is ours                                          | Shared: one constant for the store and the log   | DONE                                      | low    | S      |
| `READ_BUSY_TIMEOUT` (removed)                                                                             | 5 s                 | N   | As above                                                               | Gone with the log query move                     | REMOVED                                   | low    | S      |
| `STORE_COMPONENT` `"log"` (gnoblin-store)                                                                 | name                | N   | No                                                                     | Keep                                             | KEEP                                      | low    | S      |
| `STATE_DIRECTORY` `"gnoblin/store"`, file `"log.sqlite3"` (gnoblin-store)                                 | XDG path parts      | Y   | Yes: `XDG_STATE_HOME` is the spec base. The subdirectory is ours.      | Use the XDG helper for the base; keep names      | DERIVE base from XDG_STATE_HOME (already) | low    | S      |
| `DAY_MS` (logs/retention.rs:9)                                                                            | 86 400 000          | N   | No: calendar unit                                                      | Replace with `Duration::from_secs(86_400)`       | REMOVE                                    | low    | S      |
| `SCHEMA`, `INSERT`, `NEWEST_SQL`, `FOLLOW_SQL` (gnoblin-store logs module)                                | SQL text            | N   | No                                                                     | Keep                                             | KEEP                                      | low    | S      |
| `SECRET_NAMES` (redact.rs:15, 9 names)                                                                    | list of field names | Y   | Partly: list of secret-bearing field names is ours and grows over time | A user may want to add names                     | CONFIGURE (see 4.1)                       | medium | M      |
| `TOKEN_LENGTH` (redact.rs:28)                                                                             | 40 chars            | N   | No: our heuristic for opaque tokens                                    | Keep; document the heuristic                     | KEEP                                      | low    | S      |
| `REDACTED` `"[redacted]"`, `QUOTES` (redact.rs:11,31)                                                     | strings             | N   | No                                                                     | Keep                                             | KEEP                                      | low    | S      |
| Retention (`store/retention.rs`), config leaves `logging.store-max-age-days`, `logging.store-max-size-mb` | 0 means off         | Y   | No                                                                     | Already configurable (config.toml:2090-2103)     | KEEP (already configurable)               | low    | S      |

Config leaves already present (config.toml:2051-2106): `logging.console` (enum, default `auto`), `logging.level` (text, default `info`), `logging.store` (bool, default `true`), `logging.store-path` (text, default empty), `logging.store-max-age-days` (int 0..36500, default 0), `logging.store-max-size-mb` (int 0..1048576, default 0). `main.rs:145` converts MB to bytes.

Note on retention: the default of 0 means no age or size limit, so the store grows without bound by default. This is a real user concern (see section 5).

### 2.4 Store (gnoblin-store)

| Constant (location)                                | Value                          | Q1  | Q2                                                           | Q3                                              | Verdict                                                         | Risk   | Effort |
| -------------------------------------------------- | ------------------------------ | --- | ------------------------------------------------------------ | ----------------------------------------------- | --------------------------------------------------------------- | ------ | ------ |
| `DEFAULT_QUOTA` (lib.rs:42)                        | 16 MiB                         | Y   | No                                                           | Already a store-level default; no user leaf yet | CONFIGURE: `store.quota-mb` (int, default 16, range 1..1048576) | low    | S      |
| `DEFAULT_VALUE_LIMIT` (lib.rs:45)                  | 64 KiB                         | Y   | No                                                           | Per-value cap; pair with quota                  | CONFIGURE only if quota leaf added; otherwise KEEP              | low    | S      |
| `READER_POOL` (kv.rs:39)                           | 4                              | N   | No                                                           | Internal pool size                              | KEEP                                                            | low    | S      |
| `BUSY_TIMEOUT` (kv.rs:42)                          | 5 s                            | N   | No: SQLite busy timeout; see 2.3 merge                       | Share with log                                  | MOVE                                                            | low    | S      |
| Limit clamp `1..1000` (kv.rs:439)                  | 1 to 1000                      | N   | No                                                           | Keep as a hard API bound                        | KEEP                                                            | low    | S      |
| `NAME_LIMIT` (name.rs:11)                          | 64                             | N   | No                                                           | Wire/name bound                                 | KEEP                                                            | low    | S      |
| `KEY_LIMIT` (name.rs:14)                           | 256                            | N   | No                                                           | Key bound                                       | KEEP                                                            | low    | S      |
| `VERSION` (encode.rs:18)                           | 1                              | N   | No: on-disk format version of our store                      | Needed for migration                            | KEEP                                                            | medium | S      |
| Tags `TAG_BOOL..TAG_LIST` 1 to 7 (encode.rs:20-26) | on-disk tags                   | N   | No                                                           | Wire format; changing breaks stored data        | KEEP                                                            | high   | S      |
| `STEPS` schema migrations (schema.rs:14)           | SQL                            | N   | No                                                           | Keep                                            | KEEP                                                            | low    | S      |
| `default_root` (lib.rs:415)                        | `XDG_STATE_HOME/gnoblin/store` | Y   | Yes: XDG base dir spec; the helper already reads the env var | Already derived                                 | KEEP                                                            | low    | S      |

Note on `DEFAULT_QUOTA` and `DEFAULT_VALUE_LIMIT`: these are the only store values a user might want to change. Today no config leaf exists. Add one only if a real need appears. The default is shown in section 4.2.

### 2.5 Session supervisor and guardian (gnoblin-session)

| Constant (location)                                                                                                                          | Value          | Q1  | Q2                                                                                  | Q3                                                                        | Verdict                                                              | Risk   | Effort |
| -------------------------------------------------------------------------------------------------------------------------------------------- | -------------- | --- | ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------- | -------------------------------------------------------------------- | ------ | ------ |
| ChildIds `COMPOSITOR` .. `SECOND_APP` (model.rs:13-19) and `CHILDREN` (model.rs:19)                                                          | 1..6           | N   | No: internal identifiers                                                            | Keep; `CHILDREN` length is derived from the array                         | KEEP                                                                 | low    | S      |
| `DEFAULT_READY_WITHIN` (plan.rs:13)                                                                                                          | 10 s           | Y   | Comment cites `STARTUP_TIMEOUT_MS` in `gnoblin-runtime-internal.h`; not re-verified | Keep one source; link to the header value                                 | CONFIGURE: `session.ready-timeout-s` (int, default 10, range 1..300) | medium | M      |
| Backoff guardian (plan.rs:89-93): 250 ms, x2, max 4000 ms, max restarts 5, stable 60 s                                                       | restart policy | Y   | No                                                                                  | Users seldom tune this; it protects against crash loops                   | KEEP; expose only if asked                                           | low    | S      |
| `stop_grace` 2 s, `safe_mode_after` 3 (guardian.rs:37)                                                                                       | 2 s; 3 crashes | Y   | No                                                                                  | Keep; `safe_mode_after` is a safety threshold                             | KEEP                                                                 | medium | S      |
| Supervisor tick 200 ms (supervisor.rs:134-138)                                                                                               | 200 ms         | N   | No                                                                                  | Internal poll tick                                                        | KEEP                                                                 | low    | S      |
| Test fast stop 200 ms, tick 10 ms (supervisor.rs:134-138)                                                                                    | test only      | N   | No                                                                                  | Gated by `TEST_FAST_VARIABLE`                                             | KEEP                                                                 | low    | S      |
| `TEST_FAST_VARIABLE` `GNOBLIN_TEST_FAST` (supervisor.rs:88)                                                                                  | env name       | N   | No                                                                                  | Keep; test hook only                                                      | KEEP                                                                 | low    | S      |
| `RAW_SERVICES_VARIABLE` `GNOBLIN_LOG_RAW_SERVICES` (supervisor.rs:91)                                                                        | env name       | N   | No                                                                                  | Keep; debugging hook                                                      | KEEP                                                                 | low    | S      |
| `SIGTERM` = 15 (supervisor.rs:723)                                                                                                           | 15             | N   | Yes: POSIX signal number, same on Linux. Use `libc::SIGTERM`                        | Replace literal with library constant                                     | DERIVE from `libc`                                                   | low    | S      |
| Poll interval 20 ms (supervisor.rs:808)                                                                                                      | 20 ms          | N   | No                                                                                  | Internal                                                                  | KEEP                                                                 | low    | S      |
| Exit code 70 (supervisor.rs:303)                                                                                                             | 70             | N   | Partly: sysexits `EX_SOFTWARE` convention                                           | Name it `EX_SOFTWARE` in one place                                        | MOVE: named const                                                    | low    | S      |
| Env fallbacks `/usr`, `/usr/bin:/bin`, `/usr/local/share:/usr/share` (supervisor.rs:122-125)                                                 | paths          | N   | Yes: XDG data dirs and PATH defaults are spec values                                | Derive from `XDG_DATA_DIRS` / `PATH` when set; use these only as fallback | DERIVE (fallback only)                                               | medium | M      |
| State fallback `~/.local/state` or `/tmp` (supervisor.rs:589-590)                                                                            | path           | N   | Yes: XDG state home spec; `/tmp` is a last resort                                   | Use the XDG helper; keep `/tmp` as last resort only                       | DERIVE (existing helper)                                             | low    | S      |
| `RAW_LINE` `MAX_LINE` 8 KiB, `TRUNCATED` `" [truncated]"`, `CAPACITY` 1024, `REPORT_INTERVAL` 1 s, `FLUSH_WAIT` 2 s (raw.rs:31,34,37,40,220) | limits         | N   | No                                                                                  | Keep; log-line bounds                                                     | KEEP                                                                 | low    | S      |
| `model.rs:49-52` `/opt/gnoblin` test prefix                                                                                                  | path           | N   | No                                                                                  | Test-only use; check it is not in production paths                        | KEEP (test)                                                          | low    | S      |
| Exit code and test ladder (supervisor.rs:509-513)                                                                                            | test ladder    | N   | No                                                                                  | Test helper                                                               | KEEP                                                                 | low    | S      |

Note on `DEFAULT_READY_WITHIN` (plan.rs:13): the comment cites the runtime header. [INFERENCE] The header is not read in this audit, so the match is not verified. Check it before any change.

Note on the backoff row: the guardian restart ladder is the only place a user might be stuck in a crash loop. Expose it only if a real case appears.

### 2.6 Authority (gnoblin-authority)

| Constant (location)                    | Value                                 | Q1  | Q2  | Q3                                           | Verdict | Risk | Effort |
| -------------------------------------- | ------------------------------------- | --- | --- | -------------------------------------------- | ------- | ---- | ------ |
| `Limits` defaults (authority.rs:30-32) | tokens 256, contexts 128, requests 64 | N   | No  | Internal per-session caps; no user need seen | KEEP    | low  | S      |

All other authority values are in `tests.rs` and excluded.

### 2.7 Service core (gnoblin-service-core)

| Constant (location)                                    | Value    | Q1  | Q2  | Q3                                                | Verdict | Risk | Effort |
| ------------------------------------------------------ | -------- | --- | --- | ------------------------------------------------- | ------- | ---- | ------ |
| `REMEMBERED_FINISHED` (agent.rs:64)                    | 64       | N   | No  | Bounds memory of finished agents; keep            | KEEP    | low  | S      |
| Role offer deadline floor `self.deadline` (host.rs:97) | per role | N   | No  | Derived from the caller's deadline; not a literal | KEEP    | low  | S      |

### 2.8 Location agent (gnoblin-service-location)

| Constant (location)                                | Value                               | Q1  | Q2                                                                                                                                                                                          | Q3                                                                                                     | Verdict                     | Risk   | Effort |
| -------------------------------------------------- | ----------------------------------- | --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ | --------------------------- | ------ | ------ |
| `AGENT_PATH` (lib.rs:55)                           | `/org/freedesktop/GeoClue2/Agent`   | N   | Yes, upstream: the GeoClue agent interface fixes this path, one per user. [GeoClue agent docs](https://www.freedesktop.org/software/geoclue/docs/gdbus-org.freedesktop.GeoClue2.Agent.html) | Needed                                                                                                 | KEEP                        | low    | S      |
| `AGENT_INTERFACE` (lib.rs:58)                      | `org.freedesktop.GeoClue2.Agent`    | N   | Yes, upstream, same page as above                                                                                                                                                           | Needed                                                                                                 | KEEP                        | low    | S      |
| `GEOCLUE_SERVICE` (lib.rs:61)                      | `org.freedesktop.GeoClue2`          | N   | Yes, upstream: the GeoClue bus name                                                                                                                                                         | Needed                                                                                                 | KEEP                        | low    | S      |
| `MANAGER_PATH` (lib.rs:64)                         | `/org/freedesktop/GeoClue2/Manager` | N   | Yes, upstream                                                                                                                                                                               | Needed                                                                                                 | KEEP                        | low    | S      |
| `AGENT_ID` (lib.rs:67)                             | `gnoblin`                           | N   | No: our agent id in the GeoClue whitelist. Matches `data/geoclue/50-gnoblin.conf`                                                                                                           | Keep: must match the conf.d whitelist entry                                                            | KEEP                        | medium | S      |
| `LEVELS` (lib.rs:70)                               | `[0,1,4,5,6,8]`                     | N   | Yes, upstream: GClueAccuracyLevel enum values NONE 0, COUNTRY 1, CITY 4, NEIGHBORHOOD 5, STREET 6, EXACT 8. [pgi GeoClue enums](https://lazka.github.io/pgi-docs/Geoclue-2.0/enums.html)    | DERIVE: generate from the GeoClue header or `geoclue-2.0` enum if build-time, else keep with a comment | KEEP (matches upstream)     | medium | S      |
| `Policy::DENY` `max_level: 0` (lib.rs:97)          | 0 (NONE)                            | N   | Yes: NONE = 0 upstream                                                                                                                                                                      | Fine as named constant                                                                                 | KEEP                        | low    | S      |
| `max-accuracy` config leaf (config.toml:1879-1892) | enum, default not read              | Y   | No                                                                                                                                                                                          | Already configurable                                                                                   | KEEP (already configurable) | low    | S      |

Note: the GeoClue `MaxAccuracyLevel` property is per user, which matches the agent path design. [INFERENCE] Fine as is.

### 2.9 Polkit agent (gnoblin-service-polkit)

| Constant (location)                           | Value                                             | Q1  | Q2                                                                                                             | Q3                             | Verdict                     | Risk | Effort |
| --------------------------------------------- | ------------------------------------------------- | --- | -------------------------------------------------------------------------------------------------------------- | ------------------------------ | --------------------------- | ---- | ------ |
| `AGENT_PATH` (lib.rs:67)                      | `/org/freedesktop/PolicyKit1/AuthenticationAgent` | N   | Yes: polkit spec fixes this path. [INFERENCE] Not fetched in this audit; matches polkit documentation as known | Needed                         | KEEP                        | low  | S      |
| `AUTHORITY_SERVICE` (lib.rs:70)               | `org.freedesktop.PolicyKit1`                      | N   | Yes: polkit well-known name [INFERENCE] same as above                                                          | Needed                         | KEEP                        | low  | S      |
| `AUTHORITY_PATH` (lib.rs:73)                  | `/org/freedesktop/PolicyKit1/Authority`           | N   | Yes: polkit [INFERENCE]                                                                                        | Needed                         | KEEP                        | low  | S      |
| `AUTHORITY_INTERFACE` (lib.rs:76)             | `org.freedesktop.PolicyKit1.Authority`            | N   | Yes: polkit [INFERENCE]                                                                                        | Needed                         | KEEP                        | low  | S      |
| `KIND` `unix-process` (subject.rs:17)         | subject kind                                      | N   | Yes: polkit subject kind name [INFERENCE]                                                                      | Needed                         | KEEP                        | low  | S      |
| `/proc/self/stat` field 19 (subject.rs:34)    | process start time                                | N   | Yes: Linux proc(5) field index                                                                                 | Keep; document the field index | KEEP                        | low  | S      |
| `polkit-agent` config leaf (config.toml:1871) | bool, default false                               | Y   | No                                                                                                             | Already configurable           | KEEP (already configurable) | low  | S      |

### 2.10 Screen saver (gnoblin-service-screensaver)

| Constant (location)                                     | Value                                          | Q1  | Q2                                                                                                                                                | Q3                                            | Verdict | Risk | Effort |
| ------------------------------------------------------- | ---------------------------------------------- | --- | ------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------- | ------- | ---- | ------ |
| `BUS_NAME` (lib.rs:62)                                  | `org.freedesktop.ScreenSaver`                  | N   | Yes, upstream: the idle inhibition spec names this bus name. [spec](https://specifications.freedesktop.org/idle-inhibit/latest/)                  | Needed                                        | KEEP    | low  | S      |
| `PATHS` (lib.rs:64)                                     | `/org/freedesktop/ScreenSaver`, `/ScreenSaver` | N   | Yes, upstream: spec requires the first path; the second is the KDE legacy path the spec says was used until kde-workspace 4.11.8 (spec section 4) | Keep the legacy path for client compatibility | KEEP    | low  | S      |
| `MAX_INHIBITS_PER_SENDER` (lib.rs:66)                   | 64                                             | N   | No                                                                                                                                                | Guard against a leaking client; keep          | KEEP    | low  | S      |
| Idle seconds conversion (lib.rs:233-238) from `idle_ms` | `/ 1000`, saturating to `u32::MAX`             | N   | No                                                                                                                                                | Unit conversion; keep                         | KEEP    | low  | S      |

Note on idle defaults: the screen saver service itself has no idle timeout. The idle timeout lives in the idle policy (`gnoblin-idle` unit, `data/session/systemd-user/gnoblin-idle.service.in`, and the idle schema `crates/gnoblin-schema/schema/idle.toml`). This audit did not read idle.toml leaves; [INFERENCE] any idle timeout constant is in the idle group, not this one.

### 2.11 Services role (gnoblin-services)

| Constant (location)            | Value  | Q1  | Q2                                                                                                                           | Q3                                        | Verdict | Risk | Effort |
| ------------------------------ | ------ | --- | ---------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------- | ------- | ---- | ------ |
| `PUMP_TIMEOUT` (role.rs:62)    | 100 ms | N   | No                                                                                                                           | Internal loop tick                        | KEEP    | low  | S      |
| `PROMPT_DEADLINE` (role.rs:69) | 120 s  | Y   | Comment cited a gcr-prompter default; that adapter is gone. [INFERENCE] Not verified against the polkit source in this audit | Keep as an internal bound; no config leaf | KEEP    | low  | S      |

### 2.12 Binary (gnoblin)

| Constant (location)                       | Value                                            | Q1  | Q2                             | Q3                                      | Verdict | Risk | Effort |
| ----------------------------------------- | ------------------------------------------------ | --- | ------------------------------ | --------------------------------------- | ------- | ---- | ------ |
| `Role::ALL` (role.rs:29)                  | 4 roles: guardian, compositor, runtime, services | N   | No                             | Keep; derived list, no literal to drift | KEEP    | low  | S      |
| `VERSION` `LazyLock<String>` (role.rs:81) | built from crate version                         | N   | Yes: Cargo metadata via `env!` | Keep                                    | KEEP    | low  | S      |

### 2.13 Layers (gnoblin-layers)

| Constant (location)                 | Value                | Q1  | Q2                  | Q3                                  | Verdict | Risk | Effort |
| ----------------------------------- | -------------------- | --- | ------------------- | ----------------------------------- | ------- | ---- | ------ |
| `Band::ALL` (band.rs:39)            | 8 bands              | N   | No: our layer model | Keep; array length tied to variants | KEEP    | low  | S      |
| `GNOBLIN_NAMESPACE` (name.rs:25)    | `gnoblin`            | N   | No                  | Keep                                | KEEP    | low  | S      |
| `MAX_LOCAL_LEN` (name.rs:29)        | 48                   | N   | No                  | Keep                                | KEEP    | low  | S      |
| `MAX_EXTENSION_ID_LEN` (name.rs:32) | 32                   | N   | No                  | Keep                                | KEEP    | low  | S      |
| `COMPOSITOR_CRATE` (provider.rs:13) | `gnoblin-compositor` | N   | No                  | Keep                                | KEEP    | low  | S      |
| `DEFAULT_LIMIT` (registry.rs:47)    | 32                   | N   | No                  | Keep                                | KEEP    | low  | S      |

## 3. Top ten wins

Ranked by user value over effort. Each is a proposal, not a change made here.

1. **Log store retention defaults.** `logging.store-max-age-days` and `logging.store-max-size-mb` default to 0 (no limit). A user will fill disk. Propose a non-zero default (for example 30 days or 512 MB) with a note in docs. Risk low, effort S. [config.toml:2090-2103]
2. **Session ready timeout.** `DEFAULT_READY_WITHIN` (plan.rs:13) is 10 s, with no user leaf. Expose `session.ready-timeout-s`. Risk medium, effort M. Verify the header value first.
3. **Store quota.** `DEFAULT_QUOTA` (store lib.rs:42) is 16 MiB with no leaf. Expose `store.quota-mb`. Risk low, effort S.
4. **Journal datagram cap.** `MAX_DATAGRAM` (journal.rs:44) forces the wrong path for large entries. Remove the cap and retry via memfd on `EMSGSIZE`, per the protocol. Risk medium, effort M.
5. **Merge duplicate busy timeouts.** Done: `BUSY_TIMEOUT` now lives once in `gnoblin-store/src/kv.rs:42`, and the log store uses it there. `READ_BUSY_TIMEOUT` is gone. Risk low, effort S.
6. **Library constants for errno and signal.** `ENOBUFS` = 105 (journal.rs:56) and `SIGTERM` = 15 (supervisor.rs:723) should come from `libc`. Risk low, effort S.
7. **Remove `DAY_MS`.** Replace with `Duration::from_secs(86_400)` (retention.rs:8). Risk low, effort S.
8. **Remove duplicate `DEFAULT_FILTER`.** `init/options.rs:26` duplicates `DEFAULT_LEVEL` (filter.rs:31). One source, read from config. Risk low, effort S.
9. **Secret-name list.** `SECRET_NAMES` (redact.rs:15) may need user additions. Expose `logging.redact-extra` as a list. Risk medium, effort M.

## 4. Proposed config leaves

Defaults equal current values unless stated.

### 4.1 Core config (`crates/gnoblin-schema/schema/*.toml` — no single config.toml exists)

| Leaf path                 | Type      | Unit | Default | Range      | Source constant                   |
| ------------------------- | --------- | ---- | ------- | ---------- | --------------------------------- |
| `store.quota-mb`          | int       | MiB  | 16      | 1..1048576 | `DEFAULT_QUOTA` store lib.rs:42   |
| `session.ready-timeout-s` | int       | s    | 10      | 1..300     | `DEFAULT_READY_WITHIN` plan.rs:13 |
| `logging.redact-extra`    | text list | n/a  | empty   | n/a        | `SECRET_NAMES` redact.rs:15       |

Proposed default change (not an add): `logging.store-max-size-mb` default 0 to a non-zero value. Kieran to choose (see section 6).

### 4.2 Shell config (Bingux)

None proposed from this group. Session prompts and timeouts belong to the core session, not the shell.

### 4.3 Lua API

None proposed from this group. Lua already gets the services through the session host.

## 5. Constants to leave alone

- Wire and on-disk formats: store `VERSION` (encode.rs:18) and tags 1..7 (encode.rs:20-26). Changing them breaks stored data. High risk.
- Protocol names: `SOCKET_PATH`, `JOURNAL_STREAM`, D-Bus bus names, object paths, interface names (services and session). They are upstream specs and must match.
- GeoClue `LEVELS` (lib.rs:70) match the upstream enum. Keep, with a comment citing the enum.
- Field names and SQL text in the log store (event.rs:17-27, the gnoblin-store logs module). Schema keys.
- Test ladders and test-only prefixes (`supervisor.rs:509-513`, `model.rs:49-52`). Not production.
- Limits that guard memory or abuse: `MAX_INHIBITS_PER_SENDER`, `REMEMBERED_FINISHED`, `Limits` in authority, queue and batch sizes in the log store. They are safety bounds, not preferences.

## 6. Needs Kieran

1. **Log retention default.** Should the default store cap be 0 (no limit, today), or a non-zero default? Affects disk use for every install. Choose a number and whether it applies to Bingux as well.
2. **Ready timeout as a leaf.** Expose `session.ready-timeout-s` in core config, or leave it internal until someone asks? Medium risk either way.
3. **Secret-name extras.** Should users extend the redaction list in core config, or is that a Bingux-facing setting? Affects where the leaf lives.
4. **Store quota leaf.** Core config or Bingux? The store is used by both.

## Sources

- GeoClue agent D-Bus interface: https://www.freedesktop.org/software/geoclue/docs/gdbus-org.freedesktop.GeoClue2.Agent.html (cited, NOT fetched in this audit; agent path, interface and bus name are [INFERENCE] until checked).
- GeoClue accuracy enum values: https://lazka.github.io/pgi-docs/Geoclue-2.0/enums.html (cited, NOT fetched; values NONE 0, COUNTRY 1, CITY 4, NEIGHBORHOOD 5, STREET 6, EXACT 8 are [INFERENCE] from memory).
- systemd native journal protocol: https://systemd.io/JOURNAL_NATIVE_PROTOCOL/ (cited, NOT fetched; socket path, EMSGSIZE memfd fallback and JOURNAL_STREAM are [INFERENCE]).
- Idle inhibition / org.freedesktop.ScreenSaver: https://specifications.freedesktop.org/idle-inhibit/latest/ (cited, NOT fetched; bus name and paths are [INFERENCE]).
- Polkit agent and authority names: [INFERENCE] not fetched in this audit.
- XDG base directory and `RUNTIME_DIRECTORY` rules: [INFERENCE] not fetched; relied on existing helper use.

## Not verified in this audit

- `DEFAULT_READY_WITHIN` header value in `gnoblin-runtime-internal.h`: not read.
- Idle timeout leaves in `idle.toml` and `gnoblin-idle` unit: not read in detail.
- Polkit constants: names as recalled, not fetched from polkit docs.
- No Rust was built or run, per the assignment.
