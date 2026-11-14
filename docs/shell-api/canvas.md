# Canvas painting

A `Canvas` is a node that shows what a shell paints on it: rectangles, text and icons. The stock widgets are canvases, so the
compositor draws the stock titlebar, panel and lock screen from the paint lists the shell writes.

This page describes the paint list and how it is drawn. The widgets built on it are in the [UI toolkit](/js-ui).

```ts
import { Canvas } from "gnoblin";
import type { PaintOp } from "gnoblin";

class Label extends Canvas {
    draw(): PaintOp[] {
        return [
            { op: "rect", x: 0, y: 0, width: 120, height: 32, radius: 6, fill: "#1e1e2e" },
            { op: "text", text: "Hello", x: 10, y: 4, size: 16, color: "#cdd6f4" },
        ];
    }
}

const label = new Label({ x: 20, y: 20, width: 120, height: 32 });
label.mountTop();
```

Put the code in `~/.config/gnoblin/init.ts`. A rounded label that says Hello appears over every window.

## `draw` and the content

`draw()` returns an array of drawing ops. The runtime encodes the array into the node's `content`, a byte string the
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

## Ops

Coordinates are pixels from the node's top left. Ops draw in order, later over earlier. Each op is an object with an `op`
field that names it.

| `op`          | Fields                                                                 | Draws                                                     |
| ------------- | ---------------------------------------------------------------------- | --------------------------------------------------------- |
| `rect`        | `x`, `y`, `width`, `height`; `radius`, `fill`, `stroke`, `strokeWidth` | A rectangle with optionally rounded corners.              |
| `line`        | `x1`, `y1`, `x2`, `y2`; `width`, `color`                               | A straight line.                                          |
| `text`        | `text`; `x`, `y`, `size`, `color`, `font`                              | Text.                                                     |
| `icon`        | `name`; `x`, `y`, `size`, `color`                                      | A built-in icon in a square of `size`.                    |
| `pushClip`    | `x`, `y`, `width`, `height`; `radius`                                  | Starts limiting the ops that follow to a rectangle.       |
| `popClip`     | none                                                                   | Ends the innermost clip. Clips nest.                      |
| `pushOpacity` | `alpha`                                                                | Starts drawing the ops that follow as a group at `alpha`. |
| `popOpacity`  | none                                                                   | Ends the group.                                           |

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

`text.metrics()` of a `Text` widget returns `{ width, height, ascent }` for its string, size and font, in whole pixels. The
compositor measures, so the answer arrives a frame later. Until then, and when the compositor refuses the question, the
numbers are estimates: half the size per character of the widest line, one and a half sizes per line, and 1.1 sizes of ascent.

- `height` is a line box per line.
- `ascent` is the distance from the top of the first line to its baseline, where the bottom of a capital letter sits. Align
  two texts of different sizes, or a text and an icon, by it.

The answer is kept with the text, size and font it was for. Changing any of them falls back to the estimates and asks again.

A `Text` made with `ellipsize: true` that a row gives less room than it measured draws the share of the text that fits and an
ellipsis. The stock titlebar's title does, so a long window title never runs under the close button.
