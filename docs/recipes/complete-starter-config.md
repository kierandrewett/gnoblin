# Complete starter config

Save this as `~/.config/gnoblin/init.ts`, or run `gnoblinctl config init` to write a commented one. Install the
commands you use; replace `foot` with your terminal.

```ts
import { defineConfig, spawn } from "gnoblin";

export default defineConfig({
    windowManagement: { focusMode: "click" },
    keys: { "Super+Return": spawn("foot") },
});
```

| Setting     | Accepted values                           | Default        | Effect                                                              |
| ----------- | ----------------------------------------- | -------------- | ------------------------------------------------------------------- |
| `focusMode` | `"click"`, `"hover"`, or `"hover-strict"` | `"click"`      | Selects when pointer or click input changes focus.                  |
| `keys`      | A table from a trigger to an action       | The stock keys | Binds keys. See [trigger syntax](/guides/shortcuts#trigger-syntax). |

Save the file and the running shell reloads it. See the [defineConfig reference](/typescript-config) and the
[window rule reference](/config/window_rule) for field constraints and precedence.

This sets click-to-focus and adds a terminal shortcut. To dim unfocused windows, see
[dim unfocused windows](/recipes/dim-unfocused-windows).

It does not install or start a desktop shell; choose one in [shell setup](/bring-your-own-shell). If the stock config
already uses Super+Enter, your key replaces it: a trigger is bound once.
