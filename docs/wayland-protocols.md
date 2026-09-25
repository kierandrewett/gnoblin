# Wayland protocol catalog

This page covers the additional protocol globals Gnoblin registers or gates
in a Gnoblin session. It does not list every core Wayland or upstream Mutter
global. A client must bind the interface advertised by the running compositor;
vendored XML alone does not mean a protocol is available.

## Configure availability

Gnoblin enables the listed protocol gates by default in its own session.
Some globals also use per-client filters, noted below. Disable a gate in
`init.lua` with its Lua key:

```lua
gnoblin.configure {
    protocols = {
        wlr_screencopy = false,
    },
}
```

Lua underscores become hyphens in the native setting name. Protocol globals
are registered at compositor startup, so **log out and back in** after changing
these gates. `gnoblinctl config reload` cannot add or remove an advertised
global. GNOME's separate login session does not advertise Gnoblin-owned
globals.

## Available interfaces

| Lua key                           | Advertised interface                                                                                                                                                                    | Use it for                                           | Guide                                        |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- | -------------------------------------------- |
| `wlr_layer_shell`                 | [`zwlr_layer_shell_v1` (v5)](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml)                                              | Bars, docks, launchers, overlays and exclusive zones | [Choose a shell](bring-your-own-shell.md)    |
| `wlr_screencopy`                  | [`zwlr_screencopy_manager_v1` (v3)](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/screencopy/wlr-screencopy-unstable-v1.xml)                                         | Native output capture                                | [Permissions](/guides/permissions)           |
| `ext_foreign_toplevel_list`       | [`ext_foreign_toplevel_list_v1` (v1)](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/foreign-toplevel-list/ext-foreign-toplevel-list-v1.xml)                          | Enumerate toplevel windows                           | [Shell integration](shell-integration.md)    |
| `wlr_foreign_toplevel_management` | [`zwlr_foreign_toplevel_manager_v1` (v3)](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/foreign-toplevel-management/wlr-foreign-toplevel-management-unstable-v1.xml) | Inspect and control windows                          | [Shell integration](shell-integration.md)    |
| `ext_data_control`                | [`ext_data_control_manager_v1` (v1)](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/data-control/ext-data-control-v1.xml)                                             | Clipboard managers                                   | [Session settings](/guides/session_settings) |
| `ext_idle_notify`                 | [`ext_idle_notifier_v1` (v2)](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/idle-notify/ext-idle-notify-v1.xml)                                                      | Idle notifications                                   | [Session settings](/guides/session_settings) |
| `wlr_gamma_control`               | [`zwlr_gamma_control_manager_v1` (v1)](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/gamma-control/wlr-gamma-control-unstable-v1.xml)                                | Per-output gamma                                     | [Session settings](/guides/session_settings) |
| `wlr_output_power_management`     | [`zwlr_output_power_manager_v1` (v1)](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/output-power-management/wlr-output-power-management-unstable-v1.xml)             | Output power state                                   | [Session settings](/guides/session_settings) |
| `ext_background_effect_v1`        | [`ext_background_effect_manager_v1` (v1)](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/ext-background-effect/ext-background-effect-v1.xml)              | Client-defined background blur regions               | [Background blur](background-effects.md)     |
| `xdg_decoration`                  | [`zxdg_decoration_manager_v1` (v1)](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/unstable/xdg-decoration/xdg-decoration-unstable-v1.xml)                        | Negotiate client or server titlebars                 | [Window frames](/guides/window_frames)       |
| `window_frame_renderer`           | [`gnoblin_window_frame_manager_v1` (v1)](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/window-frame/gnoblin-window-frame-v1.xml)                                     | Manager is visible only to its registered renderer   | [Frame renderer API](frame-renderer-api.md)  |
| `blur_fade`                       | [`gnoblin_blur_fade_manager_v1` (v1)](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/blur-fade/gnoblin-blur-fade-v1.xml)                                              | Per-item blur fade metadata                          | [Blur fades](blur-fades.md)                  |
| `ext_session_lock`                | [`ext_session_lock_manager_v1` (v1)](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/session-lock/ext-session-lock-v1.xml)                                             | Session locking for third-party lockers              | [Session locking](session-lock.md)           |

Each interface link opens its wire-level XML. The version is the maximum
global version Gnoblin registers; bind no higher than the version advertised
at runtime.

The frame-renderer manager is filtered to a renderer process launched by
Gnoblin. Regular clients will not see it in `wayland-info`.

For standard protocol requests, events and enum values, see the
[Wayland protocol documentation](https://wayland.freedesktop.org/docs/book/)
and the [wayland-protocols source](https://gitlab.freedesktop.org/wayland/wayland-protocols).
This catalog identifies the interface to use and where to find its details.

For example, a layer-shell client controls its anchors, keyboard interactivity
and exclusive zone. Gnoblin then places the surface and applies any matching
`type = "layer"` [window rules](/guides/window_rules). A panel that animates its own
whole surface can disable the compositor's layer animation for its namespace.

![A Quickshell panel above Firefox in a Gnoblin session](images/gnoblin-quickshell-firefox.png)

_Quickshell positions the panel with layer shell._

![GNOME Files open beneath a Quickshell panel in a Gnoblin session](images/gnoblin-quickshell-files.png)

_The same layer-shell surface sits above an ordinary application window._

Screen capture through `wlr_screencopy` is separate from capture through the
desktop portal. The latter follows [portal permission policy](/guides/permissions).
Turning off this global is not a blanket screen-sharing policy.

## Interface not yet available

`zwlr_output_manager_v1` has vendored XML but is not registered as a supported
Gnoblin global. Mutter exposes its `org.gnome.Mutter.DisplayConfig` D-Bus API
in the Gnoblin session; this is a Mutter interface, not a Gnoblin protocol.
See the [Mutter monitor manager reference](https://gnome.pages.gitlab.gnome.org/mutter/meta/class.MonitorManager.html).

[`gnoblinctl monitor list`](gnoblinctl.md#workspaces-and-monitors) reports
monitor geometry but does not change display layouts. Session locking uses
`ext_session_lock_manager_v1`; see [Session locking](session-lock.md) for its
requirements and security behavior.

## Inspect a running session

This catalog follows current Gnoblin source; older installed builds may
advertise fewer globals.

Check `gnoblinctl version --json` and the running registry when packaging or
debugging. Run `wayland-info` inside the Gnoblin session or devkit so it uses the
right `WAYLAND_DISPLAY`. A checkout or XML file cannot prove which globals a
login session advertised.
