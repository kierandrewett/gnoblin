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

| Lua key | Advertised interface | Use it for | Guide |
| --- | --- | --- | --- |
| `wlr_layer_shell` | `zwlr_layer_shell_v1` | Bars, docks, launchers, overlays and exclusive zones | [Choose a shell](bring-your-own-shell.md) |
| `wlr_screencopy` | `zwlr_screencopy_manager_v1` | Native output capture | [Permissions](/config/permissions) |
| `ext_foreign_toplevel_list` | `ext_foreign_toplevel_list_v1` | Enumerate toplevel windows | [Shell integration](shell-integration.md) |
| `wlr_foreign_toplevel_management` | `zwlr_foreign_toplevel_manager_v1` | Inspect and control windows | [Shell integration](shell-integration.md) |
| `ext_data_control` | `ext_data_control_manager_v1` | Clipboard managers | [Session settings](/config/session_settings) |
| `ext_idle_notify` | `ext_idle_notifier_v1` | Idle notifications | [Session settings](/config/session_settings) |
| `wlr_gamma_control` | `zwlr_gamma_control_manager_v1` | Per-output gamma | [Session settings](/config/session_settings) |
| `wlr_output_power_management` | `zwlr_output_power_manager_v1` | Output power state | [Session settings](/config/session_settings) |
| `ext_background_effect_v1` | `ext_background_effect_manager_v1` | Client-defined background blur regions | [Background blur](background-effects.md) |
| `xdg_decoration` | `zxdg_decoration_manager_v1` | Negotiate client or server titlebars | [Window frames](/config/window_frames) |
| `window_frame_renderer` | `gnoblin_window_frame_manager_v1` | External frame renderer service | [Frame renderer API](frame-renderer-api.md) |
| `blur_fade` | `gnoblin_blur_fade_manager_v1` | Per-item blur fade metadata | [Blur fades](blur-fades.md) |

The interface version that a client binds must be no higher than the version
the compositor advertises. The XML shipped under `src/protocols/` is the
wire-level reference for Gnoblin-owned implementations; this catalog explains
which interface to use and where its behavior is documented.

For example, a layer-shell client controls its anchors, keyboard interactivity
and exclusive zone. Gnoblin then places the surface and applies any matching
`type = "layer"` [window rules](/config/window_rules). A panel that animates its own
whole surface can disable the compositor's layer animation for its namespace.

Screen capture through `wlr_screencopy` is separate from capture through the
desktop portal. The latter follows [portal permission policy](/config/permissions).
Turning off this global is not a blanket screen-sharing policy.

## Interfaces not yet available

`ext_session_lock_v1` and `zwlr_output_manager_v1` have vendored XML but are
not registered as supported Gnoblin globals. The session-lock startup boundary
is compiled but deliberately advertises no global until it can enforce a lock.
Do not build a shell that requires them yet. Gnoblin's existing lock and
display configuration paths are separate from these two proposed interfaces.

## Inspect a running session

Check the running compositor when packaging or debugging a client. A Wayland
registry inspector such as `wayland-info` can show advertised globals; run it
inside the Gnoblin session or devkit terminal so it connects to the right
`WAYLAND_DISPLAY`. A source checkout, package manifest or XML file cannot prove
that a particular login session has advertised a global.
