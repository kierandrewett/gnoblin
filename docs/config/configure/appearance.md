# Appearance settings

The `appearance` key of `defineConfig` sets the colour scheme, the accent colour and the accessibility preferences
that the shell tells other toolkits about:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    appearance: {
        colorScheme: "prefer-dark",
        accent: "blue",
        reducedMotion: true,
        highContrast: true,
    },
});
```

| Setting         | Accepted value                           | Default   |
| --------------- | ---------------------------------------- | --------- |
| `colorScheme`   | `default`, `prefer-dark`, `prefer-light` | `default` |
| `accent`        | A named accent, `#rgb` or `#rrggbb`      | Unset     |
| `reducedMotion` | A boolean                                | `false`   |
| `highContrast`  | A boolean                                | `false`   |

`colorScheme` of `default` states no preference, so each application keeps its own choice. `prefer-dark` and
`prefer-light` ask applications for that scheme.

`accent` is one of the nine named accents:

- `blue`, `teal`, `green`, `yellow`, `orange`
- `red`, `pink`, `purple`, `slate`

A hex colour is accepted without an alpha channel. GTK takes only the named accents, so a hex colour is published as
the nearest named accent. When `accent` is unset, toolkits keep their own accent.

`reducedMotion: true` asks applications to stop animating. The shell also stops its own animations while it is on:
`gnoblin.motion(seconds)` returns `0`, so a workspace switch or a window move lands at once. Changing the setting
affects the next animation only. One that is already running finishes. `false` states no preference.

`highContrast: true` asks applications for a high-contrast appearance. `false` states no preference.

Both preferences reach applications through the Settings portal and the dconf layer that GTK reads. Qt and KDE
applications do not read them unless they have their own bridge.

To follow the light or dark preference in a window rule, see [light and dark appearance](/guides/theming).
The other `defineConfig` keys are in the [defineConfig reference](/typescript-config).
