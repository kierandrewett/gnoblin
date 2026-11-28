# Outputs

An output is a connected display, real or virtual. Each output shows one workspace of its own, bars sit on one output, and
a window that loses its output moves to another or waits hidden for the next. `gnoblin.outputs` is the live collection of
`Output` objects.

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core";
import { FloatingLayout, OutputAugment, Toast, Workspace, Workspaces } from "gnoblin-std";

const gnoblin = Gnoblin.get();
const workspaces = Workspaces.get();
for (const id of ["1", "2", "3"]) {
    workspaces.add(new Workspace(id, { layout: new FloatingLayout() }));
}
for (const output of gnoblin.outputs.toList()) {
    const shown = OutputAugment.get().workspace(output);
    Toast.show(`${output.name}: ${output.mode} showing ${shown?.id ?? "nothing"}`);
}
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import FloatingLayout, OutputAugment, Toast, Workspace, Workspaces

gnoblin = Gnoblin.get()
workspaces = Workspaces.get()
for workspace_id in ["1", "2", "3"]:
    workspaces.add(Workspace(workspace_id, layout=FloatingLayout()))
for output in gnoblin.outputs.to_list():
    shown = OutputAugment.get().workspace(output)
    Toast.show(f"{output.name}: {output.mode} showing {shown.id if shown else 'nothing'}")
```

```rust
use std::time::Instant;

use gnoblin_core::{self as gnoblin, Gnoblin};
use gnoblin_std::GnoblinStdExt;
use gnoblin_std::ObjectsStdExt;
use gnoblin_std::{FloatingLayout, OutputStdExt, Toast, Workspace, WorkspaceOptions};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.objects()?.std()?;
    let model = Gnoblin::get(&g.objects()?)?;
    let workspaces = model.workspaces()?;
    for id in ["1", "2", "3"] {
        let options = WorkspaceOptions {
            layout: Some(FloatingLayout::new(&std)?.as_layout()?),
            ..WorkspaceOptions::default()
        };
        workspaces.add(&Workspace::new_with(&std, id, options)?)?;
    }
    let outputs = model.outputs()?;
    for output in outputs.to_list()? {
        let shown = match output.workspace()? {
            Some(workspace) => workspace.id()?,
            None => "nothing".to_owned(),
        };
        let text = format!("{}: {} showing {shown}", output.name()?, output.mode()?);
        let started = Instant::now();
        let clock = move || Ok(started.elapsed().as_secs_f64() * 1000.0);
        Toast::show(&std, &text, None, None, None, Some(clock))?;
    }
    Ok(())
}
```

:::

Put the TypeScript in `~/.config/gnoblin/shell.ts`, the Python in `~/.config/gnoblin/shell.py`, and the Rust in a shell
library, which [Write a shell in Rust](/rust-shell) builds and runs. A message names each output, its mode and the
workspace it shows.

## Read outputs

`gnoblin.outputs` is a live, read-only collection with one `Output` for each screen the compositor publishes. An output is
added when it is plugged in and removed when it goes. Every read of one output returns the same object, so you can keep it.
A removed output keeps its last values and reads `alive === false`.

The collection has `length`, `at(index)`, `get(id)` and `contains(output)`. `toList()` returns the outputs as an array,
which `find`, `filter`, `map` and `for...of` work on.

`gnoblin.outputs.where({ primary: true })` selects live, as the other collections do, on `id`, `name`, `make`, `model`,
`mode`, `scale`, `primary`, `enabled`, `x`, `y`, `width` and `height`. [`gnoblin.find("Output", filter)`](find) returns an
array.

| Member                      | Meaning                                                                                                                                    |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `name`                      | The compositor's name for the output, such as `output-0`. Not a stable name: use `connector`.                                              |
| `connector`                 | The connector the display is plugged into, such as `DP-1`. An `OutputProfile` matches it.                                                  |
| `identity`                  | The monitor's EDID identity, `MAN:product:serial`. Empty for a display that reports none.                                                  |
| `make`, `model`             | What the compositor knows of the display. Empty when it knows nothing.                                                                     |
| `id`                        | The output's number in the compositor's tree, as text. `Display` selectors and `Bar` take it as a number.                                  |
| `mirrorOf`                  | The output this one shows a copy of, or `undefined`.                                                                                       |
| `mode`                      | The mode in use, `WxH@Hz`.                                                                                                                 |
| `scale`                     | The factor the output composes at. 2 is a HiDPI panel.                                                                                     |
| `supersample`               | The factor the output is composed above its mode, 1 for none. See [Supersampling](#supersampling).                                         |
| `virtualSize`               | `{ width, height }`: the mode times `supersample`, the size of the output's screenshots.                                                   |
| `effectiveScale`            | `scale * supersample`: the scale clients are told.                                                                                         |
| `gpuMemory`                 | `{ target, scratch, staging, views, capture, total }` in bytes: what the compositor holds for the output. See [GPU memory](../gpu-memory). |
| `transform`                 | The rotation and flip: `normal`, `rotate-90`, `rotate-180`, `rotate-270`, `flipped`, `flipped-90`, `flipped-180` or `flipped-270`.         |
| `x`, `y`, `width`, `height` | The output's rectangle in the compositor's coordinates. `bounds` is the same as one object.                                                |
| `primary`                   | Whether this is the primary output, the one the stock bars and `gnoblin.workspaces.area` are on.                                           |
| `enabled`                   | Whether the output is on.                                                                                                                  |
| `workspace`                 | The workspace the output shows, or `undefined`: `OutputAugment.get().workspace(output)`.                                                   |
| `show(workspace, options?)` | Shows a workspace on this output. `options.duration` slides the windows.                                                                   |
| `apply(options)`            | Configures the display. See [Configure an output](#configure-an-output).                                                                   |
| `mirror(source?)`           | Shows a copy of `source`, or stops when it is left out.                                                                                    |
| `alive`                     | `false` once the display is gone.                                                                                                          |
| `addListener(listener)`     | Attaches an `OutputListener`.                                                                                                              |

`gpuMemory` also carries four numbers about the whole session, the same on every output: `clients` (the bytes of every
client's live buffers), `budget` (the memory budget in bytes, 0 when it is off), `refused` (the requests refused for
passing a bound, a count) and `evicted` (the bytes given back from caches to make room). They are in
[GPU memory](../gpu-memory#the-budget).

A script reads these properties and writes none. It changes a display with `apply` and `mirror`, below, or keeps a
setting for a monitor with an [`OutputProfile`](#hardware-profiles) added by `DisplayManager.addProfile` (`add_profile` in Python and Rust).

Each language spells the same members its own way:

- Python uses `snake_case` and attributes: `output.mirror_of`, `output.virtual_size`, `gnoblin.outputs.to_list()`,
  `gnoblin.outputs.get(id)` and `gnoblin.outputs.where(OutputQuery(primary=True))`. `apply` takes an `OutputConfig`, whose
  keys are `position_x` and `position_y`.
- Rust has a getter for each property, and every call returns a `Result`: `output.mirror_of()?`, `outputs.to_list()?` and
  `outputs.where_(&query, None)?`. `apply` takes an `OutputConfig` struct, whose keys are `Option` fields such as
  `position_x: Some(0)`.

The [API reference](/reference) lists every member in each language.

## Configure an output

`output.apply({ ... })` asks the compositor to change one display. Every key is optional, and a key left out keeps its
value.

- The compositor checks the whole new arrangement before it touches hardware.
- It then drives the changed outputs. If the backend refuses a mode, the outputs already changed are put back and the
  last good arrangement stays.
- It keeps the result as that monitor's profile, so the same monitor on the same port gets it back after an unplug.

| Key                      | Values                                                                                            |
| ------------------------ | ------------------------------------------------------------------------------------------------- |
| `mode`                   | `"3440x1440@144"`, or `"preferred"` to keep what the backend drives.                              |
| `scale`                  | A number from 0.5 to 4.                                                                           |
| `supersample`            | A number from 1 to 16: compose at this multiple of the mode. See [Supersampling](#supersampling). |
| `transform`              | `"normal"`. The compositor draws no rotation, so the others are refused by name.                  |
| `positionX`, `positionY` | Whole numbers: the top-left corner in the one desktop coordinate space. Give both.                |
| `enabled`                | `false` switches the output off: it shows black and has no place in the desktop.                  |
| `mirrorOf`               | The connector to show an exact copy of. An empty string stops mirroring.                          |

A value of the wrong shape throws before anything is sent, naming the key: `Output#output-1.apply.scale must be a number
from 0.5 to 4`.

The compositor answers a frame later. `apply` and `mirror` return an `OutputOperation`, like the other calls that finish
later. Its `status` is `pending`, `succeeded` or `failed`, and `then(fn)` and `catch(fn)` run a function when it settles.

- It succeeds, and its `output` is the output.
- It fails with the compositor's own words as `error`, and a `code`. A refusal names the key, as in
  `position: HDMI-1 and DP-1 would overlap`.

The `code` says what kind of refusal it was:

- `unsupported`: the compositor cannot do what the key asks, a `transform` other than `normal`.
- `not_found`: the output went away before the answer, or the compositor does not have it connected.
- `denied`: the compositor refused the arrangement and kept the last good one, a mode, a position or a mirror.
- `read_only`: the compositor refused to let the shell write the node, as it refuses a property write.

An `OutputListener` hears the same answer in `onApplied(output, error)`, where `error` is the words, or empty when the change
took effect.

**Mode and scale.** Drive an output at another mode, composed at a fractional scale, and read what happened:

::: tabs

```typescript
import { Gnoblin, OutputConfig, OutputQuery } from "gnoblin-core";

