-- Session services and protocol defaults.
gnoblin.configure({
    compositor = {
        ["locate-pointer"] = false,
        ["locate-pointer-key"] = "Control_L", -- XKB keysym name; "disabled" removes the trigger.
        ["visual-bell"] = false,
        ["audible-bell"] = true,
        ["visual-bell-type"] = "fullscreen-flash", -- or "frame-flash"
    },

    protocols = {},

    ["layer-shell"] = {
        ["preserve-active-window"] = true,
    },
})
