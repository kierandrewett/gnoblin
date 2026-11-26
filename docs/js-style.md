# Window style

A window's look is a style: values you set from your shell file or from any listener. The style classes are the same in
TypeScript, Python and Rust: `ClientStyle` and the records `Shadow`, `Border` and the filters.

The tables on this page use the TypeScript spelling of a name:

- Python uses `snake_case`, such as `corner_radius`, `backdrop_filter` and `is_set`.
- Rust has a getter and a `set_` method for each property, such as `corner_radius()` and `set_corner_radius(..)`.
- Rust spells `where` as `where_`.

::: tabs

```typescript
import "gnoblin-std";

gnoblin.clients.style.opacity = 0.95;
gnoblin.clients.style.shadows = [{ y: 8, blur: 24, color: "#00000066" }];
gnoblin.clients.where({ appId: "firefox" }).style.shadows = [];
const client = gnoblin.clients.get("42");
if (client) {
    client.style.opacity = 0.8;
}
```

```python
from gnoblin.objects.model import ClientQuery, Gnoblin
from gnoblin.objects.std import Shadow

gnoblin = Gnoblin.get()

gnoblin.clients.style.opacity = 0.95
gnoblin.clients.style.shadows = [Shadow(y=8, blur=24, color="#00000066")]
gnoblin.clients.where(ClientQuery(app_id="firefox")).style.shadows = []
client = gnoblin.clients.get("42")
if client is not None:
    client.style.opacity = 0.8
```

```rust
use gnoblin_core::objects::{ClientQuery, ModelRoot};
use gnoblin_std::objects::Shadow;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(clients) = model.clients()? else {
        return Ok(());
    };
    clients.style()?.set_opacity(0.95)?;
    clients.style()?.set_shadows(&[Shadow {
        y: 8.0,
        blur: 24.0,
        color: "#00000066".to_owned(),
        ..Shadow::default()
    }])?;
    let firefox = ClientQuery {
        app_id: Some("firefox".to_owned()),
        ..ClientQuery::default()
    };
    clients.where_(&firefox, None)?.style()?.set_shadows(&[])?;
    if let Some(client) = clients.get_by_id("42")? {
        client.style()?.set_opacity(0.8)?;
    }
    Ok(())
}
```

:::

## Scopes

Style is set where it should apply, and a client takes the highest scope that sets each property. From lowest to
highest:

| Scope                          | Applies to                               |
| ------------------------------ | ---------------------------------------- |
| the built-in defaults          | every client                             |
| `gnoblin.clients.style`        | every client, current and future         |
| `workspace.clients.style`      | the clients of that workspace            |
| `clients.where({ ... }).style` | the clients that match, while they match |
| `client.style`                 | that one client                          |

Inheritance is per property. A workspace that sets only `cornerRadius` leaves the shadows to the global style.

A `where()` selection takes a second argument, `priority`:

- The higher priority wins.
- Selections of equal priority rank by the order their `style` was first used, and the later one wins.
- A client that stops matching a selection loses that selection's values at once and keeps its own.

A style a listener sets on one client with `client.style` is that client's own, so it beats every scope above it.

A scope's style holds only what was set on it. `isSet(property)` says whether the scope sets a property or inherits it, and
`values()` returns what the scope sets, as one record. A read of a property gives what the scope sets, else what it
inherits. `computedStyle` gives the resolved value for a window:

::: tabs

```typescript
import "gnoblin-std";

const client = gnoblin.clients.focused;
if (client) {
    console.log(client.style.isSet("corner_radius")); // false: the client does not override it
    console.log(client.computedStyle.cornerRadius.all); // 12
    console.log(client.computedStyle.origin.cornerRadius); // "global"
}
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import StyleProperty

gnoblin = Gnoblin.get()

client = gnoblin.clients.focused
if client is not None:
    print(client.style.is_set(StyleProperty.CORNER_RADIUS))  # False: the client does not override it
    print(client.computed_style.corner_radius.all)  # 12.0
    print(client.computed_style.origin.corner_radius)  # "global"
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::StyleProperty;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(client) = model.clients()?.and_then(|clients| clients.focused().ok().flatten()) else {
        return Ok(());
    };
    println!("{}", client.style()?.is_set(StyleProperty::CornerRadius)?); // false: the client does not override it
    let computed = client.computed_style()?;
    println!("{:?}", computed.corner_radius.all); // Some(12.0)
    println!("{}", computed.origin.corner_radius); // "global"
    Ok(())
}
```

:::

`computedStyle` is a snapshot. `origin` names the scope each property came from: `default`, `global`,
`workspace:<id>`, `where({...})` or `client`.

