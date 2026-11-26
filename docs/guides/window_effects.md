# Window effects

A window's blur and shadow come from its style. This page says which effects each renderer draws today. The source of
truth is the code, not this page.

## What each renderer draws

GLES is the renderer with the most effects. Vulkan is getting full parity with GLES. The software renderer draws the
simpler effects and cannot run user shaders.

| Effect                                           | GLES                          | Vulkan                        | Software                      |
| ------------------------------------------------ | ----------------------------- | ----------------------------- | ----------------------------- |
| Opacity                                          | Drawn                         | Drawn                         | Drawn                         |
| Rounded corners (window clip)                    | Drawn                         | Drawn                         | Drawn, anti-aliased           |
| Rounded corners (shadow shape)                   | Drawn                         | Drawn                         | Drawn                         |
| Outer shadow                                     | Drawn, with spread and colour | Drawn, with spread and colour | Drawn, with spread and colour |
| Inset shadow                                     | Drawn                         | Drawn                         | Drawn                         |
| Borders (several, offset)                        | Drawn                         | Drawn                         | Drawn, anti-aliased           |
| Backdrop blur                                    | Drawn                         | Not drawn                     | Drawn                         |
| Backdrop grades (saturate, brightness, contrast) | Drawn                         | Not drawn                     | Drawn                         |
| Filter grades on the window                      | Drawn                         | Drawn                         | Not drawn                     |
| Filter blur on the window                        | Drawn, one layer              | Not drawn                     | Not drawn                     |
| Background                                       | Drawn                         | Drawn                         | Drawn                         |
| Your own shader                                  | Drawn                         | Drawn                         | Not drawn: no shader compiler |

A few notes on the table:

- Rounded corners on a shader window are drawn by the shader wrapper on GLES and Vulkan, and not on software, which has
  no shader.
- A grade on the backdrop takes effect only with a blur behind the window, and only GLES and the software renderer
  draw that blur. The blur honours `downscale`, and `saturation` and `noise` as well as brightness, contrast and tint.
- Filter grades and the filter's blur on GLES apply to everything one layer paints: its shadows, background, content and
  borders. A window drawn as several layers (a titlebar child over the content) grades the layer that carries the effect.
  The filter's blur is drawn from a picture of that layer and spills past its edge.
- Vulkan grades the same things, and does not draw the filter's `blur` yet: a filter that blurs is drawn graded and not
  blurred there, and the node's `skipped` list says `filter: unsupported-filter-blur`. The software renderer stores the
  filter and draws nothing; use a shader if you need it there.
- On Vulkan an outer shadow fades with the window's opacity, as in CSS. A background, an inset shadow and a border take
  the window's tint as well.

A property that no renderer draws is still stored and reported in the window's `computedStyle`, so a config that sets
it keeps working when the renderer learns to draw it.

## Set an effect

Set the style on the scope it should reach. A style set on all windows is overridden by one set on a selection of
windows, or on one window. The scopes, the full property list and the filters are in [window style](/js-style).

::: tabs

```typescript
import "gnoblin-std";

gnoblin.clients.style.shadows = [{ x: 0, y: 8, blur: 24, color: "#00000066" }];
gnoblin.clients.where({ appId: "foot" }).style.opacity = 0.92;
```

```python
from gnoblin.objects.std import ClientStyle, Shadow, StyleQuery

style = ClientStyle.of_clients()
style.shadows = [Shadow(x=0, y=8, blur=24, color="#00000066")]
ClientStyle.of_clients().where(StyleQuery(app_id="foot")).opacity = 0.92
```

```rust
use gnoblin_std::objects::{ClientStyle, Shadow};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let style = ClientStyle::of_clients(&objects)?;
    style.set_shadows(&[Shadow {
        x: 0.0,
        y: 8.0,
        blur: 24.0,
        color: "#00000066".into(),
        ..Shadow::default()
    }])?;
    Ok(())
}
```

:::

A frosted window is a blur and a little saturation behind it:

::: tabs

```typescript
import "gnoblin-std";

gnoblin.clients.where({ appId: "foot" }).style.backdropFilter = [{ blur: 24 }, { saturate: 1.35 }];
```

```python
from gnoblin.objects.std import BlurFilter, ClientStyle, SaturateFilter, StyleQuery

style = ClientStyle.of_clients().where(StyleQuery(app_id="foot"))
style.backdrop_filter = [BlurFilter(radius=24.0), SaturateFilter(amount=1.35)]
```

```rust
use gnoblin_std::objects::{BlurFilter, ClientStyle, SaturateFilter};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let style = ClientStyle::of_clients(&objects)?;
    style.set_backdrop_filter(&[BlurFilter::default().into(), SaturateFilter::default().into()])?;
    Ok(())
}
```

:::

## Your own shader

A `ShaderFilter` gives a window a shader you wrote. It is on by default: the compositor draws one when the renderer draws
shaders, unless the `compositor.user-shaders` setting is switched off. The software renderer draws no shader.
[Custom shaders](/custom-shaders) has the contract, the limits and two complete examples.

## Background blur from applications

Applications and shells can ask for blur behind a region they draw, through the `ext_background_effect` protocol. That
path is separate from the style and is described in [background effects](/background-effects).
