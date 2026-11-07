# Constants audit: compat-other

Read-only audit of the compat crates outside the `gnoblin-compat-dbus-*` family. No Rust was changed and nothing was built. Each row cites `crates/<crate>/src/<file>.rs:<line>`. Claims about upstream behaviour cite the source checked.

Sources checked:

- XSETTINGS spec, type ids: https://specifications.freedesktop.org/xsettings/latest/ (0 Integer, 1 String, 2 Color).
- GtkSettings `gtk-xft-dpi` is in 1024ths of a DPI: https://docs.gtk.org/gtk4/property.Settings.gtk-xft-dpi.html
- GNOME interface schema (`org.gnome.desktop.interface`): https://raw.githubusercontent.com/GNOME/gsettings-desktop-schemas/master/schemas/org.gnome.desktop.interface.gschema.xml.in
- GNOME wm preferences schema (`org.gnome.desktop.wm.preferences`): https://raw.githubusercontent.com/GNOME/gsettings-desktop-schemas/master/schemas/org.gnome.desktop.wm.preferences.gschema.xml.in
- Config leaves: `crates/gnoblin-schema/schema/settings.toml` and `output.toml`.

## 1. Summary per crate

Counts are `const`/`static` items only. Functions marked `const fn` are not constants and are not counted. Test-only values are excluded.

| Crate                                     | Const/static items | Listed individually | Grouped | Notes                                                                                        |
| ----------------------------------------- | ------------------ | ------------------- | ------- | -------------------------------------------------------------------------------------------- |
| gnoblin-compat-core                       | 1                  | 1                   | 0       | Value defaults in `DesktopSettings::default` (settings.rs:187-198) are listed in section 2.1 |
| gnoblin-compat-host                       | 0                  | 0                   | 0       | Only test-scope timeouts and paths (compat.rs, settings.rs); excluded                        |
| gnoblin-compat-x11-xsettings              | 12                 | 12                  | 0       | Names and xdpi scale (lib.rs:47-91)                                                          |
| gnoblin-compat-wayland-gtk-shell1         | 5                  | 4                   | 1       | NAME, MATRIX, SHELL, SURFACE grouped as names; SHELL_VERSION listed                          |
| gnoblin-compat-wayland-mutter-x11-interop | 4                  | 3                   | 1       | NAME, INTERFACE, INTEROP_VERSION listed; MATRIX grouped                                      |
| gnoblin-compat-gtk-dconf-settings         | 5                  | 3                   | 2       | Paths, schema groups and key names (lib.rs:63-182, agent.rs:18-21)                           |
| gnoblin-compat-qt-portal-settings         | 3                  | 2                   | 1       | PLATFORM_THEME and PORTAL_THEME listed; MATRIX grouped                                       |

Total: 30 const/static items across 7 crates. Every item is covered below.

`MATRIX` (`include_str!` of `support-matrix.json`) appears in five crates. It is a build-time embed, so it is KEEP everywhere. It is not a tunable.

## 2. Findings

### 2.1 Desktop setting defaults (gnoblin-compat-core)

| Value                                           | Current             | Verdict | Q2: upstream today                                                                                    | Risk   | Effort |
| ----------------------------------------------- | ------------------- | ------- | ----------------------------------------------------------------------------------------------------- | ------ | ------ |
| gtk_theme, icon_theme, cursor_theme `"Adwaita"` | settings.rs:189-194 | DERIVE  | GNOME `interface` schema default is `'Adwaita'` for gtk-theme, icon-theme and cursor-theme (checked). | low    | S      |
| font `"Cantarell 11"`                           | settings.rs:191     | DERIVE  | GNOME schema default is `'Adwaita Sans 11'` (checked). Ours is out of date.                           | medium | S      |
| monospace_font `"Source Code Pro 10"`           | settings.rs:192     | DERIVE  | GNOME schema default is `'Adwaita Mono 11'` (checked). Ours is out of date.                           | medium | S      |
| titlebar_font `"Cantarell Bold 11"`             | settings.rs:193     | DERIVE  | wm `titlebar-font` default is `'Adwaita Sans Bold 11'` (checked). Ours is out of date.                | medium | S      |
| cursor_size `24`                                | settings.rs:195     | DERIVE  | GNOME `cursor-size` default is 24 (checked). Matches.                                                 | low    | S      |
| dpi `96`                                        | settings.rs:198     | KEEP    | Toolkits assume 96 DPI at scale 1 (settings.rs:171-172). This is a spec-level convention.             | low    | S      |

