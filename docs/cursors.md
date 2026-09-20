# Cursor themes

[Configuration reference](configuration-reference.md)

Gnoblin uses GNOME's cursor theme and size settings. Adwaita is the default.

## Select a theme

Install the theme, then run:

```sh
gsettings set org.gnome.desktop.interface cursor-theme 'ThemeName'
gsettings set org.gnome.desktop.interface cursor-size 24
```

Changes apply without restarting. To return to Adwaita:

```sh
gsettings set org.gnome.desktop.interface cursor-theme 'Adwaita'
```

## Hyprcursor themes

Install a compiled theme in `~/.local/share/icons/<theme>/` or
`~/.icons/<theme>/`, then select it as above.

Gnoblin uses Hyprcursor for compositor cursors and cursor-shape requests.
Apps supplying their own cursor buffers need matching Xcursor assets.
Missing themes or shapes fall back to Xcursor.

`HYPRCURSOR_THEME`, set before login, overrides the Hyprcursor name while the
GNOME setting still selects the Xcursor fallback.

## Build Adwaita-Hyprcursor

From the Gnoblin checkout, with Python 3, Inkscape and `hyprcursor-util` installed:

```sh
scripts/build-adwaita-hyprcursor.py
mkdir -p ~/.local/share/icons
cp -a build/Adwaita-Hyprcursor ~/.local/share/icons/
gsettings set org.gnome.desktop.interface cursor-theme 'Adwaita-Hyprcursor'
```

The builder refuses to overwrite its output. Use `--output` for another
directory and `--fallback` to select the Xcursor theme copied alongside it.

The theme uses GNOME Adwaita SVG artwork and preserves hotspots and animation.
Artwork and licences are in `src/cursor/adwaita/`.

## Build support and tests

Mutter needs Hyprcursor 0.1.13+ and `-Dhyprcursor=enabled`.
Nix enables it; local Meson builds detect the development package.

Run `tests/test-adwaita-artwork.py` and `tests/test-adwaita-hyprcursor.py`
for artwork checks. They also need Pillow and `rsvg-convert`.

For native testing, run `tests/test-hyprcursor.py` and
`tests/test-launch-feedback.py` through the [private test harness](testing.md).
They cover sizes, hotspots, timing, fallback and launch cursor restoration.
