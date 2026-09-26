# gnoblin.configure.input_sources

Set the active input sources with `gnoblin.configure {input_sources = {...}}`.
On reload, this replaces the active source list in memory. Remove the table to
restore GNOME's session sources.

- `sources` is required. It is an array of records with a `type` and a
  nonempty `id`.
- `sources[].type` accepts `"xkb"` for a keyboard layout or `"ibus"` for an
  input method.
- `sources[].id` is an installed XKB layout ID or IBus engine ID, depending on
  `type`.
- `per_window` is optional and defaults to `false`. Set it to `true` to
  remember a different source for each window. With the default `false`, all
  windows share the same active source; switching layouts in one window changes
  the source used in the others too.

## Choose an ID

Use the exact source ID, not its display name. For example, `"us+intl"` selects
the US International variant. Available layouts and variants depend on the
XKB data installed on your system.

| Source           | Example ID  | Meaning                                              |
| ---------------- | ----------- | ---------------------------------------------------- |
| US English       | `"us"`      | The base US layout.                                  |
| British English  | `"gb"`      | The base UK layout.                                  |
| US International | `"us+intl"` | The US layout with the `intl` variant.               |
| Anthy            | `"anthy"`   | An IBus Japanese input engine; it must be installed. |

Find layouts and variants in **Settings → Keyboard → Input Sources**. GNOME's
[`GnomeXkbInfo`](https://gnome.pages.gitlab.gnome.org/gnome-desktop/html/gnome-desktop3/gnome-desktop3-GnomeXkbInfo.html)
API lists the layout IDs GNOME recognizes. The
[XKB introduction](https://xkbcommon.org/doc/current/xkb-intro.html) explains
how layouts and variants combine. For an overview of available layouts, see the
[XKB layout gallery](https://xkeyboard-config.freedesktop.org/layouts/).

To find installed IBus engine IDs, run:

```sh
ibus list-engine --name-only
```

See the [`ibus` command reference](https://manpages.debian.org/testing/ibus-data/ibus.1.en.html)
for the engine-list command.

## Configure the sources

This example lets you switch between the base US and UK layouts:

```lua
gnoblin.configure {
    input_sources = {
        sources = {
            {type = "xkb", id = "us"},
            {type = "xkb", id = "gb"},
        },
        per_window = false,
    },
}
```

With `per_window = false` (or with `per_window` omitted), the selected layout is
shared. If you switch from US to UK while typing in one window, a different
window also uses UK when it gets focus.

Set `per_window = true` when you want each window to keep its own last selected
source:

```lua
gnoblin.configure {
    input_sources = {
        sources = {
            {type = "xkb", id = "us"},
            {type = "xkb", id = "gb"},
        },
        per_window = true,
    },
}
```

Open a terminal and a document window. Select US and UK, respectively. With
sloppy focus, moving the pointer between windows restores each window's last
selected layout. The same happens with click-to-focus when you click between
them.

A window without a saved selection starts with the layout that was active when
it was first focused.

Gnoblin remembers the active source per window. It does not assign layouts by
app ID or give each window a different source list.

Use [Lua events](/config/lua-events) when a rule should follow the pointer
window or depend on its app ID. For example, the pointer-window event can
change touchpad scroll speed for Chromium and restore the usual speed elsewhere.
That is a separate rule from `per_window`, which remembers the layout selected
in each focused window.

Use the layout-plus-variant ID for a variant. An IBus ID must name an engine
installed on your system:

```lua
gnoblin.configure {
    input_sources = {
        sources = {
            {type = "xkb", id = "us+intl"},
            {type = "ibus", id = "anthy"},
        },
        per_window = false,
    },
}
```

This setting selects layouts or input methods; it does not set XKB options.
Configure those separately with
[`input.keyboard.xkb_options`](/config/configure/input/keyboard).

## Type definition

The IDs are installation-dependent strings. `?` marks an optional field.

```lua
gnoblin.configure {
    input_sources = {
        sources = {{type = "xkb" | "ibus", id = string}, ...},
        per_window = boolean?,
    },
}
```
