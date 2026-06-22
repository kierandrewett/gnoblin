# Clients

Rust and Slint clients are grouped by shell surface. Thin binary crates own the
per-surface event loop and Wayland layer-shell setup; shared behavior belongs in
`shell-ui`.

## Layout

- `shell-ui/` is the shared Rust library. Put cross-client code here: desktop
  entry parsing/launching, app context menus, appmenu/DBusMenu integration,
  shell D-Bus wrappers, config helpers, theme data, tray, notification-center,
  quick-settings, and shared Slint assets/widgets.
- `topbar/` owns the top layer-shell bar. Its layout is driven by `[topbar]`
  config arrays, including flexible spacers, focused-app, appmenu, status, tray,
  notifications, and clock widgets.
- `dock/` owns the bottom layer-shell dock and uses `shell-ui::app_context_menu`
  so dock right-clicks match the focused-app menu.
- `wallpaper/` owns the background layer-shell surface and must stay
  input-transparent.
- `window-menu/` owns the compositor window action menu used by the shell.
- `launcher/`, `notifyd/`, `osd/`, `night-light`, and `power-menu/` are shell
  service or utility clients.
- `control-center/` is a C/GTK settings panel and is excluded from the Rust
  workspace.

## Rules Of Thumb

- Keep Slint surface-specific wiring in the role crate. Move anything reused by
  two clients into `shell-ui`.
- Keep desktop app behavior in `shell-ui` so the dock, launcher, focused-app
  widget, and topbar all agree.
- Keep layer-shell protocol and frame-callback behavior in the role crates;
  compositor-side protocol semantics live under `src/protocols/`.
- Do not edit generated files under `target/`.
