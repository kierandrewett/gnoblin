# Monitor settings

Use `gnoblin.configure.monitors` for preferences that apply to connected
monitors.

## Privacy screen

Set `privacy_screen` to request the privacy screen state on monitors that
support it:

```lua
gnoblin.configure {
    monitors = {
        privacy_screen = true,
    },
}
```

`privacy_screen` accepts `true`, `false`, or `"inherit"`. It defaults to
`"inherit"` when omitted. `true` requests that privacy screens turn on;
`false` requests that they turn off. `"inherit"` follows the system privacy
screen preference.

Gnoblin applies an explicit choice for the current session and does not save it
to the system preference. The request applies globally. Monitors without a
privacy screen are unaffected. Remove the setting or use `"inherit"` to follow
the system preference again.
