# Cursor themes

[Configuration reference](/config/configure)

Set the compositor cursor theme and size in `~/.config/gnoblin/init.lua`.
Gnoblin uses an installed Xcursor theme by default. Source builds made with
`./build.sh --with-vector-cursors` also read Hyprcursor themes.

- `cursor.theme` accepts an installed Xcursor or Hyprcursor theme name. Its
  default is `default`, the system cursor theme. If the named theme is not
  installed, Gnoblin shows the default cursor instead.
- `cursor.size` accepts an integer from `1` to `256` logical pixels. Its
  default is `24`.

Omitted values use these defaults.

## Select a theme

Install a cursor theme, then replace `ThemeName` below with its installed
theme name:

```lua
gnoblin.configure {
    cursor = {
        theme = "ThemeName",
        size = 24,
    },
}
```

Changes apply after saving; run `gnoblinctl config reload` to apply them now.
To select an installed Adwaita Xcursor theme:

```lua
gnoblin.configure {cursor = {theme = "Adwaita", size = 24}}
```

![The pointer visible while Fuzzel searches for Firefox](../images/gnoblin-waybar-launcher.png)

## Hyprcursor themes

Build with `--with-vector-cursors` to enable Hyprcursor support. The upstream
[theme guide](https://github.com/hyprwm/hyprcursor/blob/main/docs/MAKING_THEMES.md)
describes theme files and cursor metadata.

Install a compiled theme in `~/.local/share/icons/<theme>/` or
`~/.icons/<theme>/`, then set `cursor.theme` to its installed name. The theme
and size apply to compositor cursors and the launch wait cursor. Animated
frames retain their hotspots and timing.

If the selected theme or shape is unavailable through Hyprcursor, Gnoblin
tries Xcursor. Client applications that supply their own cursor surfaces
continue to draw those surfaces themselves.

## Adwaita-Hyprcursor

The optional source build can install Gnoblin's Adwaita vector artwork. Install
Hyprcursor 0.1.13 or newer, hyprcursor-util, librsvg and your distribution's
Adwaita cursor theme, then build with:

```sh
./build.sh --with-vector-cursors
```

This adds `Adwaita-Hyprcursor` under the build prefix. Select it in
`~/.config/gnoblin/init.lua`:

```lua
gnoblin.configure {cursor = {theme = "Adwaita-Hyprcursor", size = 24}}
```

The compiled theme includes Adwaita SVG artwork, animation and Xcursor
fallbacks. Other installed themes need no theme build step.
