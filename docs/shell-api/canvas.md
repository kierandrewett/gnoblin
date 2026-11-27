# Canvas painting

A `Canvas` is a node that shows what a shell paints on it: rectangles, text and icons. The stock widgets are canvases, so the
compositor draws the stock titlebar, panel and lock screen from the paint lists the shell writes.

This page describes the paint list and how it is drawn. The widgets built on it are in the [UI toolkit](/js-ui).

::: tabs

```typescript
import { Canvas, Widget } from "gnoblin-std/generated/objects";
import type { PaintOp } from "gnoblin-std/generated/objects";

class Label extends Widget {
    paint(): PaintOp[] {
        return [
            { op: "rect", x: 0, y: 0, width: 120, height: 32, radius: 6, fill: "#1e1e2e" },
            { op: "text", text: "Hello", x: 10, y: 4, size: 16, color: "#cdd6f4" },
        ];
    }
}

const label = new Canvas({ x: 20, y: 20, width: 120, height: 32, behaviour: new Label() });
label.mountTop();
```

```python
from gnoblin.objects.std import Canvas, PaintKind, PaintOp, Widget


class Label(Widget):
    def paint(self):
        return [
            PaintOp(op=PaintKind.RECT, x=0.0, y=0.0, width=120.0, height=32.0, radius=6.0, fill="#1e1e2e"),
            PaintOp(op=PaintKind.TEXT, text="Hello", x=10.0, y=4.0, size=16.0, color="#cdd6f4"),
        ]


label = Canvas(x=20.0, y=20.0, width=120.0, height=32.0, behaviour=Label())
label.mount_top()
```

```rust
use gnoblin_core::{self as gnoblin, Gnoblin};
use gnoblin_std::objects::{Canvas, CanvasOptions, PaintKind, PaintOp, WidgetHandlers};
use gnoblin_std::GnoblinStdExt;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;

    let widget = WidgetHandlers {
        paint: Some(Box::new(move || {
            Ok(vec![
                PaintOp {
                    x: Some(0.0),
                    y: Some(0.0),
                    width: Some(120.0),
                    height: Some(32.0),
                    radius: Some(6.0),
                    fill: Some("#1e1e2e".into()),
                    ..PaintOp::new(PaintKind::Rect)
                },
                PaintOp {
                    x: Some(10.0),
                    y: Some(4.0),
                    size: Some(16.0),
                    color: Some("#cdd6f4".into()),
                    text: Some("Hello".into()),
                    ..PaintOp::new(PaintKind::Text)
                },
            ])
        })),
        ..Default::default()
    }
    .implement(&objects)?;

    let label = Canvas::new_with(
        &objects,
        CanvasOptions {
            children: None,
            x: Some(20.0),
            y: Some(20.0),
            width: Some(120.0),
            height: Some(32.0),
            visible: None,
            opacity: None,
            secure: None,
            input: None,
            placement: None,
            behaviour: Some(widget),
            on_pointer: None,
            on_key: None,
        },
    )?;
    label.mount_top()?;

    Ok(())
}
```

:::

Put the code in your shell: `shell.ts` or `shell.py` for a script, or the `lib.rs` of a Rust shell library. A rounded label that says Hello appears over every window.

## The paint list and the content

The `paint` method of a widget (`paint()` in TypeScript and Python, the `paint` handler in Rust) returns a list of drawing ops. The runtime encodes the list into the node's `content`, a byte string the
compositor decodes, and writes it only when it changed. A script does not write `content` by hand.

The compositor paints the list into a picture of the node's size when `content` or the size changes, not every frame, and
draws that picture where the node sits, with the node's opacity and transform.

The picture is painted at the scale of the output the canvas is on, so text and strokes are sharp on a HiDPI screen. A canvas
200 by 28 pixels on an output at scale 2 is a 400 by 56 pixel picture drawn over the same 200 by 28 rectangle.

- A canvas on two outputs is painted at the larger scale. It is painted again when it moves to an output of another scale.
- A node with no area, a `width` or `height` of 0, draws nothing.
- A picture over 4096 by 4096 pixels is not drawn. A canvas whose picture at the output's scale would pass that limit is
  painted at scale 1.

A canvas whose `secure` is `true`, or one under a secure node, is drawn on the screen as usual and masked in a screen capture,
as a secure window is.

If an op cannot be encoded, the runtime reports `draw op N: ...` and leaves the content as it was. If the compositor cannot
decode a list, it refuses the write with `bad-paint` and the canvas keeps its last content.

## Placement

A canvas that is a child of a window has a `placement`, which says where it sits in the window's frame:

- `frame` is the default. The canvas is in front of the window's content and part of the frame's shape, so the rounded
  corners, the border ring and the shadow follow it. A titlebar is one.
- `over` is in front of the content and not part of the frame's shape.
- `under` is painted behind the content and not part of the frame's shape.

A badge or a focus ring that reaches past the frame is `over`, and a glow is `under`, so neither widens the rounded frame.
Any other value is refused with `bad-value`, and the canvas keeps its placement.

