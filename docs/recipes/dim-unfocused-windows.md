# Dim unfocused windows

Set the opacity of windows that are not focused. Put this in `~/.config/gnoblin/init.ts`:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    rules: [{ focused: false, style: { opacity: 0.95 } }],
});
```

`focused: false` matches windows that do not have focus. `opacity` is a number from `0` (transparent) to `1` (opaque);
the example uses `0.95`. The rule applies to windows, not layer surfaces.

Opacity affects text and controls too. For a translucent panel with opaque text, use transparency in the client's
background instead. Remove the rule and save to go back to full opacity; a window whose rule stops matching returns to
opacity `1`.

See the [window rule reference](/config/window_rule) for all query keys and field ranges.
