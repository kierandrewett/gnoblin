# Light and dark appearance

A Gnoblin session tells applications which colour scheme you prefer. GTK 4 and libadwaita apps use that preference to
choose their light or dark appearance.

| Value          | Meaning                                      |
| -------------- | -------------------------------------------- |
| `default`      | Follow the desktop's default appearance.     |
| `prefer-dark`  | The user prefers dark application surfaces.  |
| `prefer-light` | The user prefers light application surfaces. |

## Set the preference

Set it in your config with the [`appearance`](/config/configure/appearance) key:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    appearance: { colorScheme: "prefer-dark" },
});
```

The setting reaches applications through the Settings portal and the dconf layer, and a change is announced to apps that
are already running. The GNOME setting is the same one under the hood: `colorScheme` of `default` leaves each
application to its own choice. You can also set the system preference with `gsettings`:

```sh
gsettings set org.gnome.desktop.interface color-scheme prefer-dark
```

## What follows the setting

GTK 4 and libadwaita apps use this preference to choose their light or dark appearance. GTK 3 apps commonly follow the
`gtk-theme` setting, such as `Adwaita` or `Adwaita-dark`. The preference does not force every application to change:
some apps use their own theme controls, while some non-GTK apps choose to read `color-scheme` themselves.

Gnoblin does not pick an application theme. It publishes the preference you set.

## Window styling

A window rule's `style` carries `opacity`, with `cornerRadius` and `shadows` stored and not drawn. It has no border
fields, so a palette change cannot recolour window borders. A config cannot read the preference and branch on it: there
is no colour-scheme event in the TypeScript shell. To restyle windows for dark mode, change the `rules` and reload.
