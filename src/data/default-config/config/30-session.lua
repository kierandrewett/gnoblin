-- Session services, protocol defaults and permissions.
gnoblin.configure {
    compositor = {
        locate_pointer = false,
        locate_pointer_key = "Control_L", -- XKB keysym name; "disabled" removes the trigger.
        visual_bell = false,
        audible_bell = true,
        visual_bell_type = "fullscreen-flash", -- or "frame-flash"
    },

    permissions = {
        default = "inherit",
        rules = {}, -- up to 256 rules; see the example below
    },

    protocols = {},

    layer_shell = {
        preserve_active_window = true,
    },

}
