# Window management settings

The focus, raise, tiling and titlebar-click policy of the compositor has no object yet: see
[settings with no object yet](/js-settings#dropped-settings). The runtime keeps these values, and no shell can change
them:

- Focus follows a click. Moving the pointer does not change focus.
- A new window takes focus, and a window raises when you click it.
- Dropping a window on a screen edge tiles it, and a dragged window stays inside the work area.
- A double click on a stock titlebar toggles maximise, a right click opens the window menu, and a middle click does
  nothing.

The library's focus policy keeps the first two, with no line in the shell, from the moment a shell makes a workspace, a
display or a window frame. A shell that decides focus itself calls `FocusPolicy.get().stop()` first (in Rust,
`FocusPolicy::get(&objects)?.stop()?`).

A client that draws its own titlebar handles those clicks itself, and GTK apps usually do.

## Move and resize with the pointer

`MoveBinding` binds `Super` and a left drag to move a window, and `ResizeBinding` binds `Super` and a right drag to resize
it from the nearest corner. These gestures work on application windows with client or server decorations. The stock
shell installs both. To bind another drag, give the binding a `trigger` and install it:

::: tabs

```typescript
import { FloatingLayout, MoveBinding, ResizeBinding, Workspace, Workspaces } from "gnoblin-std/generated/objects";

Workspaces.get().add(new Workspace("main", { layout: new FloatingLayout() }));
new MoveBinding({ trigger: "Alt+MouseLeft+Drag" }).install();
new ResizeBinding({ trigger: "Alt+MouseRight+Drag" }).install();
```

```python
from gnoblin.objects.std import FloatingLayout, MoveBinding, ResizeBinding, Workspace, Workspaces

Workspaces.get().add(Workspace("main", layout=FloatingLayout()))
MoveBinding(trigger="Alt+MouseLeft+Drag").install()
ResizeBinding(trigger="Alt+MouseRight+Drag").install()
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_std::objects::{
    FloatingLayout, MoveBinding, MoveBindingOptions, ResizeBinding, ResizeBindingOptions, Workspace, WorkspaceOptions,
    Workspaces,
};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let layout = FloatingLayout::new(&objects)?.as_layout()?;
    let workspace = Workspace::new_with(&objects, "main", WorkspaceOptions { name: None, gaps: None, layout: Some(layout) })?;
    Workspaces::get(&objects)?.add(&workspace)?;
    MoveBinding::new_with(
        &objects,
        MoveBindingOptions { priority: None, trigger: Some("Alt+MouseLeft+Drag".into()), snap: None, snap_margin: None },
    )?
    .install(None)?;
    ResizeBinding::new_with(
        &objects,
        ResizeBindingOptions { priority: None, trigger: Some("Alt+MouseRight+Drag".into()), grip: None },
    )?
    .install(None)?;
    Ok(())
}
```

:::

## Workspaces

Declare workspaces when the shell starts, as in [workspaces](/shell-api/workspaces). To place a window on a workspace
when it opens, add a rule: see [window rules](/guides/window_rules).
