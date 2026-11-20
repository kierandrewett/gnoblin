# Window effects

A window's blur and shadow come from its style. This page says which effects work today and which do not. The source
of truth is the code, not this page.

## What works now

| Effect                 | Where it works     | Set from                                            |
| ---------------------- | ------------------ | --------------------------------------------------- |
| Opacity                | Every renderer     | `style.opacity`                                     |
| Outer shadow           | GLES renderer only | `style.shadows`                                     |
| Blur behind the window | GLES renderer only | `style.backdropFilter`                              |
| Rounded corners        | Stored, not drawn  | `style.cornerRadius`                                |
| Borders, filter, fill  | Stored, not drawn  | `style.borders`, `style.filter`, `style.background` |
| Your own shader        | See below          | A `ShaderFilter` in `style.filter`                  |

A property that no renderer draws is still stored and reported in the window's `computedStyle`, so a config that sets
it keeps working when the renderer learns to draw it.

## Set an effect

Set the style on the scope it should reach:

```ts
gnoblin.clients.style.shadows = [{ x: 0, y: 8, blur: 24, color: "#00000066" }];
gnoblin.clients.where({ appId: "foot" }).style.opacity = 0.92;
```

`gnoblin.clients.style` sets the style of every window. The style of a `where` selection, a workspace's `clients` or one
client sits above it and wins. The scopes, the full property list and the filters are in [window style](/js-style).

A frosted window is a blur and a little saturation behind it:

```ts
gnoblin.clients.where({ appId: "foot" }).style.backdropFilter = [{ blur: 24 }, { saturate: 1.35 }];
```

## Your own shader

A `ShaderFilter` gives a window a shader you wrote. It is on by default: the compositor draws one when the renderer draws
shaders, unless the `compositor.user-shaders` setting is switched off. [Custom shaders](/custom-shaders) has the
contract, the limits and two complete examples.

## Background blur from applications

Applications and shells can ask for blur behind a region they draw, through the `ext_background_effect` protocol. That
path is separate from the style and is described in [background effects](/background-effects).
