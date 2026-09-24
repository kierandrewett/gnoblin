# Cursors

[Configuration reference](/config/configure)

Gnoblin reads the compositor cursor theme and size from
`~/.config/gnoblin/init.lua`. The config API is `cursor.theme` and
`cursor.size`; Gnoblin currently renders those settings with Hyprcursor.
The default theme is `Adwaita-Hyprcursor` at size `24`. Cursor theme and size
are no longer read from GSettings in a Gnoblin session.

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
To select the Adwaita artwork theme:

```lua
gnoblin.configure {cursor = {theme = "Adwaita-Hyprcursor", size = 24}}
```

## Hyprcursor support

Gnoblin loads compositor cursor themes through Hyprcursor. The upstream
[theme guide](https://github.com/hyprwm/hyprcursor/blob/main/docs/MAKING_THEMES.md)
describes theme files and cursor metadata.

Install a compiled theme in `~/.local/share/icons/<theme>/` or
`~/.icons/<theme>/`, then set `cursor.theme` to its name. The configured theme
and size control compositor cursors and the launch wait cursor. Animated frames
retain their hotspots and timing.

Client applications that supply their own cursor surfaces continue to draw
them themselves. Gnoblin does not look up Xcursor theme files as a fallback.

## Build Adwaita-Hyprcursor

From the Gnoblin checkout, with Python 3, Inkscape and `hyprcursor-util` installed:

```sh
scripts/build-adwaita-hyprcursor.py
mkdir -p ~/.local/share/icons
cp -a build/Adwaita-Hyprcursor ~/.local/share/icons/
```

Then select it in `init.lua` with `gnoblin.configure {cursor = {theme =
"Adwaita-Hyprcursor", size = 24}}` and reload the config.

The builder refuses to overwrite its output. Use `--output` for another
directory.

The theme uses GNOME Adwaita SVG artwork and preserves hotspots and animation.
Artwork and licences are in `src/cursor/adwaita/`.

## Build support and tests

Mutter needs Hyprcursor 0.1.13+ and `-Dhyprcursor=enabled`.
Nix enables it; local Meson builds detect the development package.

Run `tests/test-adwaita-artwork.py` and `tests/test-adwaita-hyprcursor.py`
for artwork checks. They also need Pillow and `rsvg-convert`.

For native testing, run `tests/test-hyprcursor.py` and
`tests/test-launch-feedback.py` through the [private test harness](/testing).
They cover sizes, hotspots, timing, missing shapes and launch cursor restoration.
