# gnoblin.configure.xwayland

Configure Xwayland behavior in `gnoblin.configure`:

```lua
gnoblin.configure {
    xwayland = {
        allow_grabs = false,
        grab_access_rules = {"legacy-app", "!blocked-app"},
        disable_extensions = {},
        allow_byte_swapped_clients = false,
        scaling_factor = 0,
    },
}
```

| Setting                      | Accepted value                                                                                                                          | Default |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | ------- |
| `allow_grabs`                | Boolean. Allows supported X11 clients to take keyboard grabs on override-redirect windows.                                              | `false` |
| `grab_access_rules`          | Array of resource names or resource classes. `*` and `?` are wildcards; prefix a value with `!` to deny it. Deny rules take precedence. | `{}`    |
| `disable_extensions`         | Array containing `"security"`, `"xtest"`, both, or neither.                                                                             | `{}`    |
| `allow_byte_swapped_clients` | Boolean. Allows X11 clients with a different byte order to connect.                                                                     | `false` |
| `scaling_factor`             | `0` for automatic scaling, or a finite number from `0.5` through `2147483520`. Mutter rounds positive values to the nearest integer.    | `0`     |

Mutter always adds its compiled system grab-access rules. An empty
`grab_access_rules` list therefore removes only user-configured rules; it does
not remove those built-in rules. Grab rules only permit a grab when
`allow_grabs` is `true`.

`"security"` disables Xwayland's SECURITY extension. `"xtest"` disables its
XTEST extension, which lets clients synthesize input events. Mutter ignores a
request to disable an extension when Xwayland was built without that extension.

Changing `disable_extensions` or `allow_byte_swapped_clients` requires
restarting Xwayland; restart the Gnoblin session for those changes to affect
the running X server. The other settings can be reloaded during the session.

The upper scale limit keeps Mutter's conversion to a signed integer in range:
Mutter stores the value as a `float`, rounds it, and converts it to `int` for
Xwayland coordinate scaling. Values below `0.5` would round to zero, so they
are rejected. This is a representational limit, not a recommended display
scale.

## Type definition

```lua
gnoblin.configure {
    xwayland = {
        allow_grabs = boolean?,
        grab_access_rules = {string, ...}?,
        disable_extensions = {"security" | "xtest", ...}?,
        allow_byte_swapped_clients = boolean?,
        scaling_factor = number?, -- 0 or 0.5–2147483520
    },
}
```
