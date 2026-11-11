# Light and dark appearance

GNOME stores the user's preferred color scheme in
`org.gnome.desktop.interface` as `color-scheme`. Gnoblin reports preference
changes to Lua config, so compositor rules can follow the same choice as
applications.

| Value          | Meaning                                      |
| -------------- | -------------------------------------------- |
| `default`      | Follow the desktop's default appearance.     |
| `prefer-dark`  | The user prefers dark application surfaces.  |
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

## Window styling

Window rules carry no border fields, so a palette change cannot recolour window borders from Lua. The `corners` rule
field was removed, and the stock shell draws blur and shadow only.

See [Lua events](/events/) for the `gnoblin.appearance.color-scheme-changed` event fields.
