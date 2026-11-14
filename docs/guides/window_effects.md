# Window effects

A window's blur and shadow come from its style. This page says which effects work today and which do not. The source
of truth is the code, not this page.

## What works now

| Effect                 | Where it works     | Set from                                            |
| ---------------------- | ------------------ | --------------------------------------------------- |
| Opacity                | Every renderer     | `style.opacity`, or a rule's `style`                |
| Outer shadow           | GLES renderer only | `style.shadows`                                     |
| Blur behind the window | GLES renderer only | `style.backdropFilter`                              |
| Rounded corners        | Stored, not drawn  | `style.cornerRadius`                                |
| Borders, filter, fill  | Stored, not drawn  | `style.borders`, `style.filter`, `style.background` |

A property that no renderer draws is still stored and reported in the window's `computedStyle`, so a config that sets
it keeps working when the renderer learns to draw it.

## Set an effect

Set the style where it should apply. X

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    style: { shadows: [{ y: 8, blur: 24, color: "#00000066" }] },
    rules: [{ appId: "foot", style: { opacity: 0.92 } }],
});
```

The `style` key sets the style of every window. A window rule's `style` sets the matched window's own style, which beats
it. The scopes, the full property list and the filters are in [window style](/js-style).

## What a window rule cannot do

A rule does not take `blur`, `corners`, `shader`, `animation`, `frame` or `match.layer`. A rule that names one is
refused when the config loads, and the message names the key. Use `style.backdropFilter` for blur.

## Background blur from applications

Applications and shells can ask for blur behind a region they draw, through the `ext_background_effect` protocol. That
path is separate from the style and is described in [background effects](/background-effects).
