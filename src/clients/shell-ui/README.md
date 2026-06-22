# gnoblin-shell-ui

`gnoblin-shell-ui` is the shared Rust crate for shell clients. It is not a
surface by itself; role crates such as `topbar`, `dock`, `wallpaper`, and
`window-menu` link it for common behavior.

## What Belongs Here

- Shell D-Bus wrappers in `shell.rs`.
- Desktop entry parsing, app resolution, and launching in `lib.rs`.
- Focused-app and dock context menu construction/activation in
  `app_context_menu.rs`.
- GTK and KDE/DBusMenu appmenu helpers in `appmenu.rs`.
- Config, theme, tray, notification-center, quick-settings, date/time, drag and
  drop, preferences, and night-light helper modules.
- Shared Slint assets and component library under `vendor/slint/`.

## Role Crates

The layer-shell event loops live in the binary crates next to this crate:

- `../topbar` handles the configurable topbar layout, focused-app/appmenu
  widgets, popouts, and top exclusive zone.
- `../dock` handles favorites/running apps, dock context menus, launching, and
  bottom exclusive zone.
- `../wallpaper` handles the background layer-shell surface.
- `../window-menu` handles the compositor window action menu.

Keep per-surface Wayland plumbing in those crates. Move shared app behavior here
as soon as two clients need to agree on it.

## Slint Visual Review Loop

Do not sign off shell chrome, topbar, dock, popout, tooltip, or quick-settings
changes from static tests alone. Capture a screenshot and run:

```sh
scripts/design-review.sh /tmp/gnoblin-notif-cc-history-expanded.png
```

The report is written to `build/design-review/`. It uses Claude vision when
available and opencode as a secondary source/code reviewer. A score below 7/10
means keep iterating before handing the UI back to the user.

Use this rubric before editing Slint:

- Icon systems must be optically consistent, recognizable, and sharp at the
  rendered size. Avoid mixing icon families inside one control cluster.
- Shell surfaces need visible hierarchy: topbar, quick toggles, sliders, and
  notification history should not read as one undifferentiated block.
- State cannot rely on a tiny shade shift. Use color, contrast, shape, and text
  emphasis so active/inactive states scan at a glance.
- Slint internals should use `Tokens`, `Spacing`, `Radius`, `Motion`, and
  exported `Shell*` primitives. Treat one-off pixel values as design debt.
- Desktop shell chrome should be calm and compact, but not cramped. If text is
  truncated in a primary control, the layout has failed.
