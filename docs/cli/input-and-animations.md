# Input and Animation Commands

Inspect input devices and sources, select a source, and preview registered animations.

Commands require a running session. Animation previews act on a live compatible target.

## Layer surfaces

List current layer-shell surfaces and their namespaces with:

```sh
gnoblinctl layer list
```

The command reads the API 1.37 `layers.list` snapshot and keeps the `layers`
object in JSON output. Records include a state revision.

Use a surface's `namespace` as the `layer` value in a window rule. The
`animation surfaces` command reports the same surfaces for animation previews.

## Input devices

List input devices detected by the compositor with:

```sh
gnoblinctl input devices
```

Each record reports the device name, type, seat, and available capabilities.
Vendor and product IDs appear when the compositor provides them.

Device IDs last only for the current compositor session. The command returns a
one-time snapshot through the shared Lua read introduced in API 1.46. This read
requires the Lua supervisor for every supported socket client version.

API 1.4 socket clients can also subscribe to device add and removal events.
`gnoblinctl input list` reads the API 1.6 `input.sources` snapshot through the
shared Lua read introduced in API 1.46. It prints the configured XKB sources.

`gnoblinctl input current` reads `input.current_source` through the same Lua
route. Its JSON record includes `available` and, when known, `source`.

`gnoblinctl input select TYPE ID` calls `input.select` and waits for Mutter
to confirm the layout change. Copy `TYPE` and `ID` exactly from
`gnoblinctl input list`; they identify a configured XKB source.

`gnoblinctl input orientation-lock` reads the current orientation-lock
record through `input.orientation_lock`. Pass `on` or `off` to set the lock;
pass `inherit` to clear the runtime override and restore the configured value
or follow the system setting. This command does not write the config file.

```sh
gnoblinctl input orientation-lock
gnoblinctl input orientation-lock on
gnoblinctl input orientation-lock inherit
```

The Lua-only aliases `gnoblin.input.list()` and `gnoblin.input.current()` are
not socket methods. Socket clients use the names above.

## Animations

Use animation previews to inspect registered curves for compatible window and
layer-shell targets. A preview starts paused and changes only the target's
visual transform; it does not minimize or close the target.

```sh
gnoblinctl animation list
gnoblinctl animation get gnome-open
gnoblinctl animation surfaces
gnoblinctl animation inspect gnome-open --window active
session=$(gnoblinctl animation preview gnome-open --window active --format table | sed -n 's/^session: //p')
gnoblinctl animation seek "$session" 50
gnoblinctl animation step "$session" 16
gnoblinctl animation play "$session"
gnoblinctl animation pause "$session"
gnoblinctl animation stop "$session"
```

Omitting `--window` uses the active window. For a layer-shell target, pass
`--layer ID` or `--namespace NAME`. The namespace must match exactly one visible
surface.

`seek` accepts an integer percentage from 0 to 100. `step` accepts elapsed time
in milliseconds. `play` and `pause` change playback; `stop` restores the target’s
original visual state.

Seeking to 100% keeps the last frame visible until `stop`.

`inspect --event EVENT` selects an event variant. `preview --autoplay` starts
playback immediately.

`animation surfaces` prints layer surface IDs, namespaces, and titles. Layer
surfaces are not included in `window list`.

The Lua console exposes the same snapshot through
`gnoblin.animations.surfaces()`. Its `surfaces` array and nested records are
read-only:

```lua
for _, surface in ipairs(gnoblin.animations.surfaces().surfaces) do
    print(surface.id, surface.namespace, surface.title)
end
```

Inspect the resolved animation for a visible target with
`gnoblin.animations.inspect {name, target, event?, target_type?}`. The result
and its nested tables are read-only:

```lua
local inspection = gnoblin.animations.inspect {
    name = "gnome-open",
    target = "active",
}
print(inspection.event, inspection.duration)
```

`animation list` marks entries that current preview targets can run with
`previewable`.

`animation get NAME` prints the configured record for an exact name and returns
`null` when no animation matches. Use `--json` in scripts.

`inspect --event EVENT` accepts the animation events supported by the config,
such as `open`, `close`, `focus`, `minimize`, and `workspace-switch`. The
[animation guide](/guides/animations) lists the events and their target types.

Workspace and shadow animations run on internal compositor actors or effects,
so the current CLI cannot preview them against a window or layer surface.

See the [animation guide](/guides/animations) for custom curves, events and
GNOME-style presets.
