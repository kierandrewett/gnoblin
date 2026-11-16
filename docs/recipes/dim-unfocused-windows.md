# Dim unfocused windows

Set the opacity of windows that are not focused. Put this in `~/.config/gnoblin/init.ts`:

```ts
import { ClientListener } from "gnoblin-core";
import type { Client } from "gnoblin-core";

gnoblin.clients.addListener(
    new (class extends ClientListener {
        onFocusChanged(client: Client, focused: boolean) {
            client.style.opacity = focused ? 1 : 0.95;
        }
    })(),
);
```

`onFocusChanged` runs each time a window gains or loses the focus. `opacity` is a number from `0` (transparent) to `1`
(opaque); the example uses `0.95`. The listener hears windows, not layer surfaces.

To fade the change instead of cutting it, wrap the assignment in
`gnoblin.requestTransition(() => { ... }, { duration: 180 })`, which tweens over 180 ms.
See [transitions](/js-transitions).

Opacity affects text and controls too. For a translucent panel with opaque text, use transparency in the client's
background instead. Remove the listener and save to go back to full opacity.

See [window style](/js-style) for the style properties.
