# Window state shortcuts

Gnoblin can restore a maximised or snapped window before minimising it. Configure
these bindings in `~/.config/gnoblin/init.lua`:

```lua
g.config.keybindings = {
    wm = {maximize = {"<Super>Up"}, minimize = {}, unmaximize = {}},
    mutter = {["toggle-tiled-left"] = {"<Super>Left"}, ["toggle-tiled-right"] = {"<Super>Right"}},
}
g.config.shortcuts = {
    {name = "restore-or-minimize", binding = "<Super>Down",
     command = {"gnoblinctl", "window", "restore-or-minimize", "active"}},
}
```

When replacing an existing Super+Down binding, save the empty `minimize` and
`unmaximize` arrays first. Then add the custom shortcut. This releases the old
binding before Gnoblin registers the command.

`restore-or-minimize` restores native maximisation or tiling first. For a custom
snap, it restores the saved frame. Otherwise it minimises the window.

Bingux marks its top-edge region with `maximize = true`. Gnoblin applies native
maximisation for that region, so the title bar can restore the window when dragged
away. The top-edge preview covers the work area. Other regions retain their
configured inner and outer gaps.

To keep square maximised windows with a border, set `corners.keep-maximized =
false` and `borders.keep-maximized = true` in the relevant window rules. Maximised
borders use square corners. A border side that touches the physical monitor edge
is omitted only while maximised. Edges beside a topbar or dock remain visible.
Floating windows retain their configured radius and all four border sides.