## Setting, unsetting and no value

::: tabs

```typescript
import "gnoblin-std";

const client = gnoblin.clients.focused;
if (client) {
    client.style.opacity = 0.5; // an override
    client.style.unset("opacity"); // inherit again
    client.style.reset(); // unset every property
    client.style.background = ""; // a value: no background, whatever the scopes above say
    client.style.shadows = []; // a value: no shadows
}
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import StyleProperty

gnoblin = Gnoblin.get()

client = gnoblin.clients.focused
if client is not None:
    client.style.opacity = 0.5  # an override
    client.style.unset(StyleProperty.OPACITY)  # inherit again
    client.style.reset()  # unset every property
    client.style.background = ""  # a value: no background, whatever the scopes above say
    client.style.shadows = []  # a value: no shadows
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::StyleProperty;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(client) = model.clients()?.and_then(|clients| clients.focused().ok().flatten()) else {
        return Ok(());
    };
    let style = client.style()?;
    style.set_opacity(0.5)?; // an override
    style.unset(StyleProperty::Opacity)?; // inherit again
    style.reset()?; // unset every property
    style.set_background("")?; // a value: no background, whatever the scopes above say
    style.set_shadows(&[])?; // a value: no shadows
    Ok(())
}
```

:::

An empty text and an empty list are values of the scope, never "inherit". A list you assign replaces the inherited list
whole.

## Properties

| Property         | Accepts                                   | Default | Drawn                                 |
| ---------------- | ----------------------------------------- | ------- | ------------------------------------- |
| `opacity`        | a number from 0 to 1                      | 1       | by every renderer                     |
| `shadows`        | a list of `Shadow`, at most 8             | none    | outer shadows: GLES and software      |
| `backdropFilter` | a list of filters, at most 8              | none    | a blur: GLES                          |
| `cornerRadius`   | a number of at least 0, or one per corner | 0       | one radius: GLES, software and Vulkan |
| `borders`        | a list of `Border`, at most 8             | none    | GLES and software                     |
| `filter`         | a list of filters, at most 8              | none    | not drawn                             |
| `background`     | a colour or the empty text                | none    | not drawn                             |

A colour is `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`. A property that no renderer draws is still stored and reported
in the computed style; see "What is drawn" below.

### Shadow

- `x` and `y` offset the shadow. Default 0.
- `blur` is the softness of the edge, from 0 to 100. Default 12.
- `spread` grows the shadow, from -100 to 100. Default 0.
- `color` defaults to `#00000080`.
- An `inset` shadow is kept and not drawn.

The first shadow of the list is on top.

### Border

`width` (at least 0, default 1), `color` (default `#ffffff`), `offset` (default 0) and `inset` (default false).

### Corner radius

`cornerRadius` is one number for every corner, or a `CornerRadius` record with `all` or one value for each corner.

- The corners are `topLeft`, `topRight`, `bottomRight` and `bottomLeft`.
- Python spells them `top_left` and so on.
- In Rust each is an `Option<f64>` of the record.

### Filters

A filter is one of five classes: `BlurFilter`, `SaturateFilter`, `BrightnessFilter`, `ContrastFilter` and `ShaderFilter`.

- `BlurFilter` has a `radius` from 0 to 100. Default 12.
- `SaturateFilter`, `BrightnessFilter` and `ContrastFilter` have an `amount` from 0 to 2. Default 1.
- `ShaderFilter` has a WGSL `source` of at most 16 KiB and up to 8 `params`. It is drawn unless the compositor's custom
  shaders are switched off (they are on by default): see [custom shaders](/custom-shaders).

TypeScript and Python also take a shorthand in a list, which becomes the class: an object (a dict in Python) with one key,
the kind of the filter. Rust writes the class and calls `.into()`.

::: tabs

```typescript
import { BlurFilter, SaturateFilter } from "gnoblin-std";

const style = gnoblin.clients.style;
style.backdropFilter = [{ blur: 18 }, { saturate: 1.25 }]; // the shorthand
style.backdropFilter = [new BlurFilter({ radius: 18 }), new SaturateFilter({ amount: 1.25 })]; // the classes
console.log(style.backdropFilter[0] instanceof BlurFilter); // true
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import BlurFilter, SaturateFilter

gnoblin = Gnoblin.get()

style = gnoblin.clients.style
style.backdrop_filter = [{"blur": 18}, {"saturate": 1.25}]  # the shorthand
style.backdrop_filter = [BlurFilter(radius=18), SaturateFilter(amount=1.25)]  # the classes
print(isinstance(style.backdrop_filter[0], BlurFilter))  # True
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::{BlurFilter, Filter, SaturateFilter};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(clients) = model.clients()? else {
        return Ok(());
    };
    let style = clients.style()?;
    style.set_backdrop_filter(&[BlurFilter { radius: 18.0 }.into(), SaturateFilter { amount: 1.25 }.into()])?;
    let first_is_blur = matches!(style.backdrop_filter()?.first(), Some(Filter::Blur(_)));
    println!("{first_is_blur}"); // true
    Ok(())
}
```

