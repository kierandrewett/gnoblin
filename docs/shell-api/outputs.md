# Outputs

An output is a connected display. The compositor publishes one `output` object per display it drives. Its fields are
the ones the `output` object declares in the `gnoblin` schema.

## Read outputs

| Method                        | Arguments             | Result                       |
| ----------------------------- | --------------------- | ---------------------------- |
| `gnoblin.outputs.list()`      | None                  | All current `Output` objects |
| `gnoblin.outputs.primary()`   | None                  | Primary output or `nil`      |
| `gnoblin.outputs.by_id(name)` | String connector name | Matching output or `nil`     |

`gnoblin.find("Output", filter?)` filters the same objects by their fields. See [find](/shell-api/find).

```lua
for _, output in ipairs(gnoblin.outputs.list()) do
    print(output.name, output.width, output.height)
end
```

`gnoblin.outputs.primary()` returns `nil` when no output is marked primary.

## Live output objects

Every read of one output returns the same object, so you can keep it and read it later. The output emits `closed` when the display is unplugged or the compositor stops driving it. `output.alive` then turns `false`, and the object keeps its last state.

## Output records

| Field             | Type            | Meaning                                                                                    |
| ----------------- | --------------- | ------------------------------------------------------------------------------------------ |
| `name`            | String          | Connector name, such as `"DP-1"`. The identity `by_id` takes.                              |
| `make`            | String          | Manufacturer, as EDID reports it. Empty when the output has none.                          |
| `model`           | String          | Model name. Empty when the output has none.                                                |
| `modes`           | List of strings | Every supported mode, each as `<width>x<height>@<refresh>`.                                |
| `mode`            | String          | The mode in use. It is one of `modes`.                                                     |
| `scale`           | Number          | Logical scale, such as 1 or 1.5.                                                           |
| `transform`       | Integer         | Rotation and flip in Mutter's numbering, listed below.                                     |
| `primary`         | Boolean         | Whether this output carries the primary logical monitor.                                   |
| `enabled`         | Boolean         | Whether the output drives a logical monitor. A disabled output is still an object.         |
| `x`, `y`          | Integer         | Logical position of the logical monitor's top-left corner.                                 |
| `width`, `height` | Integer         | Logical size of the logical monitor, which is the mode size divided by the scale.          |
| `layout`          | String          | The layout a workspace on this output runs when it has no override. `floating` when unset. |

`transform` uses Mutter's numbering, and the compositor does not translate it, so a `DisplayConfig` caller sends the same numbers back:

- `0`: normal;
- `1`, `2`, `3`: rotated 90, 180 and 270 degrees;
- `4`: flipped;
- `5`, `6`, `7`: flipped and rotated 90, 180 and 270 degrees.

## Change an output

`output:apply(x, y, scale, transform, primary, enabled, mode)` applies a new configuration to one output. The result
arrives as property changes, so read the object to see what the compositor did.

`output:set_layout(root, module, hash, options)` sets the default layout for workspaces on this output. It takes the
same layout reference as [workspace:set_layout](/shell-api/workspaces).

## Not in the schema yet

These names appear in earlier design notes. The `output` object does not declare them, so they are not available from
Lua:

- `gnoblin.outputs.renderer()` and the `Renderer` record;
- the output fields `id`, `kind`, `serial`, `monitor_id`, `index`, `work_area`, `privacy_screen`, `renderer` and
  `revision`;
- the output signal `changed`, and the `gnoblin.output.added`, `gnoblin.output.changed` and
  `gnoblin.output.removed` events;
- the privacy screen state, and the `monitors.privacy_screen` override that sets it.
