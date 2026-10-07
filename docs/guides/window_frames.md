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

Set `window_management.button_layout` to choose Mutter's global server-side
button order. The per-window `frame.button_layout` option controls buttons for
Gnoblin frame renderers. See [titlebar button placement](/config/configure/window_management#titlebar-button-placement).

The embedded default config enables Gnoblin's built-in `native` renderer with
`mode = "auto"` and extents `{32, 1, 1, 1}`. Apps keep their own titlebars;
apps requesting server decorations get a native frame. It needs no desktop shell
or external renderer. The default config disables animations.

Use `mode` to decide which windows get a frame and `renderer` to choose what
draws it. Setting a renderer alone does not enable frames. A config without a
frame rule retains the API default, `"off"`.

Run `gnoblinctl config restore-default` to replace your config with the current
embedded defaults; Gnoblin backs up your previous config. To keep your other
settings, add the frame rule below instead.

## Choose a mode

| Mode            | Behavior                                                            |
| --------------- | ------------------------------------------------------------------- |
| `off`           | Let the app draw its titlebar; add no frame or cropping             |
| `auto`          | Add a frame only when the app explicitly asks for one               |
| `prefer-server` | Prefer a Gnoblin frame for apps that support decoration negotiation |
| `replace`       | Hide the configured app margins and draw a Gnoblin frame            |

Apps negotiate decorations through Wayland decoration protocols. GTK on Wayland
uses KDE's server-decoration protocol; other clients commonly use
`xdg-decoration`. Gnoblin supports server decorations through both. GTK windows
that create the KDE decoration object use `auto` behavior by default, so
Gnoblin's native renderer supplies their frame. A matching Lua window rule can
select another mode. `auto` uses client requests instead of guessing from
appearance.

GTK apps with a custom titlebar can still draw titlebar controls inside their
surface. Gnoblin applies its frame and clips declared client shadow margins,
but it cannot identify arbitrary pixels inside an app surface as a border
without risking removal of app content. `corners.remove_csd` reconstructs
detected client-rounded corners before applying Gnoblin's shape.

With `prefer-server`, Gnoblin uses the app's declared visible bounds when the
app keeps CSD. It clips buffer margins outside those bounds and draws Gnoblin's
configured border and shadow around them. Apps that omit visible bounds use the
full surface.

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

The `crop` option hides a strip of the app and makes that strip unclickable. It
cannot distinguish a titlebar from tabs, search boxes or other controls. Use
it only when you know exactly which margins you want to hide.

With `prefer-server`, the declared visible bounds are cropped automatically
when an app keeps CSD; `crop` adds extra margins in that case. Negotiated SSD
clients use their own declared bounds and ignore `crop`. With `replace`, zero
extents give a crop-only window. Fullscreen temporarily removes explicit crop
and frame extents.

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

Wayland window effects exclude transparent buffer margins outside the client's
declared window geometry. Borders and shadows include Gnoblin's server frame.
When an app uses Gnoblin's frame, `corners.radius` rounds the whole window,
frame and content, even in `auto` mode, so square app content never shows
outside the frame's corners. States where rounding is off, such as maximised
with `keep_maximized = false`, make both the frame and the content square.

Popup positions use the parent window's client origin, excluding Gnoblin's
frame extents and accounting for cropped margins. This also applies when a
popup acknowledges its configuration or follows a moved parent.

For renderer implementation and tests, see
[renderer architecture](/window-frame-renderers) and the [author guide](/frame-renderer-api).