A canvas with `input` set to `false` lets the pointer through to what is under it. A child of a window that lies outside the
window's rectangle still takes the pointer. The [window frame](/js-window-frame#layers-and-the-canvas-placement) writes
`over` and `under` for its two layers.

## Ops

Coordinates are pixels from the node's top left. Ops draw in order, later over earlier. Each op is an object with an `op`
field that names it.

| `op`           | Fields                                                                 | Draws                                                     |
| -------------- | ---------------------------------------------------------------------- | --------------------------------------------------------- |
| `rect`         | `x`, `y`, `width`, `height`; `radius`, `fill`, `stroke`, `strokeWidth` | A rectangle with optionally rounded corners.              |
| `line`         | `x1`, `y1`, `x2`, `y2`; `width`, `color`                               | A straight line.                                          |
| `text`         | `text`; `x`, `y`, `size`, `color`, `font`                              | Text.                                                     |
| `icon`         | `name`; `x`, `y`, `size`, `color`                                      | A built-in icon in a square of `size`.                    |
| `push-clip`    | `x`, `y`, `width`, `height`; `radius`                                  | Starts limiting the ops that follow to a rectangle.       |
| `pop-clip`     | none                                                                   | Ends the innermost clip. Clips nest.                      |
| `push-opacity` | `alpha`                                                                | Starts drawing the ops that follow as a group at `alpha`. |
| `pop-opacity`  | none                                                                   | Ends the group.                                           |

The `op` column gives the value of `op` in TypeScript. The same kinds in the other languages:

- Python: `PaintKind.RECT`, `PaintKind.PUSH_CLIP`, and so on.
- Rust: `PaintKind::Rect`, `PaintKind::PushClip`, and so on.

Field names are camelCase in TypeScript (`strokeWidth`) and snake_case in Python and Rust (`stroke_width`).

Defaults of the optional fields:

| Field            | Default                     | Notes                                                                      |
| ---------------- | --------------------------- | -------------------------------------------------------------------------- |
| `radius`         | 0                           | A radius over half the shorter side is clamped.                            |
| `fill`, `stroke` | none                        | A rectangle that omits one has none. The outline is drawn inside the edge. |
| `strokeWidth`    | 0                           |                                                                            |
| `width` (line)   | 1                           |                                                                            |
| `color`          | white                       | An op that omits its colour draws white.                                   |
| `x`, `y`         | 0                           |                                                                            |
| `size`           | 14 for text, 16 for an icon |                                                                            |
| `font`           | `sans`                      | See [Fonts](#fonts).                                                       |

A text op's `(x, y)` is the top left of the line box, which is `floor(size * 1.5)` pixels tall. A `\n` starts another line.
A colour is `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.

A list has at most 4096 ops, groups nest at most 32 deep, a text op carries at most 16 KiB of text, and the encoded list is at
most 1 MiB. A list over the op or depth limit, or one that is not a paint list at all, is refused whole with `bad-paint`. One
over 1 MiB is refused `too-large`.

Everything is clipped to the canvas: text far wider or taller than the node, at any size, draws only the part on it, and the
glyphs and lines wholly past its edges are skipped rather than drawn.

## Fonts

The `font` of a text op is a family name such as `"Noto Sans"`, matched case-insensitively against the installed fonts. It may
also be a generic name: `sans`, `serif` or `monospace` take the first installed family from a list of well-known ones. A name
that matches nothing draws with the default sans.

The compositor finds fonts in these directories:

- `/usr/share/fonts` and `/usr/local/share/fonts`
- `~/.local/share/fonts` and `~/.fonts`
- the `fonts` directory of each `XDG_DATA_DIRS` entry

With no font installed it draws with an embedded subset of Droid Sans: Basic Latin, Latin-1 and common punctuation.

Text is laid out by advance widths alone: there is no kerning, shaping, bidirectional reordering or hinting. A glyph the
chosen font lacks is taken from the embedded face, and drawn as that font's missing-glyph box when neither has it.

## Icons

These icons are built in, each with or without the `-symbolic` suffix:

- `window-close`, `window-minimize`, `window-maximize` and `window-restore`
- `list-add`, `list-remove` and `open-menu`

They are drawn as vector strokes, so the stock titlebar does not depend on an icon theme. An icon that is not built in draws
nothing.

## Measuring text

The `metrics` property of a `Text` widget is `{ width, height, ascent }` for its string, size and font, in logical pixels: `text.metrics` in TypeScript and Python, and `metrics` in the Rust text widget's accessors. It is read-only, and the library's painter measures the string when you read it.

- `height` is a line box per line.
- `ascent` is the distance from the top of the first line to its baseline, where the bottom of a capital letter sits. Align
  two texts of different sizes, or a text and an icon, by it.

A `Text` made with `ellipsize` on (`ellipsize: true` in TypeScript and Python, `ellipsize` set in Rust) that a row gives less room than it measured draws the share of the text that fits and an
ellipsis. The stock titlebar's title does, so a long window title never runs under the close button.
