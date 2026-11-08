-- Device defaults and touchpad workspace gestures.
gnoblin.configure({
    cursor = {},

    input = {
        mouse = {},
        touchpad = {},
        trackball = {},
        ["pointing-stick"] = {},
        keyboard = {},
    },

    ["touchpad-gestures"] = {
        {
            name = "workspace-previous",
            gesture = "swipe",
            fingers = 3,
            path = { { x = 0, y = 0 }, { x = 1, y = 0 } },
            action = "workspace.previous",
        },
        {
            name = "workspace-next",
            gesture = "swipe",
            fingers = 3,
            path = { { x = 0, y = 0 }, { x = -1, y = 0 } },
            action = "workspace.next",
        },
    },
})