const gnoblin = Gnoblin.get();
gnoblin.outputs
    .where(new OutputQuery({ name: "output-1" }))
    .at(0)
    ?.apply(new OutputConfig({ mode: "2560x1440@60", scale: 1.25 }))
    .catch((error, code) => {
        gnoblin.spawn(["notify-send", `Display change refused (${code}): ${error}`]);
    });
```

```python
from gnoblin.objects.model import Gnoblin, OutputConfig, OutputQuery

gnoblin = Gnoblin.get()


def refused(error, code):
    gnoblin.spawn(["notify-send", f"Display change refused ({code}): {error}"])


output = gnoblin.outputs.where(OutputQuery(name="output-1")).at(0)
if output:
    output.apply(OutputConfig(mode="2560x1440@60", scale=1.25)).catch(refused)
```

```rust
use std::rc::Rc;

use gnoblin_core::{Gnoblin, OutputConfig, OutputQuery};
use gnoblin_core::{self as gnoblin};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let model = Rc::new(Gnoblin::get(&g.objects()?)?);
    let outputs = model.outputs()?;
    let query = OutputQuery {
        name: Some("output-1".to_owned()),
        ..OutputQuery::default()
    };
    let Some(output) = outputs.where_(&query, None)?.at(0)? else {
        return Ok(());
    };
    let config = OutputConfig {
        mode: Some("2560x1440@60".to_owned()),
        scale: Some(1.25),
        ..OutputConfig::default()
    };
    let notifier = Rc::clone(&model);
    output.apply(&config)?.catch(move |error, code| {
        let text = format!("Display change refused ({code}): {error}");
        notifier.spawn(&["notify-send".to_owned(), text], None)?;
        Ok(())
    })?;
    Ok(())
}
```

:::

**Position.** Put an output below another. Outputs share one coordinate space, so a window can straddle the seam:

::: tabs

```typescript
import { Gnoblin, OutputConfig } from "gnoblin-core";

Gnoblin.get()
    .outputs.at(1)
    ?.apply(new OutputConfig({ positionX: 0, positionY: 1080 }));
```

```python
from gnoblin.objects.model import Gnoblin, OutputConfig

output = Gnoblin.get().outputs.at(1)
if output:
    output.apply(OutputConfig(position_x=0, position_y=1080))
```

```rust
use gnoblin_core::{Gnoblin, OutputConfig};
use gnoblin_core::{self as gnoblin};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let outputs = model.outputs()?;
    if let Some(output) = outputs.at(1)? {
        let config = OutputConfig {
            position_x: Some(0),
            position_y: Some(1080),
            ..OutputConfig::default()
        };
        output.apply(&config)?;
    }
    Ok(())
}
```

:::

**Mirror.** Show the first output on the second, scaled to fit and letterboxed when the shape differs. The pair is one
monitor: the mirrored output has no place of its own, no workspace, and the pointer has one monitor for both.

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core";

const outputs = Gnoblin.get().outputs;
outputs.at(1)?.mirror(outputs.at(0));
outputs.at(1)?.mirror(); // stop
```

