# gnoblin.configure.accessibility

Configure this part of `gnoblin.configure` with the `accessibility` key.

Turn on the magnifier and choose a colour filter. Both apply to every output,
and both are off by default:

```lua
gnoblin.configure {
    accessibility = {
        magnifier = true,
        ["magnifier-zoom"] = 2,
        ["magnifier-mode"] = "lens",
        ["colour-filter"] = "none",
    },
}
```

| Setting          | Accepted value                                                                   | Default |
| ---------------- | -------------------------------------------------------------------------------- | ------- |
| `magnifier`      | A boolean                                                                        | `false` |
| `magnifier-zoom` | A number from 1 to 32                                                            | `2`     |
| `magnifier-mode` | One of `lens`, `full-screen`                                                     | `lens`  |
| `colour-filter`  | One of `none`, `invert`, `greyscale`, `protanopia`, `deuteranopia`, `tritanopia` | `none`  |

`magnifier = true` shows each output magnified by `magnifier-zoom`. A zoom of
`1` leaves the output unchanged.

`magnifier-mode` is read and validated, but both values show the whole output
at the chosen zoom, centred on that output. The `lens` value does not draw a
lens at the pointer or focus.

`colour-filter` changes every colour on every output:

- `none` leaves colours unchanged.
- `invert` inverts each colour channel. Alpha is kept.
- `greyscale` replaces each colour with its luma, using Rec. 709 weights.
- `protanopia`, `deuteranopia` and `tritanopia` simulate a viewer who has no
  working red, green or blue cones. They show how the screen looks to that
  viewer. They do not correct the colours.