What to do: the defaults are a second copy of the desktop's defaults. They should come from the Gnoblin settings model, which is `crates/gnoblin-schema/schema/settings.toml`, so the compat layer reads what the settings object already holds. The fallback values should match current GNOME. The table above shows Cantarell and Source Code Pro are stale.

Config leaves that already exist in `settings.toml`: `gtk_theme`, `icon_theme`, `cursor_theme`, `cursor_size`, `font_name`, `document_font_name`, `monospace_font_name`, `text_scaling_factor`, `button_layout`, `color_scheme`, `accent_red/green/blue`, `contrast`, `reduced_motion`. Already configurable: all of the values above except `titlebar_font` and `dpi`.

Gap: there is no leaf for `titlebar_font`. Add `settings.titlebar_font_name` (string, default `Adwaita Sans Bold 11`) to `settings.toml`. Type: string. Unit: a Pango font description. Belongs in core config. Risk low, effort S.

### 2.2 DPI and scale (gnoblin-compat-core, x11-xsettings)

| Value                               | Location                   | Verdict | Q1 / Q2 / Q3                                                                                                                                                           | Risk | Effort |
| ----------------------------------- | -------------------------- | ------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---- | ------ |
| `dpi_for_scale` clamp `48.0..768.0` | settings.rs:175-180        | KEEP    | Q1 no, a guard against a nonsensical scale. Q3: the bound stops `as u32` overflow. Could move into `Scale` validation.                                                 | low  | S      |
| `96.0 * scale`                      | settings.rs:176            | KEEP    | Spec-level convention (settings.rs:171-172).                                                                                                                           | low  | S      |
| `xdpi` multiplier `1024`            | x11-xsettings lib.rs:88-91 | KEEP    | XSETTINGS/GTK store DPI in 1024ths. Checked against https://docs.gtk.org/gtk4/property.Settings.gtk-xft-dpi.html. The saturating multiply states the 32-bit invariant. | low  | S      |
| `Xft/DPI` name                      | lib.rs:53                  | KEEP    | De-facto name read by GTK and Xft (XSETTINGS-compatible convention). [INFERENCE] not in the XSETTINGS spec itself.                                                     | low  | S      |

No `dpi` config leaf exists. DPI is derived from the output scale. That is correct: a separate DPI knob would be a second source of truth. Do not add one (Q3). Q2: if a user wants a fixed DPI, the Gnoblin answer is the output `scale` leaf (`output.toml`), not a DPI leaf.

### 2.3 XSETTINGS wire names and type ids (gnoblin-compat-x11-xsettings)

| Constant                                             | Line           | Verdict | Q2 finding                                                                                                                       | Risk | Effort |
| ---------------------------------------------------- | -------------- | ------- | -------------------------------------------------------------------------------------------------------------------------------- | ---- | ------ |
| `INTERFACE` `_XSETTINGS_S0`                          | lib.rs:50      | KEEP    | Selection name is `_XSETTINGS_Sn` per screen; spec-defined. Screen 0 is correct only for the single Xwayland screen. [INFERENCE] | low  | S      |
| `FONT` `Gtk/FontName`                                | lib.rs:56      | KEEP    | Gtk/ prefix is an XSETTINGS convention. Value comes from `font_name`.                                                            | low  | S      |
| `MONOSPACE_FONT` `Gtk/MonospaceFontName`             | lib.rs:59      | KEEP    | As above.                                                                                                                        | low  | S      |
| `THEME` `Net/ThemeName`, `GTK_THEME` `Gtk/ThemeName` | lib.rs:62-65   | KEEP    | `Net/` names are spec-reserved. Both are needed: toolkits read different ones.                                                   | low  | S      |
| `ICON_THEME` / `GTK_ICON_THEME`                      | lib.rs:68-71   | KEEP    | Same split as the theme names.                                                                                                   | low  | S      |
| `CURSOR_THEME` `Gtk/CursorThemeName`                 | lib.rs:74      | KEEP    | Gtk/ convention.                                                                                                                 | low  | S      |
| `CURSOR_SIZE` `Gtk/CursorThemeSize`                  | lib.rs:77      | KEEP    | Gtk/ convention. Value is `cursor_size`, an existing leaf.                                                                       | low  | S      |
| `BUTTON_LAYOUT` `Gtk/DecorationLayout`               | lib.rs:80      | KEEP    | Gtk/ convention. Value is `button_layout`, an existing leaf.                                                                     | low  | S      |
| Type ids 0 Integer, 1 String                         | enum `Setting` | KEEP    | Spec gives 0 Integer, 1 String, 2 Color (checked). Colour is not used.                                                           | low  | S      |

