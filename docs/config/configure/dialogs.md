# Dialog settings

The compositor draws its own dialogs: the consent dialog an app's screenshot, screen cast or input request raises, and
the passphrase prompt that unlocks the keyring. A script sees each request through the dialogs object, `gnoblin.dialogs`
in TypeScript and Python and `Dialogs` in Rust, and may refuse it, as below.

The look of the dialogs (the radius, accent, font, button layout and scale) has no TypeScript home yet, and no Python or
Rust one either: see [settings with no object yet](/js-settings#dropped-settings). The dialogs keep their stock look.

## Refusing a request from a script

A script can look at each request before its dialog opens and refuse it. This is policy only: it cannot allow a
request, answer for the user, or see a passphrase.

::: tabs

```typescript
import { DialogListener } from "gnoblin-core";
import type { DialogRequest } from "gnoblin-core";

class NoScreenshots extends DialogListener {
    onConsentRequest(request: DialogRequest) {
        if (request.subject === "Screenshot" && request.appId !== "org.gnome.Shell") {
            request.decline("screenshots are off");
        }
    }
}

gnoblin.dialogs.addListener(new NoScreenshots());
```

```python
from gnoblin.objects.model import DialogListener, DialogRequest, Gnoblin

gnoblin = Gnoblin.get()


class NoScreenshots(DialogListener):
    def on_consent_request(self, request: DialogRequest) -> None:
        if request.subject == "Screenshot" and request.app_id != "org.gnome.Shell":
            request.decline("screenshots are off")


gnoblin.dialogs.add_listener(NoScreenshots())
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{DialogListenerHandlers, ModelRoot};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = ModelRoot::get(&objects)?;
    let dialogs = model.dialogs()?;
    let listener = DialogListenerHandlers {
        on_consent_request: Some(Box::new(|request| {
            if request.subject()? == "Screenshot" && request.app_id()? != "org.gnome.Shell" {
                request.decline(Some("screenshots are off"))?;
            }
            Ok(())
        })),
        ..Default::default()
    }
    .implement(&objects)?;
    dialogs.add_listener(&listener)?;
    Ok(())
}
```

:::

The compositor shows each consent and passphrase request to the shell for 250 ms before the dialog opens.
`decline(reason)` refuses it: the requester is told it was cancelled and no dialog opens. In Rust the reason is an
`Option`, so pass `Some("...")` or `None`.

A listener that does nothing, throws, or runs past its time budget leaves the request to the built-in dialog. That
dialog asks the user with the same delay and the same identity line as ever.

A request has:

- `kind`: `consent` or `secret`. In TypeScript these are the strings `"consent"` and `"secret"`; in Python,
  `DialogKind.CONSENT` and `DialogKind.SECRET`; in Rust, `DialogKind::Consent` and `DialogKind::Secret`;
- `appId` (`app_id` in Python and Rust): the app the compositor verified;
- `subject`: the portal interface of a consent request, or the app's own prompt text of a passphrase request, which
  is not to be trusted as identity;
- `deadline`: when the compositor takes the request back, in milliseconds on its own clock.

It has no method that allows a request and gives no access to a secret. A listener that throws is logged once and the
next listener still runs.

## What a dialog never hides

The dialogs are the compositor's own, so nothing a script does can turn one into a disguise. Deny is first in every
layout and holds the initial focus. Nothing hides or shortens the 500 ms delay before Allow works, the name of the app
that asks, or the lock icon on the passphrase prompt.
