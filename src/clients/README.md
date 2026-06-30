# Clients

Rust and Slint clients are grouped by shell surface. Thin binary crates own the
per-surface event loop and Wayland layer-shell setup; shared behavior belongs in
the crates under `crates/`.

## Layout

- `crates/gnoblin-core/` is std-only shared code: config, client args, XDG path
  helpers, file-backed flags, night-light/DND flags, and test helpers.
- `crates/gnoblin-desktop/` owns XDG desktop integration: desktop entries,
  icon lookup, tray, and appmenu/DBusMenu integration.
- `crates/gnoblin-runtime/` owns Slint/layer-shell runtime code, theme/prefs,
  shell D-Bus wrappers, notification-center state, app context menus, and shared
  Slint assets/widgets.
- `bin/topbar/` owns the top layer-shell bar. Its layout is driven by `[topbar]`
  config arrays, including flexible spacers, focused-app, appmenu, status, tray,
  notifications, and clock widgets.
- `bin/dock/` owns the bottom layer-shell dock and uses `gnoblin_runtime::app_context_menu`
  so dock right-clicks match the focused-app menu.
- `bin/wallpaper/` owns the background layer-shell surface and must stay
  input-transparent.
- `bin/window-menu/` owns the compositor window action menu used by the shell.
- `bin/launcher/`, `bin/notifyd/`, `bin/osd/`, `bin/night-light`, and
  `bin/power-menu/` are shell service or utility clients.

## Rules Of Thumb

- Keep Slint surface-specific wiring in the role crate. Move reused code into
  the narrowest shared crate that matches its dependencies.
- Keep desktop app behavior in `gnoblin-desktop` so the dock, launcher,
  focused-app widget, and topbar all agree.
- Keep layer-shell protocol and frame-callback behavior in the role crates;
  compositor-side protocol semantics live under `src/protocols/`.
- Do not edit generated files under `target/`.
