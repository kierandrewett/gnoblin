# Cursor themes

Gnoblin keeps the GNOME cursor theme and size settings. Adwaita remains the
default; installing Hyprcursor support does not change the selected theme.

## Adwaita vectors

`Adwaita-Hyprcursor` packages the original GNOME Adwaita 49 SVG artwork: 35
shapes, 153 frames (including the 60-frame wait and progress cursors), and 28
aliases. It preserves the original 24px hotspots and 16ms animation timing.
Original Xcursor assets are included for applications that supply cursor buffers.
The GNOME artwork and licence files are in `src/cursor/adwaita/`.

Build with Python 3, Inkscape and `hyprcursor-util`:

```sh
scripts/build-adwaita-hyprcursor.py
cp -a build/Adwaita-Hyprcursor ~/.local/share/icons/
gsettings set org.gnome.desktop.interface cursor-theme 'Adwaita-Hyprcursor'
```

The builder refuses to replace an existing output; use `--output` to choose a
new directory. `--fallback` selects the installed Adwaita theme to copy for
Xcursor compatibility. Cursor size remains controlled by GNOME settings.
Until you log into the Hyprcursor-enabled compositor, the theme uses its matching
Adwaita bitmap fallback. To return to the system theme, select `Adwaita` again.

Validation (Pillow and `rsvg-convert` are also required):

```sh
scripts/test-adwaita-artwork.py
scripts/test-adwaita-hyprcursor.py
ADWAITA_HYPRCURSOR_PATH="$HOME/.local/share/icons/Adwaita-Hyprcursor" \
  GNOBLIN_PREFIX="$PWD/install" \
  GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-adwaita-hyprcursor.py" \
  scripts/run-gnome-shell.sh
```

The artwork check compares 188 renders with the original SVG sheet. The native
check verifies all frames, aliases, hotspots and timing at 24, 30, 37, 48 and
96 pixels. The private compositor requests 37 pixels, absent from the Xcursor
assets, to verify that Mutter renders the SVG theme.

## Other Hyprcursor themes

Mutter can load Hyprcursor themes when built with `-Dhyprcursor=enabled` and
Hyprcursor 0.1.13 or newer. Nix packaging enables it; local Meson builds detect
the development package automatically. The loader is active only in Gnoblin
sessions and falls back to Xcursor for missing themes or shapes.

Install a compiled Hyprcursor theme in `~/.local/share/icons/<theme>/` or
`~/.icons/<theme>/`, then select it with GNOME settings:

```sh
gsettings set org.gnome.desktop.interface cursor-theme 'ThemeName'
gsettings set org.gnome.desktop.interface cursor-size 24
```

Theme and size changes invalidate the cache without restarting the compositor.
`HYPRCURSOR_THEME`, if set before login, overrides the Hyprcursor theme name while
the GNOME setting still selects the Xcursor fallback. SVG frames are rendered
at the requested cursor size and monitor scale; hotspots and animated frame
timings are preserved. See the [upstream theme format](https://github.com/hyprwm/hyprcursor/blob/main/docs/MAKING_THEMES.md).

This covers compositor cursors and Wayland cursor-shape requests. Applications
that supply their own cursor buffers continue to control those images. A theme
with both Hyprcursor and Xcursor assets gives those applications a matching
fallback.

The launch-feedback IPC uses Mutter's themed wait cursor globally, including
over client surfaces. It restores the application cursor after launch completion
or timeout. Older compositor sessions use the standard GNOME cursor artwork
through the script fallback until the next login.

After building Mutter, run the vector renderer and native launch checks in
private sessions:

```sh
scripts/build-hyprcursor-test.sh
GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-hyprcursor.py" scripts/run-gnome-shell.sh
GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-launch-feedback.py" scripts/run-gnome-shell.sh
```

The vector check generates a two-frame SVG theme and verifies rendered pixels,
sizes, hotspots, timing, missing-theme fallback, invalidation and GNOME session
isolation. The launch check exercises overlapping requests, expiry, reload and
completion when a real application window maps.
