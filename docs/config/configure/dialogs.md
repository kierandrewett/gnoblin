# gnoblin.configure.dialogs

Configure this part of `gnoblin.configure` with the `dialogs` key.

Restyle the compositor's own dialogs: the consent dialog an app's screenshot, screen
cast or input request raises, and the passphrase prompt that unlocks the keyring.

```lua
gnoblin.configure {
    dialogs = {
        style = {
            radius = 14,
            accent = "#89b4fa",
            font = { family = "monospace", size = 16 },
            ["button-layout"] = "stretch",
            scale = 1.25,
        },
    },
}
```

| Setting         | Accepted value                       | Default |
| --------------- | ------------------------------------ | ------- |
| `radius`        | An integer; the dialog uses 0 to 24  | Stock   |
| `accent`        | A named accent, `#rgb` or `#rrggbb`  | Stock   |
| `font.family`   | A name of up to 64 characters        | Stock   |
| `font.size`     | An integer; the dialog uses 12 to 32 | Stock   |
| `button-layout` | `start`, `end`, `stretch`, `stacked` | `start` |
| `scale`         | A number; the dialog uses 1 to 3     | `1`     |

## What a style cannot do

The dialogs are the compositor's own, so a style can never turn one into a disguise.
The compositor clamps whatever a file or a script asks for:

- A number outside its range is moved to the nearest limit, and the move is logged once
  per commit (`dialogs.style: font size 4 is drawn as 12`). Text is never smaller than 12,
  a dialog is never drawn smaller than stock, and a scale the screen cannot hold is
  lowered until the whole dialog fits.
- An `accent` too close to the dialog's own colour (a contrast ratio below 3) is moved
  toward white or black until it stands out. The text on it is black or white, whichever
  reads better, so it always meets a contrast ratio of 4.5. Text and its background keep
  that ratio whatever the palette.
- The Deny and Allow buttons are at least 88 by 32 points, grow with the font, and are the
  same size: Deny is never smaller than Allow. Deny is first in every layout and holds the
  initial focus.
- Nothing hides or shortens the 500 ms delay before Allow works, the name of the app that
  asks, or the lock icon on the passphrase prompt. There is no setting for any of them.

`font.family` accepts `monospace` (or `mono`) for the monospace face. Any other name is
drawn in the default face: the dialogs use the fonts they embed and do not look up system
fonts yet.

A dialog that is open keeps the look it opened with; the next one uses the new style.
