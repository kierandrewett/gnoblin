# Switch monitors with a stylus button

Assign the secondary button on a stylus to switch the pointer between
monitors. Replace the example vendor and product IDs with your device's IDs:

```lua
gnoblin.configure {
    input = {
        styluses = {
            ["default-1234:5678"] = {
                secondary_button_action = "switch-monitor",
            },
        },
    },
}
```

The `default-` prefix applies the mapping to matching devices that do not have
an individual serial-number override. Other stylus buttons keep their current
actions. See the [input reference](/config/configure/input#styluses).
