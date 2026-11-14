# Accessibility settings

The `accessibility` key turns on the magnifier and chooses a colour filter. Both apply to every output, and both are
off by default:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    accessibility: {
        magnifier: true,
        magnifierZoom: 2,
        colourFilter: "none",
    },
});
```

| Setting         | Accepted value                                                                   | Default |
| --------------- | -------------------------------------------------------------------------------- | ------- |
| `magnifier`     | A boolean                                                                        | `false` |
| `magnifierZoom` | A number from 1 to 16; a larger value is used as 16                              | `2`     |
| `colourFilter`  | One of `none`, `invert`, `greyscale`, `protanopia`, `deuteranopia`, `tritanopia` | `none`  |

`magnifier: true` shows each output magnified by `magnifierZoom`, centred on the pointer while it is on that output.
The view is kept inside the output, so it stops at the edges. A zoom of `1` leaves the output unchanged.

A screenshot captures the last frame drawn for the output, so it includes the magnifier and the colour filter.

`colourFilter` changes every colour on every output:

- `none` leaves colours unchanged.
- `invert` inverts each colour channel. Alpha is kept.
- `greyscale` replaces each colour with its luma, using Rec. 709 weights.
- `protanopia`, `deuteranopia` and `tritanopia` simulate a viewer who has no working red, green or blue cones. They
  show how the screen looks to that viewer. They do not correct the colours.
