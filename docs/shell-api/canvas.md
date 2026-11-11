# Canvas painting

A `Canvas` is a node that shows what a shell paints on it: rectangles, text and icons.

The stock widgets are canvases, so the compositor draws the stock titlebar, panel and lock screen from the paint lists
the shell writes:

- `Box`, `Text` and `Icon`
- `Bar`, `Entry` and `Titlebar`

```lua
local label = Canvas({
    width = 120,
    height = 32,
    draw = {
        { rect = { 0, 0, 120, 32 }, radius = 6, fill = "#1e1e2e" },
        { text = "Hello", x = 10, y = 4, size = 16, color = "#cdd6f4" },
    },
})
root:add(label)
```

## `draw` and `content`

`draw` is an array of drawing ops, or a function that returns one (the function is a binding: it runs again when a
property it read changes). The runtime encodes the array into the node's `content`, a byte string the compositor
decodes, and writes it when it changes. A script does not write `content` by hand.

The compositor paints the list into a picture the size of the node when `content` or the size changes, not every
frame, and draws that picture where the node sits, with the node's opacity, transform and effects.

The effects are the node's `effects` list, and the renderer may not draw all of them. An effect it skips is named in
the node's read-only `skipped` list (`blur: unsupported-by-renderer`), and `seat.effect_kinds` lists the kinds it does
draw. See [window effects](/guides/window_effects).

If an op cannot be encoded, the runtime reports `draw op N: ...` and leaves the content as it was. If the compositor
cannot decode a list, it refuses the write with `bad-paint` and the canvas keeps its last content.

## Ops

Coordinates are pixels from the node's top left. Ops draw in order, later over earlier.

- `{ rect = { x, y, width, height }, radius = 0, fill = colour, stroke = colour, stroke_width = 0 }`: a rectangle with
  optionally rounded corners. The outline is drawn inside the edge. A radius over half the shorter side is clamped.
- `{ line = { x1, y1, x2, y2 }, width = 1, color = colour }`: a straight line.
- `{ text = "...", x = 0, y = 0, size = 14, color = colour, font = "sans" }`: text. `(x, y)` is the top left of the
  line box, which is `floor(size * 1.5)` pixels tall. A `\n` starts another line.
- `{ icon = "window-close-symbolic", x = 0, y = 0, size = 16, color = colour }`: a built-in icon in a square of `size`.
- `{ clip = { x, y, width, height }, radius = 0 }` and `{ clip = false }`: limit the ops between them to a rectangle.
  Clips nest.
- `{ opacity = 0.5 }` and `{ opacity = false }`: draw the ops between them as a group at that alpha.

A colour is `"#rgb"`, `"#rgba"`, `"#rrggbb"` or `"#rrggbbaa"`. An op that omits its colour draws white; a rectangle
that omits `fill` or `stroke` has none.

A list has at most 4096 ops, groups nest at most 32 deep, a text op carries at most 16 KiB of text, and the encoded
list is at most 1 MiB.

## Fonts

The `font` of a text op is a family name such as `"Noto Sans"`, matched case-insensitively against the installed fonts.
It may also be a generic name: `sans`, `serif` or `monospace` take the first installed family from a list of well-known
ones. A name that matches nothing draws with the default sans.

The compositor finds fonts in these directories:

- `/usr/share/fonts` and `/usr/local/share/fonts`
- `~/.local/share/fonts` and `~/.fonts`
- the `fonts` directory of each `XDG_DATA_DIRS` entry

With no font installed it draws with an embedded subset of Droid Sans (Basic Latin, Latin-1 and common punctuation;
Apache-2.0).

Text is laid out by advance widths alone: there is no kerning, shaping, bidirectional reordering or hinting. A glyph the
chosen font lacks is taken from the embedded face, and drawn as that font's missing-glyph box when neither has it.

## Icons

These icons are built in, each with or without the `-symbolic` suffix:

- `window-close`, `window-minimize`, `window-maximize` and `window-restore`
- `list-add`, `list-remove` and `open-menu`

They are drawn as vector strokes, so the stock titlebar does not depend on an icon theme. An icon that is not built in
draws nothing.

## Measuring text

`canvas:measure(text, size, font)` asks the compositor how much room a string takes. It answers in a later frame, so the
call returns a signal that fires with `"<width> <height>"` in whole pixels, then an error string (empty on success):

```lua
go(function()
    local answer, err = await(label:measure("Hello", 16, "sans"))
    local width, height = answer:match("^(%d+) (%d+)$")
end)
```

The answer is the extent the painter lays the same string out in, so a canvas of that size holds it exactly. The error
is `bad-value` for a size that is not a finite number in `(0, 1024]`, `too-large` for text over 16 KiB or a font name
over 128 bytes, and `unavailable` when there are no fonts.

`Text.width` is this answer. Until it arrives, and when the compositor refuses the question, the width is an estimate of
half the size per character. The answer is kept with the text, size and font it was for, so changing any of them falls
back to the estimate and asks again.
