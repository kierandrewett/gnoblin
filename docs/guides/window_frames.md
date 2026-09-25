# window_frames

[Configuration API](/config)

Most apps draw their own titlebar and buttons. This is **client-side decoration
(CSD)**. Gnoblin can draw them instead: **server-side decoration (SSD)**.

`window_management.action_*_titlebar` sets Mutter's titlebar action
preferences. GTK apps typically draw their own titlebars and read GNOME
settings directly, so this Lua setting does not change their client-side
decoration (CSD) behavior.

Gnoblin's built-in fallback server-side decoration (SSD) supports these
actions:

- `toggle-maximize`
- `toggle-maximize-horizontally`
- `toggle-maximize-vertically`
- `minimize`
- `lower`
- `menu`
- `none`

A custom SSD renderer implements its own titlebar click behavior.

Gnoblin's frames are off by default, although your desktop shell can enable
them through its config. Use `mode` to decide which windows get a frame and
`renderer` to choose what draws it. Setting a renderer alone does not enable frames.

## Choose a mode

| Mode            | Behavior                                                            |
| --------------- | ------------------------------------------------------------------- |
| `off`           | Let the app draw its titlebar; add no frame or cropping             |
| `auto`          | Add a frame only when the app explicitly asks for one               |
| `prefer-server` | Prefer a Gnoblin frame for apps that support decoration negotiation |
| `replace`       | Hide the configured app margins and draw a Gnoblin frame            |

Apps negotiate decorations through the Wayland `xdg-decoration` protocol.
Not every app supports it, and an app that gives no preference may still draw
a titlebar. `auto` uses explicit requests instead of guessing from appearance.

## Let apps request a Gnoblin titlebar {#enable-negotiated-ssd}

Add this to `~/.config/gnoblin/init.lua` after any `gnoblin.load(...)` lines:

```lua
gnoblin.window_rule {
    match = {type = "window"},
    frame = {
        mode = "auto",
        renderer = "native",
        extents = {36, 0, 0, 0},
    },
}
```

This uses the built-in renderer with a 36-pixel titlebar and no side or bottom
border. `extents` lists top, right, bottom and left sizes in that order.

Save and run `gnoblinctl config reload`. The app must also update its Wayland
surface before a decoration change takes effect; if it does not change, reopen
the app. Apps that draw their own titlebars can look unchanged under `auto`.

## Frame options

All fields below go inside `frame`. Later matching rules override individual
fields. Sizes are logical pixels.

| Field                 | Default                             | Values                                               |
| --------------------- | ----------------------------------- | ---------------------------------------------------- |
| `mode`                | `"off"`                             | Modes listed above                                   |
| `extents`             | `{32, 1, 1, 1}`                     | Top/right/bottom/left frame size; integers 0–256     |
| `crop`                | `{0, 0, 0, 0}`                      | Removed client margins; same order and range         |
| `renderer`            | `"native"`                          | Registered service name                              |
| `style`               | `"default"`                         | Style understood by that renderer                    |
| `background`          | `"#242424"`                         | Active background colour                             |
| `foreground`          | `"#eeeeee"`                         | Foreground colour                                    |
| `inactive_background` | `"#303030"`                         | Unfocused background colour                          |
| `button_layout`       | `{"minimize", "maximize", "close"}` | Ordered buttons, without duplicates; `{}` hides them |

Colours accept `#RRGGBB` or `#RRGGBBAA`.
Renderer and style names accept 1–64 letters, digits, `_` or `-`.

## Cropping client decorations

Cropping hides a strip of the app and makes that strip unclickable. It cannot
distinguish a titlebar from tabs, search boxes or other controls. Use it only
when you know exactly which margins you want to hide.

With `prefer-server`, explicit crop provides a fallback for CSD-only clients;
negotiated SSD clients are not cropped. With `replace`, zero extents give a
crop-only window. Fullscreen temporarily removes crop and frame extents.

## Custom renderers

Register the renderer command, then select its name in a rule:

```lua
gnoblin.configure {
    frame_renderers = {
        cairo = {"gnoblin-frame-cairo"},
    },
}

gnoblin.window_rule {
    match = {type = "window"},
    frame = {mode = "auto", renderer = "cairo"},
}
```

Use a command name resolved through the compositor's `PATH`, followed by
separate arguments. A full executable path is also accepted. `native` is
reserved.
[Bingux](/bring-your-own-shell) supplies its own styled renderer.

Config reload restarts external renderers, including rebuilt executables at
unchanged paths. The native frame keeps controls available during replacement
or failure. A compositor upgrade still needs logout and login.

## Limits

Frames work on normal Wayland application windows (`xdg-toplevel`). They do
not apply to X11 apps running through Xwayland, popups, bars or launchers.
Moving framed windows between displays with different scales and cropping apps
with many popups have limited test coverage.

For renderer implementation and tests, see
[renderer architecture](/window-frame-renderers) and the [author guide](/frame-renderer-api).
