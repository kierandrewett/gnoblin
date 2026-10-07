# Wallpapers

Gnoblin paints a solid black background behind desktop surfaces. It is not a
wallpaper client. The shell you run owns wallpaper surfaces and decides how to
display them above Gnoblin's background. Configure its wallpaper client using
that project's instructions.

To start a separate wallpaper client with the session, add it to Gnoblin's
autostart configuration. This example starts `swaybg` with a solid color:

```lua
gnoblin.configure {
    autostart = {
        wallpaper = {command = {"swaybg", "-c", "#242424"}},
    },
}
```

Install the command first, and use your shell's configuration if it already
manages wallpaper. Run one wallpaper client so separate background surfaces do
not cover each other.

## Apps that request a wallpaper

The GNOME portal backend can show a confirmation preview when an app asks to
set a wallpaper. After approval, it copies the selected image to
`$XDG_CONFIG_HOME/background` and updates `org.gnome.desktop.background`.
Your shell still needs to read those settings and display the image; Gnoblin's
black background remains behind its surfaces.
