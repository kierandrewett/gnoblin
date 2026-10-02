# Turn off layer animations

Disable the compositor animation for all layer surfaces, such as bars and
popups:

```lua
gnoblin.window_rule {
    match = {type = "layer"},
    animation = "none",
}
```

`type = "layer"` matches layer-shell surfaces, and the rule's `animation`
field accepts a registered animation name or `"none"`.

The matching rule overrides animation choices in earlier component rules. This
controls layer surfaces appearing or disappearing; a shell may animate its own
contents separately. Configure those animations in that shell.
See the [animation reference](/config/animation) and
[window rule reference](/config/window_rule#animation-fields) for available values.
