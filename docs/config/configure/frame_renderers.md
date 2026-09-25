# gnoblin.configure.frame_renderers

Configure this part of `gnoblin.configure` with the `frame_renderers` key.

Each entry pairs a renderer name with an `argv` array. Names may contain 1–64
letters, digits, `_` or `-`; `native` is reserved for the built-in renderer.
The array has 1–32 strings. Here is what each position means:

| Lua array item | Example                          | Meaning                                                                                                                                             |
| -------------- | -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| First item     | `"gnoblin-frame-cairo"`          | Executable selector. Gnoblin searches the compositor's `PATH`; a full executable path is also accepted. Relative paths containing `/` are rejected. |
| Second item    | `"--theme-file=frame-theme.txt"` | Optional theme-file argument supported by both bundled renderers. Keep the option and file name together in this one string.                        |
| Later items    | Renderer-specific                | One string per additional argument, only when that renderer supports it.                                                                            |

Gnoblin does not parse or expand arguments: quotes, `$HOME`, pipes and
wildcards are passed literally. Both bundled renderers, `gnoblin-frame-cairo`
and `gnoblin-frame-qt`, accept this optional argument:

| Argument            | Value                                                                    | Effect                                                                                                         |
| ------------------- | ------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------- |
| `--theme-file=FILE` | A file whose first line is a six-digit hex colour, for example `#242424` | Sets the renderer's background colour. Relative file names are resolved from the renderer's working directory. |

`--theme-file=FILE` is the frame-specific option handled by the bundled
adapters. Check a custom renderer's own documentation for its options; Gnoblin
passes those argument strings through unchanged.

Check that your shell can find a renderer with
`command -v gnoblin-frame-cairo`. Gnoblin must be started with an environment
whose `PATH` can find the same executable.

Select the registered name in a `frame` field on
[`gnoblin.window_rule`](/config/window_rule#frame-fields).

```lua
gnoblin.configure {
    frame_renderers = {
        cairo = {"gnoblin-frame-cairo"},
    },
}

gnoblin.window_rule {
    match = {type = "window"},
    frame = {mode = "auto", renderer = "cairo"},
}
```

For example, add a theme-file argument as a second array item:

```lua
gnoblin.configure {
    frame_renderers = {
        cairo = {"gnoblin-frame-cairo", "--theme-file=frame-theme.txt"},
    },
}
```

Gnoblin resolves the executable name through `PATH` and starts it with this
argument vector:

| Process argument | Value                                      |
| ---------------- | ------------------------------------------ |
| `argv[0]`        | Executable found for `gnoblin-frame-cairo` |
| `argv[1]`        | `--theme-file=frame-theme.txt`             |
| `argv[2…]`       | No further arguments                       |

The theme file must contain a line such as `#242424`. The Cairo and Qt
renderers use that colour for the frame background.
