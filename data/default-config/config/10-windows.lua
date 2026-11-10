-- Window behavior and workspaces.
-- Setting names are dashed, so they are written in brackets: Lua identifiers cannot contain a dash.
gnoblin.configure({
    ["window-management"] = {
        ["focus-mode"] = "click", -- "click", "hover", "hover-strict"
        ["focus-new-windows"] = "allow", -- windows started from a launcher take focus; "prevent" keeps focus where it is.
        ["raise-on-click"] = true,
        ["auto-raise"] = false,
        ["auto-raise-delay"] = 500, -- milliseconds, 0..10000
        ["check-alive-timeout"] = 5000, -- client liveness ping timeout in milliseconds; 0 disables it
        ["focus-change-on-pointer-rest"] = false,
        ["workspaces-only-on-primary"] = true, -- windows on other monitors follow the active workspace, as in GNOME.
        ["edge-tiling"] = true, -- drag a window to a screen edge to tile it.
        ["attach-modal-dialogs"] = true, -- GNOME Shell sets this too; dialogs stay with their parent.
        ["constrain-drag-to-work-area"] = true,

        ["action-double-click-titlebar"] = "toggle-maximize",
        ["action-middle-click-titlebar"] = "none",
        ["action-right-click-titlebar"] = "menu",
    },

    workspaces = {
        { id = "1", name = "Workspace 1" },
        { id = "2", name = "Workspace 2" },
        { id = "3", name = "Workspace 3" },
        { id = "4", name = "Workspace 4" },
    },
})
