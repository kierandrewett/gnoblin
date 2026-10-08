# gnoblin.configure.input_sources

Set the active input sources with `gnoblin.configure {input_sources = {...}}`.
On reload, this replaces the active source list in memory. Without this table,
Gnoblin exposes no switchable input sources and leaves Mutter's current keymap
in place. It does not load the saved GNOME input-source list.

- `sources` is required. It is an array of records with a `type` and a
  nonempty `id`.
- `sources[].type` accepts `"xkb"` for a keyboard layout or `"ibus"` for an
  input method.
- `sources[].id` is an installed XKB layout ID or IBus engine ID, depending on
  `type`.
- In the direct Gnoblin session, XKB-only sources do not need `ibus-daemon`.
  Gnoblin does not start the daemon. Start it in your session and install the
  selected engine before using an IBus source. Gnoblin reconnects when the
  IBus service becomes available or restarts.
- `per_window` is optional and defaults to `false`. Set it to `true` to
  remember a different source for each window. With the default `false`, all
  windows share the same active source; switching layouts in one window changes
  the source used in the others too.

After login, and after a reload that removes the current source, the first `xkb` source in the list becomes current.

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

Find layouts and variants in **Settings → Keyboard → Input Sources**. Gnoblin
reads the installed XKB rules through libxkbregistry. The
[XKB introduction](https://xkbcommon.org/doc/current/xkb-intro.html) explains
how layouts and variants combine. For an overview of available layouts, see the
[XKB layout gallery](https://xkeyboard-config.freedesktop.org/layouts/).

When it needs a spare layout in an XKB group, Gnoblin chooses the base layout
for the locale's country where one is available, then falls back to `us`.

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

## Switch sources with the keyboard

The starter config binds two keys, as a GNOME session does:

- **Super+Space** selects the next source and wraps to the first after the last.
- **Shift+Super+Space** selects the previous source.

With fewer than two sources, the keys do nothing.

These bindings live in `config/40-shortcuts.lua`. `gnoblinctl init` copies that
file once, so a copy made earlier does not have them. Run
`gnoblinctl config restore-default` to replace your folder with the current
starter files. This moves your files to a backup, so copy back any changes you
made. To keep your file instead, add the two `switch_input_source` bindings from
the starter file to your own config.

## Share or keep sources per window

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

## Type with an input method {#type-with-an-input-method}

Gnoblin includes an IBus input method. Apps that use Wayland `text-input-v3`,
such as GTK4 and Qt on Wayland, get preedit and committed text from the IBus
engine you select, with no toolkit module or `GTK_IM_MODULE` setting. Gnoblin
does not start the IBus daemon. Without a running daemon, or without an active
engine, every key goes to the app unchanged.

1. Install `ibus` and an engine. On Fedora, for the Russian transliteration
   engine used here:

   ```sh
   sudo dnf install ibus ibus-m17n
   ```

2. Start the daemon in your session. This autostart entry starts it at login and
   stops it when you remove the entry or log out. Leave out the `-d` flag, which
   makes the daemon detach from the process group that Gnoblin stops:

   ```lua
   gnoblin.configure {
       autostart = {
           ibus = {command = {"ibus-daemon", "-r", "-x"}},
       },
   }
   ```

3. Configure the source with the engine ID from `ibus list-engine --name-only`:

   ```lua
   gnoblin.configure {
       input_sources = {
           sources = {
               {type = "xkb", id = "us"},
               {type = "ibus", id = "m17n:ru:translit"},
           },
       },
   }
   ```

4. Select the IBus source and type `privet` into a GTK4 text field. The text
   becomes `привет`.

Builds made without the IBus development files (`ibus-devel` on Fedora) leave
this input method out. In that case a GTK app can use the toolkit's own IBus
module instead. Install `ibus-gtk3` and `ibus-gtk4`, and start the app with
`GTK_IM_MODULE=ibus`. Qt and X11 apps use their own IBus settings, which are not
described here.

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
