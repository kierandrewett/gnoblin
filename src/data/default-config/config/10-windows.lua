-- Window behavior, workspaces and the built-in native frame.
gnoblin.configure {
    window_management = {
        focus_mode = "click", -- "click", "sloppy", "mouse"
        focus_new_windows = "smart", -- windows started from a launcher take focus; "strict" keeps focus where it is.
        raise_on_click = true,
        auto_raise = false,
        auto_raise_delay = 500, -- milliseconds, 0..10000
        check_alive_timeout = 5000, -- client liveness ping timeout in milliseconds; 0 disables it
        focus_change_on_pointer_rest = false,
        workspaces_only_on_primary = true, -- windows on other monitors follow the active workspace, as in GNOME.
        edge_tiling = true, -- drag a window to a screen edge to tile it.
        center_new_windows = true,
        attach_modal_dialogs = true, -- GNOME Shell sets this too; dialogs stay with their parent.
        constrain_drag_to_work_area = true,

        action_double_click_titlebar = "toggle-maximize",
        action_middle_click_titlebar = "none",
        action_right_click_titlebar = "menu",
    },

    workspaces = {
        {id = "1", name = "Workspace 1"},
        {id = "2", name = "Workspace 2"},
        {id = "3", name = "Workspace 3"},
        {id = "4", name = "Workspace 4"},
    },

    frame_renderers = {},

}

gnoblin.window_rule {
    match = {type = "window"},
    frame = {
        mode = "auto",
        renderer = "native",
        extents = {32, 1, 1, 1},
    },
}

-- Soft rounded corners and a shadow. Apps that already draw rounded corners,
-- such as libadwaita apps, keep their own shape. Remove or edit this rule to
-- change the look. See the window_effects guide for every field.
gnoblin.window_rule {
    match = {type = "window"},
    corners = {
        radius = 12,
        keep_maximized = false, -- square corners when maximised, like GNOME
        smoothing = 0, -- circular corners match libadwaita; higher values square the shadow
        shadow = {x = 0, y = 8, spread = 0, blur = 32, opacity = 0.35},
    },
}

-- Without a shell, nothing shows the attention flag Mutter sets on a dialog
-- that opens behind the focused window, such as a portal prompt. Raise modal
-- dialogs so they stay visible. Focus stays where it is under "strict".
gnoblin.on("gnoblin.window.created", function(event)
    local window = event.window
    if window.modal then
        window:set_above(true)
    end
end)
