# gnoblin.configure.input.styluses

Set per-pen button overrides under a device serial or default device ID in
`gnoblin.configure.input.styluses`:

```lua
gnoblin.configure {
    input = {
        styluses = {
            ["default-1234:5678"] = {
                button_action = "default",
                secondary_button_action = "middle",
                tertiary_button_action = "right",
                button_keybinding = "",
                secondary_button_keybinding = "",
                tertiary_button_keybinding = "",
            },
        },
    },
}
```

Use a pen's hexadecimal serial number or `default-` followed by a four-digit
vendor and product ID. Each button action accepts `"default"`, `"middle"`,
`"right"`, `"back"`, `"forward"`, `"switch-monitor"`, or `"keybinding"`.
The matching `*_button_keybinding` field supplies the accelerator when its
action is `"keybinding"`.
