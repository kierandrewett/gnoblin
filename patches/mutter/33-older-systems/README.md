# Older systems

These patches let the compositor build against the libraries of Debian 13, Ubuntu 26.04 LTS, Fedora 41, Alpine 3.21
and Ubuntu 24.04 LTS. Each patch keeps the newer code when the newer library is installed, so a build on a newer system
keeps every feature.

| Patch  | Library                                                              | Effect with the older library                                                                                                                                                         |
| ------ | -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0001` | wayland-protocols 1.44 (Debian 13)                                   | None. Private copies of four 1.48 protocol files are compiled in                                                                                                                      |
| `0002` | libinput 1.28 (Debian 13)                                            | No eraser button on pens, no libinput Lua plugins                                                                                                                                     |
| `0003` | libwayland 1.23 (Debian 13, Alpine 3.21)                             | No `wl_fixes` global, `wl_seat` version 9, clients repeat keys themselves                                                                                                             |
| `0004` | gsettings-desktop-schemas 48.0, IBus 1.5.32 (Debian 13)              | Newer input settings come from the Lua configuration only                                                                                                                             |
| `0005` | libei 1.3.0 (Fedora 41, Alpine 3.21)                                 | None more than below libei 1.5: a client sync is answered at once                                                                                                                     |
| `0006` | libinput 1.27 (Fedora 41, Alpine 3.21)                               | None more than with libinput 1.28                                                                                                                                                     |
| `0007` | PipeWire 1.2.7 (Fedora 41, Alpine 3.21)                              | None more than with PipeWire 1.4.2                                                                                                                                                    |
| `0008` | gsettings-desktop-schemas 47.1, IBus 1.5.31 (Fedora 41, Alpine 3.21) | None more than with 48.0 and 1.5.32                                                                                                                                                   |
| `0009` | wayland-protocols 1.38 (Alpine 3.21)                                 | None. Private copies of five more 1.48 protocol files are compiled in                                                                                                                 |
| `0010` | GLib 2.80 (Ubuntu 24.04)                                             | None. Layouts sort with `g_qsort_with_data()`, which gives the same order                                                                                                             |
| `0011` | libwayland 1.22 (Ubuntu 24.04)                                       | Each client connection has the 4 KB buffer of libwayland, not 1 MB. A client that floods or reads slowly is disconnected sooner. The client object is kept in a late destroy listener |
| `0012` | libdisplay-info 0.1.1 (Ubuntu 24.04)                                 | None if the copy is right. The colour primaries, gamma, signal colorimetry and HDR metadata of the display are read from the EDID with a copy of the libdisplay-info 0.2 code         |
| `0013` | libinput 1.25 (Ubuntu 24.04)                                         | Tablet pad dials do nothing. The pen pressure range is not applied. Tap-and-drag lock ends after a timeout, not at the next tap. The bus type comes from sysfs                        |
| `0014` | PipeWire 1.0.5 (Ubuntu 24.04)                                        | Screen casts have no explicit sync: with a driver that needs it, a consumer can show torn or partly drawn frames                                                                      |
| `0015` | gsettings-desktop-schemas 46.1 (Ubuntu 24.04)                        | The stylus switch-monitor and keybinding actions come from the Lua configuration only. No pressure range keys                                                                         |
| `0016` | libei 1.2.1 (Ubuntu 24.04)                                           | None more than with libei 1.3.0                                                                                                                                                       |
| `0017` | IBus 1.5.29 (Ubuntu 24.04)                                           | None more than with IBus 1.5.31: the preedit has no hints                                                                                                                             |

`config.h` is written from `config.h.meson`, and a define that the template does not name is never written. `0002`
also adds `HAVE_LIBINPUT_DWT_TIMEOUT`, which `17-distro-compat/0005` set in Meson only, so the disable-while-typing
timeout now works with libinput 1.31 and newer. `0005` adds `HAVE_EIS_EVENT_SYNC` to the template. `0011`, `0013`,
`0014` and `0015` add `HAVE_WAYLAND_SERVER_1_23`, `HAVE_LIBINPUT_1_26`, `HAVE_LIBINPUT_STICKY_DRAG_LOCK`,
`HAVE_PIPEWIRE_EXPLICIT_SYNC` and `HAVE_GDESKTOP_STYLUS_BUTTON_ACTION_KEYBINDING` to Meson and the template. `0012`
passes `HAVE_LIBDISPLAY_INFO_0_2` as a compiler argument instead: `edid.h` tests it, and some Gnoblin units include
`edid.h` before `config.h`. With the define in `config.h` only, those units declared the types a second time on
Debian 13.

`0015` also reads the pressure range and the stylus button keybinding keys only when the schema has them. GSettings
aborts the process when code reads a key that the schema does not have, and gsettings-desktop-schemas 46.1 has neither.

The IBus guards are in the overlay file `src/input/method/gnoblin-ibus-input-method.c`. Two earlier patches lower
floors that need no code: `17-distro-compat/0004` (libwayland 1.24) and `17-distro-compat/0002` (PipeWire).

The portal backend has its own series in `patches/xdg-desktop-portal-gnome/06-older-systems`: Glycin is optional,
libadwaita 1.6 gets the older view switcher in the screen cast dialog, and xdg-desktop-portal 1.18.4 is accepted.
`0004` builds against libadwaita 1.5 and gsettings-desktop-schemas 46.1: the Settings portal reports no accent colour,
the wallpaper preview shows a `GtkSpinner`, and the app chooser description is not centred. The libgxdp patch in
`patches/portal-dependencies/01-libgxdp` makes the portal build against GTK 4.14.

Ubuntu 24.04 loses the most. `0011`, `0013`, `0014` and the portal `0004` each remove behaviour, and each is a file of
its own. Without one of them Ubuntu 24.04 does not build, so to refuse a loss, drop `0010` to `0017`, the portal
`0004` and the Ubuntu 24.04 build together. Later patches of this group touch the same `meson.build` and
`config.h.meson` lines, so a single patch cannot be dropped without a refresh of the ones after it. `0012` keeps the
colour and HDR data, but with code that nobody has run against a real display. No one has started a Gnoblin session on
Ubuntu 24.04.

The `series` file lists this group last, so its context includes every earlier group.
The `compile` job in `.github/workflows/verify.yml` compiles the series on the oldest release of each family.
