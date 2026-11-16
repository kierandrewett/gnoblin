# Dialog settings

The compositor draws its own dialogs: the consent dialog an app's screenshot, screen cast or input request raises, and
the passphrase prompt that unlocks the keyring. A script sees each request through `gnoblin.dialogs` and may refuse
it, as below.

The look of the dialogs (the radius, accent, font, button layout and scale) has no TypeScript home yet:
see [settings with no TypeScript home yet](/js-settings#dropped-settings). The dialogs keep their stock look.

## Refusing a request from a script

A script can look at each request before its dialog opens and refuse it. This is policy only: it cannot allow a
request, answer for the user, or see a passphrase.

```ts
import { DialogListener } from "gnoblin-core";

class NoScreenshots extends DialogListener {
    onConsentRequest(request) {
        if (request.subject === "Screenshot" && request.appId !== "org.gnome.Shell") {
            request.decline("screenshots are off");
        }
    }
}

gnoblin.dialogs.addListener(new NoScreenshots());
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

## What a dialog never hides

The dialogs are the compositor's own, so nothing a script does can turn one into a disguise. Deny is first in every
layout and holds the initial focus. Nothing hides or shortens the 500 ms delay before Allow works, the name of the app
that asks, or the lock icon on the passphrase prompt.
