# gnoblin.autostart

This is the legacy declaration form for adding or updating a named command that runs once per login. Prefer [`gnoblin.configure.autostart`](/guides/autostart), which lets you edit named entries directly. An entry with the same `name` is merged; omitted fields stay unchanged. See the [autostart guide](/guides/autostart) for launch timing and examples.

```lua
gnoblin.autostart {
    name = "panel",
    command = {"waybar"},
}
```

| Field     | Values                            |
| --------- | --------------------------------- |
| `name`    | Required nonempty string          |
| `command` | Argument list; no shell expansion |