:::

## Lists are copies

A list and a record you read from a style are copies. Change one and write it back:

::: tabs

```typescript
import "gnoblin-std";

gnoblin.clients.style.shadows = [{ blur: 10 }];
const shadows = gnoblin.clients.style.shadows;
shadows[0].blur = 20;
gnoblin.clients.style.shadows = shadows; // every client that inherits the list follows
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import Shadow

gnoblin = Gnoblin.get()

gnoblin.clients.style.shadows = [Shadow(blur=10)]
shadows = gnoblin.clients.style.shadows
shadows[0].blur = 20
gnoblin.clients.style.shadows = shadows  # every client that inherits the list follows
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::Shadow;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(clients) = model.clients()? else {
        return Ok(());
    };
    let style = clients.style()?;
    style.set_shadows(&[Shadow {
        blur: 10.0,
        ..Shadow::default()
    }])?;
    let mut shadows = style.shadows()?;
    shadows[0].blur = 20.0;
    style.set_shadows(&shadows)?; // every client that inherits the list follows
    Ok(())
}
```

:::

A style also has a list object for each list: `borderList()`, `shadowList()`, `filterList()` and `backdropFilterList()`
(`border_list()` and so on in Python and Rust).

- A list object has `add`, `insert`, `replace`, `remove(index)`, `clear`, `unset` and `items`, which check and apply at once.
- `add` takes one element and returns it.
- Each writes into the scope's own list. A scope that inherited the list gets its own copy first, so the write reaches that
  scope and nobody else.

For example:

::: tabs

```typescript
import { BlurFilter, Border } from "gnoblin-std";

const style = gnoblin.clients.style;
style.cornerRadius = 12;
style.borderList().add(new Border({ width: 2, color: "#7c3aed" }));
style.shadowList().add({ y: 8, blur: 24, color: "#0006" });
style.backdropFilterList().add(new BlurFilter({ radius: 18 }));
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import BlurFilter, Border, Shadow

gnoblin = Gnoblin.get()

style = gnoblin.clients.style
style.corner_radius = 12
style.border_list().add(Border(width=2, color="#7c3aed"))
style.shadow_list().add(Shadow(y=8, blur=24, color="#0006"))
style.backdrop_filter_list().add(BlurFilter(radius=18))
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::{BlurFilter, Border, CornerRadius, Shadow};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(clients) = model.clients()? else {
        return Ok(());
    };
    let style = clients.style()?;
    style.set_corner_radius(&CornerRadius {
        all: Some(12.0),
        ..CornerRadius::default()
    })?;
    style.border_list()?.add(&Border {
        width: 2.0,
        color: "#7c3aed".to_owned(),
        ..Border::default()
    })?;
    style.shadow_list()?.add(&Shadow {
        y: 8.0,
        blur: 24.0,
        color: "#0006".to_owned(),
        ..Shadow::default()
    })?;
    style.backdrop_filter_list()?.add(&BlurFilter { radius: 18.0 }.into())?;
    Ok(())
}
```

:::

A bad entry and a ninth entry are refused and leave the list as it was.

## Invalid values

An invalid value is refused with the path of the property, and the previous value stays. A TypeScript shell gets a thrown
`GnoblinError`, a Python shell a `GnoblinError` exception and a Rust shell an `Err`:

```text
Client#42.style.borders[2].width: must be finite and >= 0
Client#42.style.shadows: holds at most 8 entries, got 9
```

A window's node carries at most 8 effects (the backdrop blur and the outer shadows together). A style that makes more
is reported when it is applied and the node keeps the effects it had.

## What is drawn

The tree has four effect kinds (blur, shadow, border and shader), a node `corner_radius` and a node opacity. A style becomes:

- `opacity` as the window node's `opacity`.
- `cornerRadius` as the node's `corner_radius`. It rounds the window's whole frame: the content, the titlebar above it,
  the borders and the shadow share one rounded shape, so a rounded decorated window has four rounded corners. One
  radius serves every corner: a per-corner radius is drawn as the largest of the four and reported
  `not applied: style.cornerRadius: ...`.
- The outer `shadows` as `shadow` effects with `frame` set, painted from the last to the first so the first is on top.
  A shadow follows the window's frame, the titlebar included, not the content alone.
