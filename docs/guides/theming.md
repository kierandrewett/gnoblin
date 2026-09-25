# Light and dark appearance

GNOME stores the user's preferred color scheme in
`org.gnome.desktop.interface` as `color-scheme`. Gnoblin reads this preference
and reports changes to Lua config, so compositor rules can follow the same
choice as applications.

| Value | Meaning |
| --- | --- |
| `default` | Follow the desktop's default appearance. |
| `prefer-dark` | The user prefers dark application surfaces. |
| `prefer-light` | The user prefers light application surfaces. |

Set the preference in GNOME Settings → Appearance, or with `gsettings`:

```sh
gsettings set org.gnome.desktop.interface color-scheme prefer-dark
```

## What follows the setting

GTK 4 and libadwaita apps use this preference to choose their light or dark
appearance. GTK 3 apps commonly follow the `gtk-theme` setting, such as
`Adwaita` or `Adwaita-dark`. The preference does not force every application
to change: some apps use their own theme controls, while some non-GTK apps
choose to read `color-scheme` themselves.

Gnoblin does not pick an application theme. It makes the desktop preference
available to Lua configs, which can update compositor styling and settings for
shells or apps that opt in.

## Follow the preference in window rules

Add a later rule for the selected palette. The example keeps all rules already
loaded, then replaces the window-rule list with the current palette each time
GNOME reports a change:

```lua
local base_rules = gnoblin.snapshot().window_rules or {}

gnoblin.on("gnome.interface.color-scheme-changed", function(event)
    local rules = {}
    for i, rule in ipairs(base_rules) do rules[i] = rule end

    local border = event.color_scheme == "prefer-dark" and "#aab2bd99" or "#35405299"
    rules[#rules + 1] = {
        match = {type = "window"},
        borders = {inner_width = 1, inner_color = border},
    }
    gnoblin.configure {window_rules = rules}
end)
```

Gnoblin also sends the current preference after the config loads, so the first
palette applies without waiting for a later desktop change. Other values use
the light palette in this short example; branch on `default` separately if
your shell has its own default colors.

See [Lua events](/config/lua-events) for the event fields and
[window effects](/guides/window_effects) for border, shadow and blur options.