XSETTINGS names are wire protocol and are KEEP. Adding a name would be a new XSETTINGS key, not a setting.

### 2.4 GTK shell protocol (gnoblin-compat-wayland-gtk-shell1)

| Constant                                       | Line             | Verdict | Q2 finding                                                                                                                                                                          | Risk   | Effort |
| ---------------------------------------------- | ---------------- | ------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ------ |
| `SHELL_VERSION` `7`                            | global.rs:47     | DERIVE  | `gtk-shell.xml` declares `gtk_shell1` version 7 (protocols/gtk-shell.xml:35) and `gtk_surface1` version 7 (line 72). Derive the number from the vendored XML so a bump is one edit. | medium | S      |
| `NAME` `gtk-shell1`                            | compat.rs:28     | KEEP    | Compat adapter name used in the support matrix. Internal.                                                                                                                           | low    | S      |
| `SHELL` `gtk_shell1`, `SURFACE` `gtk_surface1` | compat.rs:34, 37 | KEEP    | Interface names from the XML (lines 35, 72). Keep as constants, or read from generated code.                                                                                        | low    | S      |

Q2: GTK's shell protocol is not in wayland-protocols. The vendored XML comes from Mutter (protocols/gtk-shell.xml:1-8). A version bump must follow Mutter's XML. Q3: a test that asserts `SHELL_VERSION == generated version` would remove the duplicate. The generator emits interface versions; [INFERENCE] the exact generated symbol needs checking before the change.

### 2.5 Mutter X11 interop (gnoblin-compat-wayland-mutter-x11-interop)

| Constant                         | Line         | Verdict | Q2 finding                                                                                                                | Risk   | Effort |
| -------------------------------- | ------------ | ------- | ------------------------------------------------------------------------------------------------------------------------- | ------ | ------ |
| `INTEROP_VERSION` `1`            | global.rs:23 | DERIVE  | `mutter-x11-interop.xml` declares version 1 (protocols/mutter-x11-interop.xml:8). Derive from the XML, as for gtk-shell1. | medium | S      |
| `NAME` `mutter-x11-interop`      | compat.rs:17 | KEEP    | Adapter name. Internal.                                                                                                   | low    | S      |
| `INTERFACE` `mutter_x11_interop` | compat.rs:23 | KEEP    | Equal to the XML protocol and interface name (protocols/mutter-x11-interop.xml:2, 8).                                     | low    | S      |

### 2.6 GTK dconf layer (gnoblin-compat-gtk-dconf-settings)

Keys below were checked against the GNOME schemas listed at the top. Each row verifies the key name, type and default that GTK reads.

| Key                   | Schema and group                   | Type                         | Our source                             | Verdict                           | Risk   | Effort |
| --------------------- | ---------------------------------- | ---------------------------- | -------------------------------------- | --------------------------------- | ------ | ------ |
| `font-name`           | `org/gnome/desktop/interface`      | s                            | `font_name` leaf                       | KEEP name; DERIVE default         | low    | S      |
| `monospace-font-name` | interface                          | s                            | `monospace_font_name` leaf             | KEEP                              | low    | S      |
| `icon-theme`          | interface                          | s                            | `icon_theme` leaf                      | KEEP                              | low    | S      |
| `cursor-theme`        | interface                          | s                            | `cursor_theme` leaf                    | KEEP                              | low    | S      |
| `cursor-size`         | interface                          | i                            | `cursor_size` leaf                     | KEEP                              | low    | S      |
| `color-scheme`        | interface                          | enum (`GDesktopColorScheme`) | `color_scheme` leaf, written as text   | KEEP                              | medium | S      |
| `accent-color`        | interface                          | enum (`GDesktopAccentColor`) | `accent_*` leaves, written as `name()` | KEEP                              | medium | S      |
| `titlebar-font`       | `org/gnome/desktop/wm/preferences` | s                            | `titlebar_font` (no leaf)              | KEEP name; add leaf (section 2.1) | low    | S      |
| `button-layout`       | wm/preferences                     | s                            | `button_layout` leaf                   | KEEP                              | low    | S      |

Evidence: GNOME interface schema lists `cursor-size` as type `i`, `font-name`, `icon-theme`, `cursor-theme` as `s`, and `color-scheme` / `accent-color` as enums. Wm schema lists `titlebar-font` and `button-layout` as `s`. Full URLs are at the top.

Group paths `org/gnome/desktop/interface` (lib.rs:66) and `org/gnome/desktop/wm/preferences` (lib.rs:69) are the real schema paths. KEEP.

