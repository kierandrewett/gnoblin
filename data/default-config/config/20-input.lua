-- Device defaults. Touchpad gestures are the shell's: the stock shell binds a three-finger swipe to the workspace
-- slide (shell/gestures.lua), and `touchpad-gestures` entries here only start commands.
gnoblin.configure({
    cursor = {},

    input = {
        mouse = {},
        touchpad = {},
        trackball = {},
        ["pointing-stick"] = {},
        keyboard = {},
    },
})
