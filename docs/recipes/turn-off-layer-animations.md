# Turn off layer animations

Disable the compositor animation for all layer surfaces, such as bars and
popups:

```lua
gnoblin.configure {shell = {layer_animation = "none"}}
gnoblin.window_rule {
    match = {type = "layer"},
    animation = "none",
}
```

The matching rule overrides animation choices in earlier component rules. This
controls layer surfaces appearing or disappearing; a shell may animate its own
contents separately. Configure those animations in that shell.
