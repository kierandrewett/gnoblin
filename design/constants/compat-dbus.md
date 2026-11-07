# Constants audit: compat-dbus

Read-only audit of the `gnoblin-compat-dbus-*` crates under `crates/compat/`. No Rust was changed or built.

## Sources checked

- Local interface XML in each crate's `interfaces/` directory.
- Upstream Mutter 51.0 XML: `https://raw.githubusercontent.com/GNOME/mutter/138a14fbeef09d49ebf5be8a0cb83b042dd5c841/data/dbus-interfaces/<name>.xml`. GitLab returned 406, so the GitHub mirror was used. Mutter 51.0 is the version in `data/gnome-versions.json` (api "51").
- gnome-shell main: `https://raw.githubusercontent.com/GNOME/gnome-shell/main/data/dbus-interfaces/org.gnome.Shell.{Introspect,Screenshot}.xml`.

Result of the XML check:

- All six `org.gnome.Mutter.*` XML files are byte-identical to Mutter 51.0 (Clipboard, DisplayConfig, IdleMonitor, InputCapture, RemoteDesktop, ScreenCast).
- Both `org.gnome.Shell.*` XML files are byte-identical to gnome-shell main.

Key point: the XML is authoritative for names, paths, signatures and the `Version` property where one exists. The `VERSION` constants are hand-copied. Generating them from XML is only possible where the XML carries a `Version` property.

| Interface                      | XML has `Version` property | Local `VERSION` |
| ------------------------------ | -------------------------- | --------------- |
| org.gnome.Mutter.ScreenCast    | yes (XML line 40)          | 4               |
| org.gnome.Mutter.DisplayConfig | no                         | 3               |
| org.gnome.Mutter.IdleMonitor   | no (checked, not in XML)   | 1               |
| org.gnome.Mutter.InputCapture  | no                         | 1               |
| org.gnome.Mutter.RemoteDesktop | no                         | 1               |
| org.gnome.Mutter.Clipboard     | no                         | 1               |
| org.gnome.Shell.Introspect     | no                         | 3               |
| org.gnome.Shell.Screenshot     | no                         | 1               |

The Mutter `VERSION` values are the interface version the compat layer imitates. For the Mutter interfaces with a `Version` property in the XML, the value is the one Mutter 51.0 reports. For the others, the value is INFERENCE: it comes from the code comment, not from the XML. The `Version` property on gnome-shell Introspect and Screenshot is not in the XML, so their values (3 and 1) cannot be checked from the sources read here [INFERENCE].

## Summary per crate

Counts are for `const`/`static` items in non-test code. "Grouped" means a family listed as one row.

| Crate                                      | Const items | Listed individually | Grouped | Notes                                           |
| ------------------------------------------ | ----------- | ------------------- | ------- | ----------------------------------------------- |
| gnoblin-compat-dbus-gnome-session-manager  | 0           | 0                   | 0       | No constants in non-test code.                  |
| gnoblin-compat-dbus-gnome-shell-introspect | 9           | 6                   | 3       | Bus, path, interface, version, state keys.      |
| gnoblin-compat-dbus-gnome-shell-screenshot | 7           | 5                   | 2       | Bus, path, interface, version, refusal text.    |
| gnoblin-compat-dbus-mutter-clipboard       | 4           | 4                   | 0       | Name, interface, version, matrix include.       |
| gnoblin-compat-dbus-mutter-displayconfig   | 8           | 6                   | 2       | Bus, path, interface, version, mode separators. |
| gnoblin-compat-dbus-mutter-idlemonitor     | 6           | 6                   | 0       | Bus, path, interface, version, object names.    |
| gnoblin-compat-dbus-mutter-inputcapture    | 5           | 5                   | 0       | Interface names, version.                       |
| gnoblin-compat-dbus-mutter-inputmapping    | 0           | 0                   | 0       | No constants in non-test code.                  |
| gnoblin-compat-dbus-mutter-remotedesktop   | 5           | 5                   | 0       | Interface names, version.                       |
| gnoblin-compat-dbus-mutter-screencast      | 9           | 9                   | 0       | Bus, path, interfaces, version.                 |
| gnoblin-compat-dbus-mutter-servicechannel  | 0           | 0                   | 0       | No constants in non-test code.                  |