```python
from gnoblin.objects.model import Gnoblin

outputs = Gnoblin.get().outputs
first, second = outputs.at(0), outputs.at(1)
if first and second:
    second.mirror(first)
    second.mirror()  # stop
```

```rust
use gnoblin_core::{self as gnoblin, Gnoblin};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let outputs = model.outputs()?;
    if let (Some(first), Some(second)) = (outputs.at(0)?, outputs.at(1)?) {
        second.mirror(Some(&first))?;
        second.mirror(None)?; // stop
    }
    Ok(())
}
```

:::

**Off.** Switch an output off. It keeps its node, so you can switch it back on:

::: tabs

```typescript
import { Gnoblin, OutputConfig } from "gnoblin-core";

Gnoblin.get()
    .outputs.at(1)
    ?.apply(new OutputConfig({ enabled: false }));
```

```python
from gnoblin.objects.model import Gnoblin, OutputConfig

output = Gnoblin.get().outputs.at(1)
if output:
    output.apply(OutputConfig(enabled=False))
```

```rust
use gnoblin_core::{Gnoblin, OutputConfig};
use gnoblin_core::{self as gnoblin};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let outputs = model.outputs()?;
    if let Some(output) = outputs.at(1)? {
        let config = OutputConfig {
            enabled: Some(false),
            ..OutputConfig::default()
        };
        output.apply(&config)?;
    }
    Ok(())
}
```

:::

