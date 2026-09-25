# Wallpapers

The session starts `gnoblin-gnome-wallpaper` from its autostart entry. The
client creates a non-interactive `zwlr_layer_shell_v1` surface on the
background layer for each monitor.

The client reads `org.gnome.desktop.background` through GNOME's `GnomeBG`
renderer. Existing GNOME picture, placement, color and slideshow settings
continue to apply. Gnoblin does not draw the wallpaper inside GNOME Shell.

The default entry can be removed or replaced in `~/.config/gnoblin/init.lua`:

```lua
gnoblin.configure {
    autostart = {
        ["gnoblin-gnome-wallpaper"] = {enable = false},
        my_wallpaper = {command = {"my-wallpaper-daemon"}},
    },
}
```

Autostart commands use argument arrays and start once per login. See
[autostart](/guides/autostart) for named entries. The
[configuration loading guide](/guides/files_and_load_order) explains how the
default entry combines with your config.

## Build your own wallpaper system

Use the standard [wlr-layer-shell protocol](/wayland-protocols)
from any toolkit or language with a Wayland client library. For every output,
create a surface with these properties:

- Select the `background` layer and anchor all four edges so the surface fills
  the output.
- Set keyboard interactivity to `none` and the exclusive zone to `-1`. The
  wallpaper should not take focus or reserve panel space.
- Track output additions, removals, and size changes. Recreate or resize the
  corresponding surface when the output layout changes.
- Keep drawing and input handling in your client. Gnoblin supplies placement
  and stacking; it does not impose an image format, animation model, or settings
  interface on custom clients.

GNOME-compatible clients can use `GnomeBG` for image scaling and color
controls. Other clients can use their own renderer and configuration.

Register a custom wallpaper program under a distinct autostart name, unless it
replaces the default entry. If both programs start, both surfaces occupy the
background layer.

### Existing clients

- [swaybg](https://github.com/swaywm/swaybg) is a straightforward fit: it
  explicitly supports compositors implementing `wlr-layer-shell` and
  `wl_output` version 4.
- [swww](https://github.com/LGFae/swww) offers runtime control and animated
  transitions. Check its current compositor requirements before choosing it.
- [hyprpaper](https://github.com/hyprwm/hyprpaper) is designed for Hyprland and
  provides socket controls. Gnoblin users can consider it where the needed
  Hyprland integration is available; [swaybg](https://github.com/swaywm/swaybg)
  is a compositor-generic option based directly on `wlr-layer-shell`.

Layer-shell clients need a compositor that advertises `zwlr_layer_shell_v1`.
Gnoblin enables this protocol by default; protocol availability is described in
the [protocol catalog](/wayland-protocols).