Q2 finding: key defaults drift upstream (section 2.1). The keys themselves are stable. Config-side the leaves already map 1:1 except `titlebar_font`.

| Constant                          | Line          | Verdict | Notes                                                                                  |
| --------------------------------- | ------------- | ------- | -------------------------------------------------------------------------------------- |
| `INTERFACE`, `WINDOW_PREFERENCES` | lib.rs:66, 69 | KEEP    | Real GSettings schema paths.                                                           |
| `PROFILE` `profile`               | agent.rs:18   | KEEP    | dconf profile name, internal file name. Path is under `XDG_RUNTIME_DIR` (agent.rs:29). |
| `DATABASE` `gnoblin.db`           | agent.rs:21   | KEEP    | Internal file name.                                                                    |

`XDG_RUNTIME_DIR` (agent.rs:29) is the XDG base-dir variable and is read at runtime: already DERIVE.

### 2.7 Qt portal (gnoblin-compat-qt-portal-settings)

| Constant                                | Line      | Verdict | Q2 finding                                                                                                  | Risk | Effort |
| --------------------------------------- | --------- | ------- | ----------------------------------------------------------------------------------------------------------- | ---- | ------ |
| `PLATFORM_THEME` `QT_QPA_PLATFORMTHEME` | lib.rs:43 | KEEP    | Qt's environment variable name for the platform theme plugin. [INFERENCE] stable across Qt 5 and 6.         | low  | S      |
| `PORTAL_THEME` `xdgdesktopportal`       | lib.rs:46 | KEEP    | Plugin name Qt uses to select the portal theme. [INFERENCE] not checked against qtbase source in this pass. | low  | S      |

Q3: nothing is configurable here, so none of this is a user setting. Qt reads its settings from the portal, so there is no Qt leaf to add.

### 2.8 Core and host

| Constant                                                       | Line                                      | Verdict           | Notes                                                                                            |
| -------------------------------------------------------------- | ----------------------------------------- | ----------------- | ------------------------------------------------------------------------------------------------ |
| `Mode::UNSANDBOXED`                                            | mode.rs:161                               | KEEP              | Enum-shaped value, not a tunable.                                                                |
| Host tests (`/run/user/1000/...`, sleeps 10s, 20ms, 30s, 50ms) | host compat.rs:101-357, settings.rs:77-91 | KEEP (test scope) | Test-only. Excluded from the count. Hard-coded uid 1000 is a test smell, not a product constant. |

## 3. Top ten wins

Ranked by user value over effort.

1. Replace stale fallback fonts (`Cantarell 11`, `Source Code Pro 10`, `Cantarell Bold 11`) with values read from the Gnoblin settings model. Users get current GNOME defaults. Effort S.
2. Add `settings.titlebar_font_name` leaf so `titlebar-font` is configurable. Effort S.
3. Derive `SHELL_VERSION` from the vendored `gtk-shell.xml` instead of a second copy. Effort S.
4. Derive `INTEROP_VERSION` from `mutter-x11-interop.xml`. Effort S.
5. Keep `dpi` derived from output scale, and document that no DPI leaf is needed. Effort S.
6. Move the `dpi_for_scale` 48..768 bound into `Scale` validation so it is checked once. Effort S.
7. Add a test asserting `SHELL` and `SURFACE` names match the vendored XML. Effort S.
8. Add a test asserting `INTERFACE` matches the XML. Effort S.
9. Tie `cursor_size` to one source for X11 and dconf (currently two `i32::try_from` sites). Effort S.
10. Remove the hard-coded `/run/user/1000` from host tests and use a temp `XDG_RUNTIME_DIR`. Effort S.

## 4. Constants to leave alone and why

- XSETTINGS names (`Gtk/*`, `Net/*`, `Xft/DPI`): wire names defined by the convention. Changing them breaks clients.
- XSETTINGS type ids 0 and 1: spec values.
- `1024` in `xdpi`: GTK's storage unit.
- `96` DPI base: the toolkit assumption that defines scale 1.
- GSettings schema paths and key names: must match GNOME exactly.
- `MATRIX` `include_str!` embeds: build-time data.
- `QT_QPA_PLATFORMTHEME` and `xdgdesktopportal`: Qt's own selectors.

## 5. Needs Kieran

- Should `titlebar_font_name` be a core config leaf in `settings.toml` (recommended), or a Bingux shell setting? Both are possible; the core leaf is the smaller change.
- Should the fallback defaults track current GNOME exactly (recommended, which changes what a fresh install shows) or stay as Gnoblin's own defaults? Choosing GNOME's keeps GTK and Gnoblin visually identical from first login.
