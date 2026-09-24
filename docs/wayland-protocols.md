# Wayland protocol catalog

This page covers the additional protocol globals Gnoblin registers or gates
in a Gnoblin session. It does not list every core Wayland or upstream Mutter
global. A client must bind the interface advertised by the running compositor;
vendored XML alone does not mean a protocol is available.

## Configure availability

Gnoblin enables the listed interfaces by default in its own session. Disable
one in `init.lua` with its Lua key:

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

| Lua key                           | Advertised interface                                                                                            | Max version | Use it for                                           | Guide                                        |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------- | ----------- | ---------------------------------------------------- | -------------------------------------------- |
| `wlr_layer_shell`                 | [`zwlr_layer_shell_v1`](https://github.com/swaywm/wlroots/blob/master/protocol/wlr-layer-shell-unstable-v1.xml) | 5           | Bars, docks, launchers, overlays and exclusive zones | [Choose a shell](bring-your-own-shell.md)    |
| `wlr_screencopy`                  | `zwlr_screencopy_manager_v1`                                                                                    | 3           | Native output capture                                | [Permissions](/guides/permissions)           |
| `ext_foreign_toplevel_list`       | `ext_foreign_toplevel_list_v1`                                                                                  | 1           | Enumerate toplevel windows                           | [Shell integration](shell-integration.md)    |
| `wlr_foreign_toplevel_management` | `zwlr_foreign_toplevel_manager_v1`                                                                              | 3           | Inspect and control windows                          | [Shell integration](shell-integration.md)    |
| `ext_data_control`                | `ext_data_control_manager_v1`                                                                                   | 1           | Clipboard managers                                   | [Session settings](/guides/session_settings) |
| `ext_idle_notify`                 | `ext_idle_notifier_v1`                                                                                          | 2           | Idle notifications                                   | [Session settings](/guides/session_settings) |
| `ext_session_lock`                | `ext_session_lock_manager_v1`                                                                                   | 1           | Lock the session with an external locker             | [Session locking](session-lock.md)           |
| `wlr_gamma_control`               | `zwlr_gamma_control_manager_v1`                                                                                 | 1           | Per-output gamma                                     | [Session settings](/guides/session_settings) |
| `wlr_output_power_management`     | `zwlr_output_power_manager_v1`                                                                                  | 1           | Output power state                                   | [Session settings](/guides/session_settings) |
| `ext_background_effect_v1`        | `ext_background_effect_manager_v1`                                                                              | 1           | Client-defined background blur regions               | [Background blur](background-effects.md)     |
| `xdg_decoration`                  | `zxdg_decoration_manager_v1`                                                                                    | 1           | Negotiate client or server titlebars                 | [Window frames](/guides/window_frames)       |
| `window_frame_renderer`           | `gnoblin_window_frame_manager_v1`                                                                               | 1           | External frame renderer service                      | [Frame renderer API](frame-renderer-api.md)  |
| `blur_fade`                       | `gnoblin_blur_fade_manager_v1`                                                                                  | 1           | Per-item blur fade metadata                          | [Blur fades](blur-fades.md)                  |

The `Max version` column is the version Gnoblin advertises; clients can bind
that version or a lower one. The XML shipped under `src/protocols/` is the
wire-level reference for Gnoblin-owned implementations. For standard protocol
requests, events and enum values, see the
[Wayland protocol documentation](https://wayland.freedesktop.org/docs/book/)
and the [wayland-protocols source](https://gitlab.freedesktop.org/wayland/wayland-protocols).
This catalog explains which interface to use and where its behavior is documented.

For example, a layer-shell client controls its anchors, keyboard interactivity
and exclusive zone. Gnoblin then places the surface and applies any matching
`type = "layer"` [window rules](/guides/window_rules). A panel that animates its own
whole surface can disable the compositor's layer animation for its namespace.

Screen capture through `wlr_screencopy` is separate from capture through the
desktop portal. The latter follows [portal permission policy](/guides/permissions).
Turning off this global is not a blanket screen-sharing policy.

## Session locking

Gnoblin advertises the standard `ext-session-lock-v1` manager in the Gnoblin
session. The first locker to acquire it owns the active lock; a second locker
receives `finished`. The compositor covers every output, keeps the session
locked if the client exits, and sends `locked` only after its covered frame has
presented.

Gnoblin deliberately has no built-in lock UI or locker policy. Bingux ships a
separate lock client, and unmodified clients such as hyprlock, swaylock,
gtklock and waylock can use the same protocol. The regular GNOME session never
receives this global and retains GNOME ScreenShield.

Portal permissions do not change when the session locks. An already authorised
monitor stream sees the compositor lock scene instead of desktop content.
Remote input is available after `locked` and lock-scene presentation, and goes
only to the active lock surface; it is refused during transitions and failsafe.
No lock-specific portal configuration is needed. See [session locking](session-lock.md).

`zwlr_output_manager_v1` remains vendored XML only and is not an advertised
Gnoblin global.

## Inspect a running session

Check the running compositor when packaging or debugging a client. A Wayland
registry inspector such as `wayland-info` can show advertised globals; run it
inside the Gnoblin session or devkit terminal so it connects to the right
`WAYLAND_DISPLAY`. A source checkout, package manifest or XML file cannot prove
that a particular login session has advertised a global.
