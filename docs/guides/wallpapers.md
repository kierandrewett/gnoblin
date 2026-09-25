# Wallpapers

Gnoblin includes wallpaper rendering in GNOME Shell. It reads the desktop
background settings from `org.gnome.desktop.background` and applies them to
each monitor. You do not need to install or start a wallpaper daemon.

The built-in renderer is enabled by default. Turn it off in
`~/.config/gnoblin/init.lua`:

```lua
gnoblin.configure {
    shell = {
        wallpaper = false,
    },
}
```

Set `wallpaper = true` to enable it again. The setting is persisted in
Gnoblin's feature preferences and applies on configuration reload. You can
also toggle the feature with `gnoblinctl feature wallpaper enable` or
`gnoblinctl feature wallpaper disable`.

## Systems without wallpaper images

Wallpaper images are optional. GNOME's background renderer paints the
configured color before loading a picture. If no picture is selected or its
file is missing, that color remains visible, so a fresh system still gets a
desktop background without a wallpaper bundle or image file. Set GNOME's
`primary-color` and, for a gradient, `secondary-color` in
`org.gnome.desktop.background` to choose the color.

For a solid dark background, set GNOME's picture style to `none` and choose a
color:

```sh
gsettings set org.gnome.desktop.background picture-options 'none'
gsettings set org.gnome.desktop.background primary-color '#242424'
gsettings set org.gnome.desktop.background color-shading-type 'solid'
```

GNOME picture placement and slideshow settings continue to apply when images
are configured. For example, use GNOME Settings → Appearance to choose a
picture or solid color. No Gnoblin autostart entry is needed.

## Use another wallpaper client

Disable Gnoblin's built-in renderer before starting another client, so two
backgrounds are not drawn at once:

```lua
gnoblin.configure {
    shell = {wallpaper = false},
    autostart = {
        {name = "wallpaper", command = {"swaybg", "-c", "#242424"}},
    },
}
```

The custom command is an ordinary Gnoblin autostart entry. See the
[autostart reference](/config/configure/autostart) for its restart and login
options.
