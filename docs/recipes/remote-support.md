# Prepare remote support access

Ask for portal permission by default, then allow one trusted support program to
share the primary monitor and control the keyboard and pointer. Replace the
executable path with the installed program you trust.

```lua
gnoblin.configure {
    permissions = {default = "ask"},
}

gnoblin.permission_rule {
    name = "support-tool",
    match = [[^host-exe:/usr/bin/rustdesk$]],
    capabilities = {"screen-cast", "remote-desktop"},
    level = "allow",
    monitors = {"primary"},
    devices = {"keyboard", "pointer"},
    clipboard = false,
}
```

This grants the rule only the listed monitor and input devices; clipboard
sharing stays off. Other applications still go through the consent flow. The
`host-exe:` identity must be an exact absolute path, and automatic approval
requires Gnoblin's patched portal backend. These rules do not authenticate
processes or isolate applications running as your Unix user.

Reload the configuration and inspect the decision before connecting:

```sh
gnoblinctl config reload
gnoblinctl permissions check screen-cast host-exe:/usr/bin/rustdesk
```

The check reports policy for the identity you supplied; it does not verify the
running process or monitor availability. Read the full
[permissions guide](/guides/permissions) before granting remote control.