Crates with `support-matrix.json` pull it in with `include_str!`. Those are counted as one item each in the per-crate rows.

## Findings: D-Bus names, paths and interface names

Verdict for every row in this section: KEEP-as-protocol. These are the wire contract the compat layer imitates. Q1 (user configurable) is no: changing them would break every client that calls GNOME. Q3 (needed): yes, they are the public surface. Q2 (upstream change): yes, but the upstream source is the XML, so the right fix is a generator or a test, not a runtime lookup.

Risk: high for any change (clients break). Effort: S to add a test that diffs the constant against the XML.

| Constant                                                                            | Value                                                                          | Source                                                                           | Q2 authoritative source        | Verdict                                                      | Risk | Effort |
| ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ | -------------------------------------------------------------------------------- | ------------------------------ | ------------------------------------------------------------ | ---- | ------ |
| ScreenCast `BUS_NAME`, `PATH`, `INTERFACE`, `SESSION_INTERFACE`, `STREAM_INTERFACE` | `org.gnome.Mutter.ScreenCast` and friends                                      | crates/compat/gnoblin-compat-dbus-mutter-screencast/src/service.rs:56-65         | Mutter 51.0 XML. Matches.      | KEEP-as-protocol; DERIVE check via test                      | high | S      |
| DisplayConfig `BUS_NAME`, `PATH`, `INTERFACE`                                       | `org.gnome.Mutter.DisplayConfig`, `/org/gnome/Mutter/DisplayConfig`            | crates/compat/gnoblin-compat-dbus-mutter-displayconfig/src/service.rs:52-58      | Mutter 51.0 XML. Matches.      | KEEP-as-protocol; DERIVE check via test                      | high | S      |
| IdleMonitor `BUS_NAME`, `PATH`, `INTERFACE`                                         | `org.gnome.Mutter.IdleMonitor`, `/org/gnome/Mutter/IdleMonitor/Core`           | crates/compat/gnoblin-compat-dbus-mutter-idlemonitor/src/service.rs:51-57        | Mutter 51.0 XML. Matches.      | KEEP-as-protocol; DERIVE check via test                      | high | S      |
| InputCapture `INTERFACE`, `SESSION_INTERFACE`, `ROOT`, `SESSION`                    | `org.gnome.Mutter.InputCapture`, `.Session`                                    | crates/compat/gnoblin-compat-dbus-mutter-inputcapture/src/compat.rs:12-15,49-50  | Mutter 51.0 XML. Matches.      | KEEP-as-protocol                                             | high | S      |
| RemoteDesktop `INTERFACE`, `SESSION_INTERFACE`, `ROOT`, `SESSION`                   | `org.gnome.Mutter.RemoteDesktop`, `.Session`                                   | crates/compat/gnoblin-compat-dbus-mutter-remotedesktop/src/compat.rs:12-15,87-88 | Mutter 51.0 XML. Matches.      | KEEP-as-protocol                                             | high | S      |
| Clipboard `INTERFACE`                                                               | `org.gnome.Mutter.Clipboard`                                                   | crates/compat/gnoblin-compat-dbus-mutter-clipboard/src/compat.rs:12              | Mutter 51.0 XML. Matches.      | KEEP-as-protocol                                             | high | S      |
| Introspect `BUS_NAME`, `PATH`, `INTERFACE`                                          | `org.gnome.Shell`, `/org/gnome/Shell/Introspect`, `org.gnome.Shell.Introspect` | crates/compat/gnoblin-compat-dbus-gnome-shell-introspect/src/service.rs:48-54    | gnome-shell main XML. Matches. | KEEP-as-protocol                                             | high | S      |
| Screenshot `BUS_NAME`, `PATH`, `INTERFACE`                                          | `org.gnome.Shell.Screenshot`, `/org/gnome/Shell/Screenshot`                    | crates/compat/gnoblin-compat-dbus-gnome-shell-screenshot/src/service.rs:40-46    | gnome-shell main XML. Matches. | KEEP-as-protocol                                             | high | S      |
| `NAMES` arrays in each compat.rs                                                    | Lists of the bus names above                                                   | crates/compat/*/src/compat.rs (`NAMES`, lines 121/131/185/119)                   | Same as the rows above.        | DERIVE: build from the BUS_NAME constants, not a second copy | low  | S      |

Note on the Mutter `NAME` values (`mutter-screencast`, `mutter-displayconfig`, and so on) in each compat.rs: these are adapter identifiers, not D-Bus names. Verdict KEEP, risk low, effort S. Their list is not repeated per row.

## Findings: interface versions

Each `VERSION` is the value the compat layer reports from its `Version` property (or its matrix). Verdict for each: KEEP-as-protocol, with a note on how it could be generated. Q1 no. Q2 yes: a Mutter or gnome-shell release can bump the version. The authoritative source is the upstream release, and where the XML has a `Version` property, the XML is the source.

| Constant                | Value | Location                                                                   | Authoritative source                                                                                                              | Verdict                                                                                                                             | Risk | Effort |
| ----------------------- | ----- | -------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- | ---- | ------ |
| ScreenCast `VERSION`    | 4     | crates/compat/gnoblin-compat-dbus-mutter-screencast/src/service.rs:71      | Mutter 51.0 XML line 40 has the `Version` property; the value itself is not in the XML. Comment says "as Mutter 51.0 reports it". | KEEP-as-protocol; DERIVE: read it from a generated constant or a test that compares it to the Mutter version in gnome-versions.json | high | S      |
| DisplayConfig `VERSION` | 3     | crates/compat/gnoblin-compat-dbus-mutter-displayconfig/src/service.rs:61   | Not in the XML. Comment says "as Mutter 51.0 reports it" (src/service.rs:60). Value cannot be checked from the XML [INFERENCE].   | KEEP-as-protocol                                                                                                                    | high | S      |
| IdleMonitor `VERSION`   | 1     | crates/compat/gnoblin-compat-dbus-mutter-idlemonitor/src/service.rs:61     | Not in the XML.                                                                                                                   | KEEP-as-protocol                                                                                                                    | high | S      |
| InputCapture `VERSION`  | 1     | crates/compat/gnoblin-compat-dbus-mutter-inputcapture/src/compat.rs:18     | Not in the XML.                                                                                                                   | KEEP-as-protocol                                                                                                                    | high | S      |
| RemoteDesktop `VERSION` | 1     | crates/compat/gnoblin-compat-dbus-mutter-remotedesktop/src/compat.rs:18    | Not in the XML.                                                                                                                   | KEEP-as-protocol                                                                                                                    | high | S      |
| Clipboard `VERSION`     | 1     | crates/compat/gnoblin-compat-dbus-mutter-clipboard/src/compat.rs:15        | Not in the XML.                                                                                                                   | KEEP-as-protocol                                                                                                                    | high | S      |
| Introspect `VERSION`    | 3     | crates/compat/gnoblin-compat-dbus-gnome-shell-introspect/src/service.rs:57 | gnome-shell main XML has no `Version` property. Value is INFERENCE from the adapter's own comment.                                | KEEP-as-protocol; open question for Kieran (see Needs Kieran)                                                                       | high | S      |
| Screenshot `VERSION`    | 1     | crates/compat/gnoblin-compat-dbus-gnome-shell-screenshot/src/service.rs:50 | gnome-shell main XML has no `Version` property. [INFERENCE]                                                                       | KEEP-as-protocol                                                                                                                    | high | S      |

Mutter `VERSION` for DisplayConfig is set by the adapter's own comment to "as Mutter 51.0 reports it". The XML is identical to Mutter 51.0, so the wire surface matches the version the adapter claims. The version integer itself is not in the XML and cannot be checked from these sources.

## Findings: enum-like string values and separators

| Constant                                          | Value                            | Location                                                                       | Authoritative source                                                                      | Verdict                                       | Risk | Effort |
| ------------------------------------------------- | -------------------------------- | ------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------- | --------------------------------------------- | ---- | ------ |
| Display mode separators `REFRESH='@'`, `SIZE='x'` | `@`, `x`                         | crates/compat/gnoblin-compat-dbus-mutter-displayconfig/src/state.rs:40,43      | Mutter DisplayConfig mode id format (`WxH@R`). Matches the mode id convention in the XML. | KEEP-as-protocol                              | high | S      |
| Introspect state keys `window`, `output`, `lock`  | `"window"`, `"output"`, `"lock"` | crates/compat/gnoblin-compat-dbus-gnome-shell-introspect/src/state.rs:30,33,36 | Adapter's own object-type keys, matched to the compositor's `link` vocabulary.            | KEEP: internal                                | low  | S      |
| Screenshot `LOCK`                                 | `"lock"`                         | crates/compat/gnoblin-compat-dbus-gnome-shell-screenshot/src/link.rs:25        | Same vocabulary as above.                                                                 | DERIVE: share one const with introspect state | low  | S      |
| DisplayConfig `OUTPUT`                            | `"output"`                       | crates/compat/gnoblin-compat-dbus-mutter-displayconfig/src/state.rs:37         | Same vocabulary.                                                                          | DERIVE: share one const                       | low  | S      |
| IdleMonitor `IDLE`, `LOCK`                        | `"idle"`, `"lock"`               | crates/compat/gnoblin-compat-dbus-mutter-idlemonitor/src/link.rs:29,32         | Same vocabulary.                                                                          | DERIVE: share one const                       | low  | S      |

## Other constants

| Constant                                                                                                                                                                       | Value                                   | Location                                                                   | Verdict                                                                                           | Risk            | Effort |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------- | -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- | --------------- | ------ |
| displayconfig `CLIENT_VERSION`                                                                                                                                                 | 0.2.0                                   | crates/compat/gnoblin-compat-dbus-mutter-displayconfig/src/agent.rs:26     | Version of the compositor subscription protocol this adapter speaks. Internal, not a D-Bus value. | KEEP: internal  | low    | S   |
| idlemonitor `CLIENT_VERSION`                                                                                                                                                   | 0.2.0                                   | crates/compat/gnoblin-compat-dbus-mutter-idlemonitor/src/agent.rs:23       | Same.                                                                                             | KEEP: internal  | low    | S   |
| Screenshot `NO_CAPTURE`                                                                                                                                                        | prose message                           | crates/compat/gnoblin-compat-dbus-gnome-shell-screenshot/src/service.rs:53 | User-visible refusal text.                                                                        | KEEP; text only | low    | S   |
| Introspect `MATRIX`, Screenshot `MATRIX`, Displayconfig `MATRIX`, IdleMonitor `MATRIX`, Clipboard `MATRIX`, InputCapture `MATRIX`, RemoteDesktop `MATRIX`, ScreenCast `MATRIX` | `include_str!` of `support-matrix.json` | each crate's compat.rs                                                     | KEEP: build-time include                                                                          | low             | S      |

## Ranked wins

Ranked by user value over effort. Most of these are maintenance, not user settings: no compat-dbus constant is a user-facing knob.

1. Add a test that checks every `INTERFACE`, `PATH` and `BUS_NAME` constant against the matching XML in `interfaces/`. Catches upstream drift at build time. Effort S.
2. Derive the Mutter and gnome-shell `VERSION` values from one table keyed by Mutter version (gnome-versions.json), so the version and the XML cannot disagree. Effort S.
3. Share the `"lock"`, `"output"` and `"idle"` state keys in one module instead of four copies. Effort S.
4. Build each `NAMES` array from the `BUS_NAME` constants instead of repeating string literals. Effort S.
5. Document in the matrix file which upstream version each `VERSION` was read from, so the value has a source. Effort S.
6. Leave `CLIENT_VERSION` and `NAME` alone. They are internal identifiers and not worth changing.

## Constants to leave alone and why

- All D-Bus bus names, object paths and interface names: the wire contract with GNOME clients. Changing them breaks clients.
- Mode separators `@` and `x`: the mode id format clients parse.
- `MATRIX` `include_str!` items: build-time data.
- `NAME` identifiers: internal names used in support matrices.

## Needs Kieran

- Introspect and Screenshot `VERSION` values (3 and 1) are not in the gnome-shell XML. Is the adapter's comment the source of truth, or should a version table be checked against a real GNOME Shell 51 instance?