- `borders` as `border` effects in list order, after the shadows. A border is a stroke of `width` along the frame's
  rounded edge: outside it by default, inside it with `inset`, moved `offset` pixels from the edge. Borders with
  different offsets make concentric rings.
- `backdropFilter` as one `blur` effect when it has a `BlurFilter` of radius above 0. The compositor blurs at a fixed
  radius of 12, so the radius only switches the blur on. The `SaturateFilter`, `BrightnessFilter` and `ContrastFilter`
  of the chain multiply into the blur's saturation, brightness and contrast, each at most 2. Without a blur in the
  chain they have nothing to draw.
- A `ShaderFilter` of either filter list as one `shader` effect, after the blur and before the shadows, at most four of
  them. While the compositor's custom shaders are switched off it is not written and the style reports it as
  `not applied: style.filter: ...`. The other `filter` elements have no effect kind and are not applied.

| Kind or property | GLES | Software                         | Vulkan                 |
| ---------------- | ---- | -------------------------------- | ---------------------- |
| `corner_radius`  | yes  | yes (hard edge)                  | yes                    |
| `shadow`         | yes  | yes (hard edge, stepped falloff) | no, reported `skipped` |
| `border`         | yes  | yes (hard edge)                  | no, reported `skipped` |
| `blur`           | yes  | no, reported `skipped`           | no, reported `skipped` |

Two decorated windows on the GLES renderer with `cornerRadius: 12`, a 2 pixel inset border and a shadow. The corners are
cut from the frame, the titlebar included, the border runs over the titlebar's top edge and round the arc, and the shadow
surrounds the whole frame:

![Two decorated terminals with rounded corners, a violet border over the titlebar and a shadow on the window beneath](images/js-shell/rounded.png)

The `shader` kind is not in this table: the seat's `effect_kinds` lists it when the active renderer draws it. See
[custom shaders](/custom-shaders).

A node placed in 3D (a rotation about x or y, a `z`, or a perspective camera above it) draws no shadow, border or blur,
and its `skipped` list says `unsupported-effect-on-3d`; its corner radius is still drawn.

What the active renderer does not draw stays on the node, and the node's `skipped` list names it
(`border: unsupported-by-renderer`). The style itself reports only what no renderer could draw. A client reads the warnings
as follows:

- TypeScript: `client.styleWarnings`.
- Python: `client.style_warnings`.
- Rust: `client.style_warnings()?`.

`seat.effect_kinds` lists the kinds the renderer draws.

`background`, the `filter` elements that are not a `ShaderFilter` and inset shadows have no effect kind and are not applied yet. They stay in the computed style, and
each one a style sets is reported once per reload as `not applied: style.<property>: <reason>`.

A window holds at most 16 effects (at most 8 shadows and 8 borders, and the blur).

::: tabs

```typescript
import "gnoblin-std";

gnoblin.clients.style.cornerRadius = 12;
gnoblin.clients.style.borders = [{ width: 1, color: "#ffffff33", inset: true }];
gnoblin.clients.style.shadows = [{ y: 8, blur: 24, color: "#00000066" }];
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import Border, Shadow

gnoblin = Gnoblin.get()

gnoblin.clients.style.corner_radius = 12
gnoblin.clients.style.borders = [Border(width=1, color="#ffffff33", inset=True)]
gnoblin.clients.style.shadows = [Shadow(y=8, blur=24, color="#00000066")]
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::{Border, CornerRadius, Shadow};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(clients) = model.clients()? else {
        return Ok(());
    };
    let style = clients.style()?;
    style.set_corner_radius(&CornerRadius {
        all: Some(12.0),
        ..CornerRadius::default()
    })?;
    style.set_borders(&[Border {
        width: 1.0,
        color: "#ffffff33".to_owned(),
        inset: true,
        ..Border::default()
    }])?;
    style.set_shadows(&[Shadow {
        y: 8.0,
        blur: 24.0,
        color: "#00000066".to_owned(),
        ..Shadow::default()
    }])?;
    Ok(())
}
```

:::

## Reload

A reload runs the new shell in a fresh generation. Style lines you removed are gone:

- The new generation takes the effects, the opacity and the corner radius off the windows the old one styled, with no style
  line of its own to do it.
- A style the new shell sets again is written for each window once, with the new value.
- A new shell with no style of its own takes off only what the old style wrote and the window still holds: an opacity, a
  corner radius or a list of effects the new shell wrote to the window itself stays.
- A new shell that never uses the library leaves no generation to take over, so the old generation's end takes the style
  off the windows itself.
