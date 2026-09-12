-- Minimal isolated-session configuration for the RustDesk portal proof.
local g = require("gnoblin")

g.set({
    permissions = {
        default = "deny",
        rules = {
            {
                name = "rustdesk",
                match = "^app-id:com\\.rustdesk\\.RustDesk$",
                capabilities = {"screen-cast", "remote-desktop", "input-capture", "screenshot", "access"},
                level = "allow",
                monitors = {"primary"},
                devices = {"keyboard", "pointer", "touchscreen"},
                clipboard = true,
            },
        },
    },
})
