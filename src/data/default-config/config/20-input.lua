-- Device defaults and touchpad workspace gestures.
gnoblin.configure {
    cursor = {},

    input = {
        mouse = {
        },
        touchpad = {
        },
        trackball = {
        },
        pointing_stick = {
        },
        keyboard = {
        },
    },

    touchpad_gestures = {
        {name = "workspace-previous", gesture = "swipe", fingers = 3, path = {{x = 0, y = 0}, {x = 1, y = 0}},
            action = "workspace.previous", when = "unlocked"},
        {name = "workspace-next", gesture = "swipe", fingers = 3, path = {{x = 0, y = 0}, {x = -1, y = 0}},
            action = "workspace.next", when = "unlocked"},
    },

}
