# Add a shortcut

Add a key that starts a program without replacing the rest of your keys:

```ts
import { defineConfig, spawn } from "gnoblin";

export default defineConfig({
    keys: { "Super+Return": spawn("foot") },
});
```

Use an installed terminal. Your keys are added to the stock keys, so volume, media and workspace keys keep working.

To change a key that the stock config or an earlier file binds, bind the same trigger. Yours replaces it. This
replaces the stock volume-up key with a smaller step:

```ts
import { defineConfig, spawn } from "gnoblin";

export default defineConfig({
    keys: {
        XF86AudioRaiseVolume: spawn("wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "2%+"),
    },
});
```

| Part                  | Accepted values                                   | Effect                                                                                           |
| --------------------- | ------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| The trigger           | Modifiers and a key joined by `+`: `Super+Return` | Runs the action when the key is pressed. See [trigger syntax](/guides/shortcuts#trigger-syntax). |
| `spawn(program, ...)` | A program and its arguments, as strings           | Starts the program, passing the arguments, without shell expansion.                              |

Changes apply when you save the file. To drop a stock key, bind its trigger to another action. Use one string per
program argument. See [key names and conflict handling](/guides/shortcuts) for more.
