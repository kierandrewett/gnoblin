# Disable compositor animations

Turn off compositor-managed animations while leaving shell-specific animations
to the shell:

```lua
gnoblin.configure {
    compositor = {enable_animations = false},
}
```

This is broader than disabling animation for one layer surface. To change only
bars or popups, use the [layer animation recipe](/recipes/turn-off-layer-animations)
instead. See the [compositor reference](/config/configure/compositor).
