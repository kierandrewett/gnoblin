# Declarative Gnoblin configuration (draft)

## Shape

One Lua file returns one configuration table. Imports return tables too. No
declaration or removal calls are needed.

```lua
return {
    imports = {
        "/usr/share/gnoblin/modules/*.lua",
        "./hardware.lua",
    },

    shell = {minimize_duration = 150},

    shortcuts = {
        terminal = {
            keys = {"<Super>Return"},
            command = {"ptyxis", "--new-window"},
        },
        close_window = {
            keys = {"<Super>q"},
            action = "window.close",
        },
        volume_up = {
            keys = {"XF86AudioRaiseVolume"},
            action = "audio.volume_up",
        },
        unused_import = {enable = false},
    },

    autostart = {
        bar = {command = {"waybar"}},
    },

    window_rules = {
        dim_unfocused = {
            order = 100,
            match = {type = "window", focused = false},
            opacity = 0.95,
        },
    },
}
```

The names `terminal`, `close_window` and `volume_up` identify entries. An
imported entry can be changed by setting only the fields that differ, or
disabled with `{enable = false}`. A shortcut has either `command` or `action`.
The action catalogue will include desktop, window, audio, brightness and
playback actions. `keys` can contain more than one accelerator.

## Merge rules

| Value | Result |
| --- | --- |
| `imports` | Resolve relative to the importing file; expand globs in sorted order |
| Maps | Merge recursively by key |
| Scalars and lists | The closer module wins; a file wins over its imports |
| Named entries | Merge by map key; `enable = false` omits the entry from the active config |
| `window_rules` | Apply enabled entries by `order`, then name |

The built-in Gnoblin module supplies default shortcuts. Imported modules can
override it, and the root file takes precedence. The loader reports the file
and option path for invalid or conflicting definitions. Reload rebuilds the
result from the Lua files; it leaves no separately saved shortcut state.

## Shortcut ownership

| Kind | Gnoblin responsibility |
| --- | --- |
| Commands | Register native global grabs and launch argv |
| Window and shell actions | Register bindings with Mutter and GNOME Shell from the compiled Lua config |
| Media keys | Register native grabs and handle volume, brightness and playback actions |
| Reload | Replace the active set together; release removed grabs |

Gnoblin must not write shortcut definitions to the user's GNOME GSettings.
The GNOME Settings Daemon media-key handler must not claim keys in a Gnoblin
session. Application-local shortcuts remain inside their applications.

## Migration

The current `gnoblin.configure`, `gnoblin.shortcut`, `gnoblin.load` and related
calls remain readable during migration. New examples use returned tables.
Bingux is a separate shell that currently supplies a legacy Gnoblin config
fragment; its integration must keep working until that project adopts the new
format. The loader must reject mixed definitions that cannot be resolved
without guessing.