**Span.** The desktop already is the span: every output is a rectangle in one space and a window across the seam is drawn
continuously, at any mix of scales. By default each output shows a workspace of its own. A [`span` display](#displays-and-viewports)
shows one workspace over several outputs.

**In your file.** An `OutputProfile` applies the same keys by connector when the file loads, and again when that display is
plugged in: see [Hardware profiles](#hardware-profiles). One the compositor refuses is reported as a diagnostic naming
`outputs.profiles.<id>.<key>`.

### Supersampling

`supersample` renders an output at a multiple of its mode and averages the result down, like macOS's scaled resolutions.

- The compositor composes the output on a virtual surface of `mode * supersample` pixels, tells clients the scale
  `scale * supersample`, and shows the result at the real mode, averaged down.
- The logical size (`mode / scale`), window geometry and pointer do not change, so the desktop looks the same,
  anti-aliased and crisp, and screenshots and captures come out hi-res.
- Wayland clients draw `supersample` times the pixels. Xwayland's client is given the same factor, so an X11 window keeps
  its logical size and its toolkit is told to draw `supersample` times the pixels through `Gdk/WindowScalingFactor` and
  `Xft/DPI`. A window that is already open keeps the pixels it has until its client redraws or is reopened.

- **Supported factors.** 1, 2 and 4. Any other number from 1 to 16 is accepted when `mode * supersample` is a whole size
  and logs one notice, `supersample N is not recommended: use 1, 2 or 4`; the downsample is then bilinear. In a script the
  notice is the log line `output.apply` writes, and the same factor asked for again is not reported twice.
- **Refused by name.** A value that is not a number from 1 to 16 throws before anything is sent, naming the key. A factor
  that gives a fractional size (1.5 on a 1367 pixel wide mode) or a surface past the renderer's limit (4 on 3840x2160 is
  15360 pixels wide, over the 8192 limit) is refused by the compositor with the numbers, and the output keeps what it had.
- **What `wl_output` and `xdg_output` say.** The mode stays the real one, `scale` is the next whole number above
  `scale * supersample`, the preferred fractional scale is exact, and the logical size does not change.

::: tabs

```typescript
import { Gnoblin, OutputConfig, OutputQuery } from "gnoblin-core";

const output = Gnoblin.get()
    .outputs.where(new OutputQuery({ name: "output-1" }))
    .at(0);
if (output) {
    output.apply(new OutputConfig({ supersample: 1 })); // the default: composed at the mode, nothing averaged
    output.apply(new OutputConfig({ supersample: 2 })); // 1920x1080 composed at 3840x2160; output.virtualSize is { width: 3840, height: 2160 }
    output.apply(new OutputConfig({ supersample: 4 })); // 1920x1080 composed at 7680x4320: a GPU with 8192 texels per edge is needed
}
```

```python
from gnoblin.objects.model import Gnoblin, OutputConfig, OutputQuery

output = Gnoblin.get().outputs.where(OutputQuery(name="output-1")).at(0)
if output:
    output.apply(OutputConfig(supersample=1))  # the default: composed at the mode, nothing averaged
    output.apply(OutputConfig(supersample=2))  # 1920x1080 composed at 3840x2160; output.virtual_size is 3840x2160
    output.apply(OutputConfig(supersample=4))  # 1920x1080 composed at 7680x4320: a GPU with 8192 texels per edge is needed
```

```rust
use gnoblin_core::{Gnoblin, OutputConfig, OutputQuery};
use gnoblin_core::{self as gnoblin};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let outputs = model.outputs()?;
    let query = OutputQuery {
        name: Some("output-1".to_owned()),
        ..OutputQuery::default()
    };
    let Some(output) = outputs.where_(&query, None)?.at(0)? else {
        return Ok(());
    };
    let with = |supersample: f64| OutputConfig {
        supersample: Some(supersample),
        ..OutputConfig::default()
    };
    output.apply(&with(1.0))?; // the default: composed at the mode, nothing averaged
    output.apply(&with(2.0))?; // 1920x1080 composed at 3840x2160; output.virtual_size() is 3840x2160
    output.apply(&with(4.0))?; // 1920x1080 composed at 7680x4320: a GPU with 8192 texels per edge is needed
    Ok(())
}
```

:::

From a terminal every key is optional and a key left out keeps its value, so a call names only what it changes:

```sh
gnoblinctl output call <id> apply supersample=1   # no supersampling
gnoblinctl output call <id> apply supersample=2   # twice the mode
gnoblinctl output call <id> apply supersample=4   # four times the mode
gnoblinctl output get <id> supersample            # what it is now
```

**What a screenshot looks like.** A screenshot or capture of a supersampled output is the composed frame, so it has
`supersample` times the pixels of the display on each axis, with the secure-content masks applied before anything is read:

| Factor | On a 1920x1080 mode | What it holds                                                                          |
| ------ | ------------------- | -------------------------------------------------------------------------------------- |
| 1      | 1920x1080           | The frame the display shows.                                                           |
| 2      | 3840x2160           | Four rendered pixels for each displayed one: text and edges are crisp.                 |
| 4      | 7680x4320           | Sixteen rendered pixels for each displayed one; refused where the limit is under 7680. |

`wlr-screencopy`, `ext-image-copy-capture`, the PipeWire export and `capture.screen` all advertise that size. The payload
of `capture.screen` takes `display` for the frame as the display shows it, at the mode, which is the hi-res frame
averaged down; use it when a consumer wants the picture the monitor shows rather than the sharpest one.

- **Software and GLES on a virtual output.** The hi-res frame is kept and read back as it is.
- **Nested.** The frame is averaged into the parent's buffer, which stays the size of the parent's window; a parent that
  rounded a fractional scale (no viewport) has a buffer that is not the mode, and the factor is refused by name.
- **Hardware (DRM).** The output composes on a persistent virtual surface and a box-filter pass fills the scan-out buffer.
  The DRM backend keeps no readable frame of an output, so there is no screenshot or capture of a hardware output at any
  factor; that limit is older than this setting and unchanged by it.
- **Vulkan.** The offscreen Vulkan renderer draws at any compositing scale into an image of any size the device allows, so
  it takes a factor the same way; the device's `maxImageDimension2D` is its limit and a factor past it is refused with
  the numbers.

### What each backend applies

The display backend decides what it can drive. Position, mirror and off need no modeset and work on every backend.
Changing a mode or scale works on the virtual (headless) backend and on a hardware (DRM) output. The nested backend, a
window on another desktop, refuses both by name, and takes a supersample factor (see [Supersampling](#supersampling)).

On a hardware output:

- **Off until you ask.** A mode change on a hardware output is off. It is refused with an error that names
  `GNOBLIN_DRM_MODESET`, and the output keeps its mode. Start the session with `GNOBLIN_DRM_MODESET=1` to allow it
  (`true`, `yes` and `on` also count; anything else, or no variable, leaves it off). A config file or a script cannot
  turn it on.
- **Mode.** The mode must be one the monitor lists, with the refresh rate the kernel computes for it (`2560x1440@59.95`
  and `2560x1440@60` are different modes). The screen goes black for the length of the modeset.
- **Refused.** A mode the monitor does not list, a mode the kernel refuses, and an output that is off are all refused.
  The output keeps its mode, its picture and its colour settings.
- **Several outputs.** A configuration of several outputs is tested on all of them before the first is driven.
- **Scale.** Changing only the scale commits nothing: the output composes at the new scale from its next frame and does
  not blank.
- **Kept across a mode change.** The night-light gamma table is written again after the modeset, and variable refresh
  and the other output controls are left as they were.
- **Not applied.** A transform other than `normal` is refused on every backend: no renderer draws a rotation.

Turn the mode change on, check it on your own display before relying on it, and report a driver that refuses it.

The `wlr-output-management` protocol (`wlr-randr`, `kanshi`) and `gnoblinctl output call <id> apply` go through the same
engine as `apply`: the same checks, the same test before the first output is driven and the same rollback.

`output.apply` takes the configuration of one output, and each key is optional:

- **Keys.** `positionX`, `positionY`, `scale`, `transform`, `enabled`, `mode` and `supersample`. A key left out keeps the value the
  output has, as a key left out of a shell's `screen.apply` does, so `apply supersample=2` changes only the factor and
  `apply scale=1` keeps the factor. A call with no key changes nothing.
- **Position.** `positionX` and `positionY` are one setting. Given only one, the other is the output's current
  coordinate.
- **Mode.** An empty `mode` keeps the mode the output has, as leaving it out does.
- **Strict.** A name that is not a key is refused, and so is a value of the wrong type or of a range the engine
  does not take (a `supersample` under 1, a `scale` outside 0.5 to 4). In each case nothing is applied.
- **Primary.** `primary` is not a key: the layout derives the primary output. A call that carries it is refused with
  an error that names `primary`, and nothing is applied. In a script, `output.apply({ primary: true })` throws a
  `TypeError` that names it.
- **Refusal.** The call completes with `mode-refused`, `scale-refused`, `position-refused`, `enabled-refused`,
  `transform-refused`, `supersample-refused` or `refused`.

## Hardware profiles

An `OutputProfile` is the persistent setting of one monitor: which monitor it is for (the first argument) and the mode,
scale, transform and place it should have (its `settings`). Add it with `DisplayManager.get().addProfile(profile)` (`add_profile` in Python and Rust).

A profile is inert while no matching output is connected. When one is, the profile applies as one request, and it applies
again when the monitor comes back after an unplug.

::: tabs

```typescript
import { DisplayManager, OutputMatch, OutputPosition, OutputProfile, ProfileSettings } from "gnoblin-std";

const manager = DisplayManager.get();
const main = new OutputProfile(new OutputMatch({ connector: "DP-1" }), {
    settings: new ProfileSettings({
        mode: "3440x1440@144",
        scale: 1.25,
        position: new OutputPosition({ kind: "absolute", x: 0, y: 0 }),
    }),
});
const side = new OutputProfile(new OutputMatch({ connector: "HDMI-A-1" }), {
    settings: new ProfileSettings({
        mode: "preferred",
        position: new OutputPosition({ kind: "right_of", profile: main, align: "top" }),
    }),
});
manager.addProfile(main);
manager.addProfile(side);
```

```python
from gnoblin.objects.std import (
    DisplayManager,
    OutputMatch,
    OutputPosition,
    OutputProfile,
    PositionKind,
    ProfileSettings,
)

manager = DisplayManager.get()
main = OutputProfile(
    OutputMatch(connector="DP-1"),
    settings=ProfileSettings(
        mode="3440x1440@144",
        scale=1.25,
        position=OutputPosition(PositionKind.ABSOLUTE, x=0, y=0),
    ),
)
side = OutputProfile(
    OutputMatch(connector="HDMI-A-1"),
    settings=ProfileSettings(
        mode="preferred",
        position=OutputPosition(PositionKind.RIGHT_OF, profile=main, align="top"),
    ),
)
manager.add_profile(main)
manager.add_profile(side)
```

```rust
use gnoblin_core::{self as gnoblin, Gnoblin};
use gnoblin_std::GnoblinStdExt;
use gnoblin_std::ObjectsStdExt;
use gnoblin_std::{
    DisplayManager, OutputMatch, OutputPosition, OutputProfile, OutputProfileOptions, PositionKind, ProfileSettings,
};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.objects()?.std()?;
    let manager = DisplayManager::get(&std)?;

    let mut origin = OutputPosition::new(PositionKind::Absolute);
    origin.x = Some(0);
    origin.y = Some(0);
    let main_profile = OutputProfile::new_with(
        &std,
        &OutputMatch {
            connector: Some("DP-1".to_owned()),
            ..OutputMatch::default()
        },
        OutputProfileOptions {
            id: None,
            settings: Some(ProfileSettings {
                mode: Some("3440x1440@144".to_owned()),
                scale: Some(1.25),
                position: Some(origin),
                ..ProfileSettings::default()
            }),
        },
    )?;
    manager.add_profile(&main_profile)?;

    let mut beside = OutputPosition::new(PositionKind::RightOf);
    beside.profile = Some(main_profile);
    beside.align = Some("top".to_owned());
    let side = OutputProfile::new_with(
        &std,
        &OutputMatch {
            connector: Some("HDMI-A-1".to_owned()),
            ..OutputMatch::default()
        },
        OutputProfileOptions {
            id: None,
            settings: Some(ProfileSettings {
                mode: Some("preferred".to_owned()),
                position: Some(beside),
                ..ProfileSettings::default()
            }),
        },
    )?;
    manager.add_profile(&side)?;
    Ok(())
}
```

:::

| Argument or key | Values                                                                                                              |
| --------------- | ------------------------------------------------------------------------------------------------------------------- |
| first argument  | `connector`, `make`, `model` and `serial`, all that you give. `serial` is the third part of `identity`.             |
| `id`            | An option: the profile's name in the collection. Default: the match written out, such as `DP-1`.                    |
| `mode`          | A setting: `"preferred"` or `"WIDTHxHEIGHT@HERTZ"`, such as `"3440x1440@144"`.                                      |
| `scale`         | A setting: a number from 0.5 to 4.                                                                                  |
| `transform`     | A setting: `"normal"`: the others are refused, as for `output.apply`.                                               |
| `position`      | A setting: `{ kind: "absolute", x, y }`, or a `kind` of `right_of`, `left_of`, `above` or `below` with a `profile`. |
| `enabled`       | A setting: `false` switches the output off.                                                                         |
| `vrr`           | A setting, not applied yet: accepted and reported, because the output request has no variable refresh rate key.     |

- **By hardware.** `new OutputProfile({ make: "BOE", model: "Panel", serial: "SN42" })` finds a monitor whatever port it is on.
  `output.make`, `output.model` and `output.identity` show what a connected display reports.
- **Next to another.** A relative position places the output against the other profile's output.
    - The neighbour's own settings go first, and the position is sent once its answer is in, so a mode or scale change
      moves what is next to it.
    - `align` is `top`, `center` or `bottom` beside the neighbour, and `left`, `center` or `right` above or below it.
    - A neighbour that is not connected leaves the position unapplied, and a notice says so.
    - Placements that depend on themselves throw.
- **Refusal.** The compositor checks the whole arrangement, so an overlap, a gap or a mode the backend refuses leaves the
  last good one. The refusal is reported as a diagnostic naming `outputs.profiles.<id>.<key>`.
- **Changing and removing.** Assign `profile.settings` later, and the change applies at the next frame.
  Removing a profile stops the shell from applying it; the compositor keeps the arrangement it applied last.

## Displays and viewports

An `Output` is a connector. A `Display` is what the desktop presents on one or several outputs, under one `Viewport`
whose `content` is the workspace shown. `gnoblin.displays` is the live collection.

- The shell keeps one `single` display for every output that none of your displays presents, so each monitor is always
  in the collection.
- `gnoblin.input.primary.activeDisplay` is the display under the pointer.

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core";
import { InputContextAugment, Workspace, Workspaces } from "gnoblin-std";

const gnoblin = Gnoblin.get();
const workspaces = Workspaces.get();
const desktop = new Workspace("desktop");
const web = new Workspace("web");
workspaces.add(desktop);
workspaces.add(web);

const seat = gnoblin.input.primary;
seat.bind("Super+2", () => {
    const display = InputContextAugment.get().activeDisplay(seat);
    if (display?.viewport) display.viewport.content = web;
});
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import InputContextAugment, Workspace, Workspaces

gnoblin = Gnoblin.get()
workspaces = Workspaces.get()
desktop = Workspace("desktop")
web = Workspace("web")
workspaces.add(desktop)
workspaces.add(web)

seat = gnoblin.input.primary


def show_web(event):
    display = InputContextAugment.get().active_display(seat)
    if display and display.viewport:
        display.viewport.content = web


seat.bind("Super+2", show_web)
```

```rust
use gnoblin_core::{self as gnoblin, Gnoblin};
use gnoblin_std::GnoblinStdExt;
use gnoblin_std::ObjectsStdExt;
use gnoblin_std::{InputContextStdExt, Workspace};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.objects()?.std()?;
    let model = Gnoblin::get(&g.objects()?)?;
    let workspaces = model.workspaces()?;
    let desktop = Workspace::new(&std, "desktop")?;
    let web = Workspace::new(&std, "web")?;
    workspaces.add(&desktop)?;
    workspaces.add(&web)?;

    let input = model.input()?;
    let seat = input.primary()?;
    seat.bind(
        "Super+2",
        Some(move |_| {
            let input = model.input()?;
            let seat = input.primary()?;
            if let Some(display) = seat.active_display()? {
                if let Some(viewport) = display.viewport()? {
                    viewport.set_content(&web)?;
                }
            }
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

Assigning `viewport.content` shows the workspace on that display, as `gnoblin.workspaces.show` does. A workspace is on one
display at a time: assigning one that another display shows moves it, and that display takes the workspace this one
showed. Reading `content` gives what the display shows now. Inside a `gnoblin.requestTransition` callback the assignment
is part of the transition, and a failed callback puts it back.

`new Display(selectors, { id, mode, viewport })` takes selectors: `{ profile }` for an `OutputProfile` (persistent: the
display is parked while no matching output is connected and keeps its workspace) or `{ output }` for a connected output's
number:

| `mode`     | What it presents                                                                                         |
| ---------- | -------------------------------------------------------------------------------------------------------- |
| `"single"` | One output (the default).                                                                                |
| `"mirror"` | The first output's picture on the others, through the compositor's mirror (scaled to fit, letterboxed).  |
| `"span"`   | One workspace laid out over the bounding box of the outputs' work areas. A window may straddle the seam. |

::: tabs

```typescript
import {
    Camera2D,
    Display,
    DisplayManager,
    DisplaySelector,
    OutputMatch,
    OutputProfile,
    Viewport,
    Workspace,
    Workspaces,
} from "gnoblin-std";

const manager = DisplayManager.get();
const main = new OutputProfile(new OutputMatch({ connector: "DP-1" }));
const side = new OutputProfile(new OutputMatch({ connector: "HDMI-A-1" }));
manager.addProfile(main);
manager.addProfile(side);

const desktop = new Workspace("desktop");
Workspaces.get().add(desktop);

const spanned = new Display([new DisplaySelector({ profile: main }), new DisplaySelector({ profile: side })], {
    mode: "span",
    viewport: new Viewport({ content: desktop, camera: new Camera2D() }),
});
manager.addDisplay(spanned);
```

```python
from gnoblin.objects.std import (
    Camera2D,
    Display,
    DisplayManager,
    DisplaySelector,
    OutputMatch,
    OutputProfile,
    PresentMode,
    Viewport,
    Workspace,
    Workspaces,
)

manager = DisplayManager.get()
main = OutputProfile(OutputMatch(connector="DP-1"))
side = OutputProfile(OutputMatch(connector="HDMI-A-1"))
manager.add_profile(main)
manager.add_profile(side)

desktop = Workspace("desktop")
Workspaces.get().add(desktop)

spanned = Display(
    [DisplaySelector(profile=main), DisplaySelector(profile=side)],
    mode=PresentMode.SPAN,
    viewport=Viewport(content=desktop, camera=Camera2D()),
)
manager.add_display(spanned)
```

```rust
use gnoblin_core::{self as gnoblin, Gnoblin};
use gnoblin_std::GnoblinStdExt;
use gnoblin_std::ObjectsStdExt;
use gnoblin_std::{
    Camera2D, Display, DisplayManager, DisplayOptions, DisplaySelector, OutputMatch, OutputProfile, PresentMode,
    Viewport, ViewportOptions, Workspace, Workspaces,
};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.objects()?.std()?;
    let manager = DisplayManager::get(&std)?;
    let mut selectors = Vec::new();
    for connector in ["DP-1", "HDMI-A-1"] {
        let profile = OutputProfile::new(
            &std,
            &OutputMatch {
                connector: Some(connector.to_owned()),
                ..OutputMatch::default()
            },
        )?;
        manager.add_profile(&profile)?;
        selectors.push(DisplaySelector {
            profile: Some(profile),
            output: None,
        });
    }

    let desktop = Workspace::new(&std, "desktop")?;
    Workspaces::get(&std)?.add(&desktop)?;

    let viewport = Viewport::new_with(
        &std,
        ViewportOptions {
            content: Some(desktop),
            camera: Some(Camera2D::new(&std)?.as_scene_node()?),
            clear_color: None,
        },
    )?;
    let spanned = Display::new_with(
        &std,
        &selectors,
        DisplayOptions {
            id: None,
            mode: Some(PresentMode::Span),
            viewport: Some(viewport),
        },
    )?;
    manager.add_display(&spanned)?;
    Ok(())
}
```

:::

- **One display per output.** `gnoblin.displays.add` throws a `GnoblinError`, naming both displays, when one of the
  outputs belongs to a display of yours already. `gnoblin.displays.replace(old, new)` swaps them as one change: it
  removes `old` and every other display of yours that shares an output with `new`, then adds `new` in `old`'s place.
- **Removing.** `gnoblin.displays.remove(display)` ends the mirror or span the display made; each of its outputs gets a
  display of the shell's again. A display of the shell's own cannot be removed.
- **A span's area.** The bounding box of unequal outputs has corners that no output shows; a window a layout puts there
  is not visible. Maximise and fullscreen use the output under the window.
- **A mirror** ends with the display. A reload whose config does not declare it does not undo it: apply
  `output.mirror(null)` for that.

## The display manager

`DisplayManager.get()` is the service behind `gnoblin.displays` and the output profiles. It is for a shell that wants
the lists and the events in one place.

- It holds the lists `displays` and `profiles`.
    - Change them with `addProfile`, `removeProfile`, `addDisplay`, `removeDisplay`, `replaceDisplay` and `clearDisplays`.
    - Look one up with `profileNamed`, `displayNamed` or `displayAt(x, y)`.
    - `changed` fires when a list changes or a display presents other outputs.
    - `applied(output, error)` fires when the compositor answers a profile's request.
    - `noted(text)` carries each sentence of the diagnostics.
- `OutputProfile.formatMode(width, height, refreshRate)` writes a mode from its parts.
    - `id` is the match written out until you set one. It cannot change while the profile is in the manager.
- A `Display` has `outputs`, which lists the nodes it presents now, and `implicit`, which is true for a display the library
  made.
- `Viewport` has `content`, `camera` and `clearColor`, and `display` is the display that shows it. Assigning `content` on a display's viewport
  shows the workspace there, as `workspace.show` does.

The checks refuse a scale outside 0.5 to 4, a match with no key, a display with nothing to present, a second display over
an output that has one, and a position that depends on itself.

Example 33 lists the displays, applies a profile, adds a display of its own and removes it.

## Cameras

A viewport looks at its content through a camera, `Camera2D` or `Camera3D`. Both are scene nodes, so a
`gnoblin.requestTransition` can animate their properties.

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core";
import { Camera2D, Camera3D, GnoblinAugment, InputContextAugment } from "gnoblin-std";

const gnoblin = Gnoblin.get();
const seat = gnoblin.input.primary;
const flat = new Camera2D();
const deep = new Camera3D({ projection: "perspective", fov: 65, position: { x: 0, y: 0, z: 1600 } });
let threeD = false;
seat.bind("Super+Tab", () => {
    threeD = !threeD;
    GnoblinAugment.get().requestTransition(
        () => {
            const display = InputContextAugment.get().activeDisplay(seat);
            if (display?.viewport) display.viewport.camera = threeD ? deep : flat;
        },
        { duration: 750 },
    );
});
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import (
    Camera2D,
    Camera3D,
    GnoblinAugment,
    InputContextAugment,
    Projection,
    TransitionOptions,
    Vec3,
)

gnoblin = Gnoblin.get()
seat = gnoblin.input.primary
flat = Camera2D()
deep = Camera3D(projection=Projection.PERSPECTIVE, fov=65, position=Vec3(x=0, y=0, z=1600))
three_d = False


def switch_camera():
    display = InputContextAugment.get().active_display(seat)
    if display and display.viewport:
        display.viewport.camera = deep if three_d else flat


def toggle(event):
    global three_d
    three_d = not three_d
    GnoblinAugment.get().request_transition(switch_camera, TransitionOptions(duration=750))


seat.bind("Super+Tab", toggle)
```

```rust
use std::rc::Rc;

use gnoblin_core::{self as gnoblin, Gnoblin};
use gnoblin_std::GnoblinStdExt;
use gnoblin_std::ObjectsStdExt;
use gnoblin_std::{
    Camera2D, Camera3D, Camera3DOptions, InputContextStdExt, Projection, TransitionOptions, Vec3,
};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.objects()?.std()?;
    let model = Gnoblin::get(&g.objects()?)?;
    let flat = Rc::new(Camera2D::new(&std)?.as_scene_node()?);
    let deep = Camera3D::new_with(
        &std,
        Camera3DOptions {
            projection: Some(Projection::Perspective),
            fov: Some(65.0),
            position: Some(Vec3 { x: 0.0, y: 0.0, z: 1600.0 }),
            ..Camera3DOptions::default()
        },
    )?;
    let deep = Rc::new(deep.as_scene_node()?);

    let input = model.input()?;
    let seat = input.primary()?;
    let seat = Rc::new(seat);
    let context = Rc::clone(&seat);
    let mut three_d = false;
    seat.bind(
        "Super+Tab",
        Some(move |_| {
            three_d = !three_d;
            let camera = Rc::clone(if three_d { &deep } else { &flat });
            let context = Rc::clone(&context);
            model.request_transition(
                move || {
                    if let Some(display) = context.active_display()? {
                        if let Some(viewport) = display.viewport()? {
                            viewport.set_camera(&camera)?;
                        }
                    }
                    Ok(())
                },
                Some(&TransitionOptions {
                    duration: 750.0,
                    ..TransitionOptions::default()
                }),
                Some(|| Ok(())),
            )?;
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

`Camera2D` has `position` (`{ x, y, z }`), `zoom` and `roll`; `Camera3D` has `position`, `projection` (`orthographic`,
`perspective` or `stereo`), `fov`, `size` (orthographic), `near` and `far`.

**What is drawn.** Windows sit directly under the tree root and keep their own positions, so the shell has no
per-display root to transform. A camera on a viewport is kept and reads back, and nothing more is applied:

- A `Camera2D` that is not the identity (`zoom` 1, `position` 0, `roll` 0) is not applied yet: it is reported once
  per reload as not applied.
- A `Camera3D` is not applied yet: it is reported once per reload as `not applied: camera3d (needs a projected
workspace)`. A flat-workspace perspective projection is renderer work.
- `viewport.clearColor` is not applied yet: it is kept, and the compositor has no background colour behind a workspace
  to apply it to.

A layout can place windows in depth with `z`, `rotation` and `scale`; the camera does not add to that.

## Listen for plugging and unplugging

::: tabs

```typescript
import { Gnoblin, OutputCollectionListener } from "gnoblin-core";
import type { Output } from "gnoblin-core";

const gnoblin = Gnoblin.get();

class Hotplug extends OutputCollectionListener {
    onConnected(output: Output) {
        gnoblin.spawn(["notify-send", `${output.name} plugged in`]);
    }
    onDisconnected(output: Output) {
        gnoblin.spawn(["notify-send", `${output.name} unplugged`]);
    }
}

gnoblin.outputs.addListener(new Hotplug());
```

```python
from gnoblin.objects.model import Gnoblin, Output, OutputCollectionListener

gnoblin = Gnoblin.get()


class Hotplug(OutputCollectionListener):
    def on_connected(self, output: Output) -> None:
        gnoblin.spawn(["notify-send", f"{output.name} plugged in"])

    def on_disconnected(self, output: Output) -> None:
        gnoblin.spawn(["notify-send", f"{output.name} unplugged"])


gnoblin.outputs.add_listener(Hotplug())
```

```rust
use std::rc::Rc;

use gnoblin_core::{Gnoblin, Output, OutputCollectionListenerHandlers};
use gnoblin_core::{self as gnoblin};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let objects = g.objects()?;
    let model = Rc::new(Gnoblin::get(&objects)?);
    let outputs = model.outputs()?;
    let (plugged, unplugged) = (Rc::clone(&model), Rc::clone(&model));
    let listener = OutputCollectionListenerHandlers {
        on_connected: Some(Box::new(move |output: Output| {
            let text = format!("{} plugged in", output.name()?);
            plugged.spawn(&["notify-send".to_owned(), text], None)?;
            Ok(())
        })),
        on_disconnected: Some(Box::new(move |output: Output| {
            let text = format!("{} unplugged", output.name()?);
            unplugged.spawn(&["notify-send".to_owned(), text], None)?;
            Ok(())
        })),
        ..Default::default()
    }
    .implement(&objects)?;
    outputs.add_listener(&listener)?;
    Ok(())
}
```

:::

[Events](/events/desktop-state#listen-to-outputs) lists the methods an `OutputCollectionListener` can define.

Each language can listen for one event without a class:

- TypeScript: `gnoblin.outputs.on("connected", handler)` and `gnoblin.outputs.on("disconnected", handler)` return a `Listener` whose `stop()` stops listening.
- Python: `gnoblin.outputs.on("connected", handler)`.
- Rust: `outputs.on_connected(run)?` and `outputs.on_disconnected(run)?` listen until the `Listener` they return is dropped.

A listener on the collection hears a connection only for
an output plugged in while the shell runs. A disconnection is reported after the output is gone, and its last values stay
readable. Membership changes reach a collection listener as additions and removals, as they do for windows.

## One workspace on one output

A `Workspace` is on one output at a time. Each output starts on the first workspace no output shows. `output.show(ws)` puts
`ws` on that output. If another output shows it, that output takes the workspace this one showed, so no workspace is on two
outputs.

| Call                                       | Acts on                                                                         |
| ------------------------------------------ | ------------------------------------------------------------------------------- |
| `output.show(ws, options?)`                | That output. `options.duration` slides the windows over that many milliseconds. |
| `gnoblin.workspaces.show(ws)`, `ws.show()` | The output under the pointer, or the primary output when no pointer says.       |
| `Super+1` to `Super+9` (stock keys)        | The same as `gnoblin.workspaces.show`.                                          |

- `ws.active` is `true` while an output shows the workspace, so with two outputs two workspaces are active.
- `output.workspace` is the workspace an output shows, and `gnoblin.workspaces.ofOutput(n)` gives the workspace the output
  numbered `n` shows.
- A new window joins the workspace the output under the pointer shows.
- `ShowOptions` takes `output` too, as the output's number: `gnoblin.workspaces.show(ws, { output: Number(output.id) })`.

Each display chooses its own workspace: set `display.viewport.content = workspace` for the display you mean (see
[Displays and viewports](#displays-and-viewports)).

## Areas and bars

A layout computes in the area of the output that shows its workspace, in the compositor's coordinates: a workspace on an
output at `x = 1920` lays windows out at `x >= 1920`. `gnoblin.workspaces.area` is the primary output's area.

A `Bar` sits on the primary output unless you give it one:

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core";
import { Bar } from "gnoblin-std";

const second = Gnoblin.get().outputs.at(1);
if (second) {
    new Bar({ edge: "bottom", height: 24, output: Number(second.id) });
}
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import Bar

second = Gnoblin.get().outputs.at(1)
if second:
    Bar(edge="bottom", height=24, output=int(second.id))
```

```rust
use gnoblin_core::{self as gnoblin, Gnoblin};
use gnoblin_std::GnoblinStdExt;
use gnoblin_std::ObjectsStdExt;
use gnoblin_std::{Bar, BarEdge, BarOptions};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.objects()?.std()?;
    let model = Gnoblin::get(&g.objects()?)?;
    let outputs = model.outputs()?;
    if let Some(second) = outputs.at(1)? {
        let options = BarOptions {
            edge: Some(BarEdge::Bottom),
            height: Some(24.0),
            output: second.id()?.parse::<u64>().ok(),
            ..BarOptions::default()
        };
        Bar::new_with(&std, options)?;
    }
    Ok(())
}
```

:::

A bar takes room only from windows on its own output.

A bar hides while a fullscreen window shows on the workspace its output
shows, and the other outputs' bars stay. A bar whose output is unplugged is hidden and takes no room. Maximise and fullscreen
use the output the window is on, and a drag stays inside that output's area.

## Unplugging

When an output goes:

- **Another output remains.** The windows of the workspace it showed move to the workspace the remaining output shows, fitted
  to that output: a maximised window fills it, a snapped one tiles again, a floating one is shrunk to fit and moved inside
  it. The workspace the unplugged output showed is on no output until you show it.
- **No output remains.** The windows wait hidden, their workspace is not active, and the keyboard focus is dropped, because a
  window with no output cannot hold it. The next output to appear shows that workspace again, the windows come back, and the
  first of them takes the focus when nothing else holds it.
    - The windows stay in the tree while they wait, however long the wait is.
    - Each comes back as the same window: on the workspace it was on, snapped or maximised as it was.

A script that runs with no output at all still has a workspace on show; it just lays nothing out. A reload hands the next
configuration what each output showed, by output name, and each output goes back to that workspace when it still exists.

## Move a window between outputs

::: tabs

```typescript
import { ClientQuery, Gnoblin } from "gnoblin-core";
import { ClientAugment, OutputAugment } from "gnoblin-std";

const gnoblin = Gnoblin.get();
const [client] = gnoblin.findClients(new ClientQuery({ focused: true }));
const second = gnoblin.outputs.at(1);
const target = second ? OutputAugment.get().workspace(second) : undefined;
if (client && target) {
    ClientAugment.get().moveToWorkspace(client, target); // true when it moved
}
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import ClientAugment, OutputAugment

gnoblin = Gnoblin.get()
clients = gnoblin.find("Window", {"focused": True})
second = gnoblin.outputs.at(1)
target = OutputAugment.get().workspace(second) if second else None
if clients and target:
    ClientAugment.get().move_to_workspace(clients[0], target)  # True when it moved
```

```rust
use gnoblin_core::{ClientQuery, Gnoblin};
use gnoblin_core::{self as gnoblin};
use gnoblin_std::{ClientStdExt, OutputStdExt};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let focused = ClientQuery {
        focused: Some(true),
        ..ClientQuery::default()
    };
    let client = model.find_clients(Some(&focused))?.into_iter().next();
    let outputs = model.outputs()?;
    let target = match outputs.at(1)? {
        Some(second) => second.workspace()?,
        None => None,
    };
    if let (Some(client), Some(target)) = (client, target) {
        client.move_to_workspace(&target)?; // true when it moved
    }
    Ok(())
}
```

:::

`client.moveToOutput(output)` moves a window to the workspace an output shows, and `client.moveToWorkspace(ws)` does the same for a workspace. Each fits the window to the work area it
lands in and returns `false`, moving nothing, for:

- a fullscreen window;
- a dialog, which moves only with its parent;
- a dead window, an output that is gone or shows nothing, or a workspace that is not in `gnoblin.workspaces`;
- the workspace the window is on already.

Python calls these on `ClientAugment.get()`, as `move_to_output(client, output)` and `move_to_workspace(client, workspace)`.
Rust has `client.move_to_output(&output)?` and `client.move_to_workspace(&workspace)?` from the `ClientStdExt` trait.

A window that held the focus and leaves the shown workspace hands the focus to a window that stays, or to nobody. A snapped
window's remembered floating rectangle is not refitted for the new output.

## Dialogs

A window that its client made a dialog of another window (`xdg_toplevel.set_parent` or xdg-foreign) has a `parentWindow`.
The stock shell:

- puts the dialog on its parent's workspace and centres it in the parent when it opens;
- moves it by the same distance whenever the parent moves, whoever moved the parent;
- takes it along when the parent changes workspace or output, and never offers it to a tiling layout;
- places it as an ordinary window when its parent is gone.

## Windows from a launcher

`client.fromLauncher` is `true` when the window opened on an activation token that a privileged or trusted Wayland
connection, the session's launcher, asked for. The compositor records it when the window redeems such a token. The
`focus.newWindows` setting then decides whether a launched window takes the focus: only `allow` lets it.

A program started with `gnoblin.spawn` is not marked, because the shell's spawner does not mint a token and the compositor
has nothing to attribute the window to. A launcher that runs as a Wayland client and mints a token for the program it starts
does mark it.
