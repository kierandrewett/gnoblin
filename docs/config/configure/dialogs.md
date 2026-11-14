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

In the [TypeScript shell](/typescript-shell) the same settings are the `dialogs` key of `defineConfig`. Keys are
camelCase there (`buttonLayout`) and the document's kebab-case (`button-layout`) here; the values are the same, and
the compositor refuses and clamps them the same way.

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    dialogs: {
        style: {
            radius: 14,
            accent: "#89b4fa",
            font: { family: "monospace", size: 16 },
            buttonLayout: "stretch",
            scale: 1.25,
        },
    },
});
```

A value the compositor refuses (`radius: 5000`) fails the reload with the key path
(`defineConfig: dialogs.style.radius: ...`) and leaves the running config in place. Removing the `dialogs` key and
reloading puts every setting back to `Stock`.

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

## Refusing a request from a script

A script can look at each request before its dialog opens and refuse it. This is policy only: it cannot allow a
request, answer for the user, or see a passphrase.

```ts
import { DialogListener, defineConfig } from "gnoblin";

class NoScreenshots extends DialogListener {
    onConsentRequest(request) {
        if (request.subject === "Screenshot" && request.appId !== "org.gnome.Shell") {
            request.decline("screenshots are off");
        }
    }
}

export default defineConfig({
    setup() {
        gnoblin.dialogs.addListener(new NoScreenshots());
    },
});
```

The compositor shows each consent and passphrase request to the shell for 250 ms before the dialog opens.
`request.decline(reason)` refuses it: the requester is told it was cancelled and no dialog opens.

A listener that does nothing, throws, or runs past its time budget leaves the request to the built-in dialog. That
dialog asks the user with the same delay and the same identity line as ever.

A request has:

- `kind`: `consent` or `secret`;
- `appId`: the app the compositor verified;
- `subject`: the portal interface of a consent request, or the app's own prompt text of a passphrase request, which
  is not to be trusted as identity;
- `deadline`.

It has no method that allows a request and gives no access to a secret. A listener that throws is logged once and the
next listener still runs.
