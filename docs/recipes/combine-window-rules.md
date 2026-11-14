# Combine window rules

Use one rule for shared defaults, then a later rule to override selected fields. Put this in `~/.config/gnoblin/init.ts`:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    rules: [
        { appId: "*", style: { opacity: 1 }, maximised: false },
        { focused: false, style: { opacity: 0.95 } },
    ],
});
```

The first rule applies to every window, because a glob of `*` matches any app id. The second changes only opacity when a
window loses focus; the `maximised` value from the first rule remains in effect. See
[window rules](/guides/window_rules#rule-order).
