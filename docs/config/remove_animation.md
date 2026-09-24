# gnoblin.remove_animation

Remove an animation declaration with the given name from the configuration
assembled so far. This is useful when a later config file overrides a shared
configuration. It does not remove built-in animations.

```lua
gnoblin.remove_animation("soft-open")
```

Only declarations already evaluated are removed. A later
[`gnoblin.animation`](/config/animation) call can register the name again. See
the [animation guide](/guides/animations) for declaration order and selection.
