# Window frames (SSD)

Native SSD with experimental [language-independent renderers](window-frame-renderers.md).
The former GJS renderer factory is no longer supported.

SSD is **off by default**, including when a client requests server decorations.
Opt in with a rule whose `frame.mode` is `auto`, `prefer-server` or `replace`.
Selecting a renderer alone does not enable SSD. Gnoblin's bundled `native`
renderer is deliberately basic: title, built-in vector buttons with hover/press
feedback, and native move/resize controls. It needs no GTK, icon theme or external
process. It is also the recovery frame for an explicitly enabled external SSD.
Bingux owns the styled GTK4/libadwaita renderer; it is not a Gnoblin default.

Gnoblin can negotiate `xdg-decoration` v1 with Wayland clients, or replace a
client decoration using an explicit crop. Native code commits crop and frame
extents with the client's acknowledged configure. Painting and picking use
the same crop; fullscreen temporarily removes both crop and frame extents.

Add a `frame` table to an ordinary Lua window rule:

```lua
{
    match = { ["app-id"] = "^(spotify|com\\.spotify\\.Client(?:\\.desktop)?)$" },
    frame = {
        mode = "prefer-server",
        extents = { 36, 2, 2, 2 },
        background = "#242424",
        foreground = "#eeeeee",
        ["inactive-background"] = "#303030",
        ["button-layout"] = { "minimize", "maximize", "close" },
    },
}
```

`extents` and `crop` are four integers in logical pixels: top, right, bottom,
left (0–256). `crop` defaults to zero. Only set it after measuring the actual
client chrome: it removes pixels **and their input targets**. Excessive crop
that would eliminate the whole committed client geometry is ignored.

- `auto` (opt-in): honor client CSD preference; provide SSD when the client
  requests it or leaves the choice to Gnoblin.
- `prefer-server`: choose SSD when the client has an xdg-decoration object.
  An explicit nonzero crop supplies a replacement-frame fallback for CSD-only
  clients. No fallback crop is applied to a negotiated SSD client.
- `replace`: retain protocol CSD mode, crop the requested margins and add SSD.
- `off` (default): request CSD and disable Gnoblin crop/frame layout.

With `replace`, zero extents and nonzero crop give a crop-only window. Existing
`corners` and `borders` rules style the outer frame. Window blur is not required.

## Custom renderers

Build the non-JS examples with `scripts/build-frame-renderers.sh`. Register named
services at the root of your Lua configuration, using absolute executable paths
and argv arrays (no shell expansion):

```lua
return {
    ["frame-renderers"] = {
        cairo = { "/absolute/path/gnoblin-frame-cairo", "--theme-file=/absolute/path/theme.txt" },
        qt = { "/absolute/path/gnoblin-frame-qt", "--theme-file=/absolute/path/theme.txt" },
    },
    ["window-rules"] = {
        { match = { ["app-id"] = "spotify" },
          frame = { mode = "prefer-server", renderer = "cairo", style = "default",
                    extents = { 36, 2, 2, 2 } } },
    },
}
```

When SSD is enabled, `native` is the basic renderer and reserved fallback. Renderer/style names accept
letters, digits, `_` and `-`, up to 64 characters. Each service starts lazily and
handles all assigned windows. Unknown or unavailable services retain native
frames. Service definitions require a new compositor session; rule selection and
native appearance can reload. The example theme file contains `#204080`; valid
color edits repaint, invalid edits retain the previous color.

External renderers draw buffers and declare standard-action regions through the
private Wayland protocol. They cannot paint or intercept the client body. The
native fallback handles crashes and pending geometry. See the protocol and C/Qt
adapters linked in the architecture document to implement another language.

## Current boundaries

This implementation targets Wayland xdg-toplevels, not Xwayland, popups or layer
shell. The stock renderer uses a top titlebar; use a custom renderer for other
layouts. State-specific crop profiles, decoration protocol v2 and external
Quickshell-rendered frames are not implemented. Mixed-scale transitions and
popup-heavy cropped applications need further integration coverage.

Native changes require a new compositor session. Reloading Lua can change
frame policy and appearance only once the native frame API is running.

## Tests

`tests/window-frame-policy.test.mjs` validates rule policy. Run
`tests/test-window-frames.py` through `scripts/run-gnome-shell.sh` using
`GNOBLIN_TEST_DBUS_CLIENT` for real pixel, crop, input, fullscreen and window
control checks. `GNOBLIN_SSD_NEGOTIATED=1` exercises Qt decoration negotiation;
`GNOBLIN_SSD_RENDERER=cairo` or `qt`, with `GNOBLIN_SSD_THEME` pointing to the
service's theme file, exercises external rendering, theme reload and recovery.
`tests/test-window-frames-spotify.py` is an optional installed-Spotify check
using a private display, private bus, empty profile and disabled network.
