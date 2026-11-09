-- tests/lua/shell_test.lua: the stock shell (data/lua/gnoblin/shell/*.lua) over the fake core.
-- Failure modes 1, 2 and 7 of design section 8.2. Mode 8 needs Floating.place (section 8.4), which is not yet
-- written, so it is not tested here.
-- The shell is loaded into the same Lua state as node.lua, as the native host does. `Floating` is a test-only
-- subclass of Workspace: the real layout is written in section 8.4, and this shell only needs a class to create.
local H = require("harness")

-- Loads node.lua over `core` as a fresh runtime. A restart does this again over the same core, so the native
-- tree (the windows, the screen, the seat) survives it, as it does in Gnoblin.
local function boot(core, host)
    host.tick = assert(loadfile(GNOBLIN_LUA_DIR .. "/node.lua"))(core)
end

-- The shell files, then the test-only Floating layout. `sys` is the host's system table; the stub is inert.
local function load_shell()
    sys = sys or {
        spawn = function() end,
        date = function()
            return ""
        end,
    }
    dofile(GNOBLIN_LUA_DIR .. "/shell/ui.lua")
    dofile(GNOBLIN_LUA_DIR .. "/shell/init.lua")
    Floating = Workspace:extend("Floating")({})
end

-- The shell starts: every Screen and every Window gets `ready`, as a shell that comes up after its windows
-- have mapped would see them. Screens go first, so the Desk exists before any window is placed.
local function start_shell(host)
    local events = {}
    for _, class in ipairs({ "Screen", "Window" }) do
        for _, id in ipairs(host.ids(class)) do
            events[#events + 1] = { type = "ready", target = id }
        end
    end
    host.frame(0, events)
end

return {
    {
        "a window that maps before the shell waits under root, then is placed when the shell starts",
        function()
            local core, host = require("fake_core")()
            boot(core, host)
            root:add(Seat({}))
            root:add(Lock({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            local win = root:add(Window({ app_id = "foot" }))
            H.check(win.parent == root, "before the shell the window must wait under root")

            load_shell()
            root:find(Seat).screen = screen
            start_shell(host)
            H.check(win.parent ~= root, "the shell must place the window when it starts")
            H.check(win.parent == desk().active, "the window must go into desk().active")
            H.check(desk().active.name == "1", "the first workspace is active, got " .. tostring(desk().active.name))
        end,
    },

    {
        "a shell restart drops the old shell's nodes and keeps windows at their geometry",
        function()
            local core, host = require("fake_core")()
            boot(core, host)
            root:add(Seat({}))
            root:add(Lock({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            local win = root:add(Window({ app_id = "foot" }))
            load_shell()
            root:find(Seat).screen = screen
            start_shell(host)

            win.x, win.y, win.width, win.height = 40, 50, 300, 200
            host.frame(1, {})
            local old_desk = desk()
            H.check(#root:all(Desk) == 1, "the first shell must have made a Desk")

            -- Gnoblin returns the window to root and drops the old shell's subtree. Destroying the Desk is
            -- the same native call the host makes for every node the old shell created.
            root:add(win)
            old_desk:destroy()
            host.frame(2, {})
            host.frame(3, {})
            H.check(#root:all(Desk) == 0, "the old Desk must be dropped")
            H.check(#host.ids("Workspace") == 0, "the old workspaces must be dropped")
            H.check(win.parent == root, "the window must be back under root after the restart")

            boot(core, host)
            load_shell()
            start_shell(host)
            -- The new runtime has its own wrappers: the old `win` belongs to the discarded runtime.
            local new_win = root:all(Window)[1]
            H.check(#root:all(Desk) == 1, "exactly one Desk after the restart")
            H.check(new_win.parent == desk().active, "the window goes into the new desk().active")
            H.check(
                new_win.x == 40 and new_win.y == 50 and new_win.width == 300 and new_win.height == 200,
                ("geometry must be kept, got %s,%s %sx%s"):format(new_win.x, new_win.y, new_win.width, new_win.height)
            )
        end,
    },

    {
        "a dialog whose parent is closed has no parent_window and is placed as a normal window",
        function()
            local core, host = require("fake_core")()
            boot(core, host)
            root:add(Seat({}))
            root:add(Lock({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            load_shell()
            root:find(Seat).screen = screen
            start_shell(host)

            local parent = desk().active:add(Window({ app_id = "editor" }))
            local dialog = Window({ app_id = "editor", parent_window = parent })
            host.close(host.ids("Window")[1])
            host.frame(1, {})
            host.frame(2, {})
            H.check(dialog.parent_window == nil, "the closed parent must clear parent_window")

            root:add(dialog)
            H.check(dialog.parent == desk().active, "the dialog must be placed in desk().active")
        end,
    },

    {
        "a dialog whose parent is on another workspace goes with its parent",
        function()
            local core, host = require("fake_core")()
            boot(core, host)
            root:add(Seat({}))
            root:add(Lock({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            -- Made before the shell, so its one `ready` comes at start and it lands in workspace 1. Moving
            -- it afterwards is not a first add, so `ready` does not place it again.
            local parent = root:add(Window({ app_id = "editor" }))
            load_shell()
            root:find(Seat).screen = screen
            start_shell(host)

            local other = desk().space.children[2]
            other:add(parent)
            local dialog = Window({ app_id = "editor", parent_window = parent })
            root:add(dialog)
            H.check(dialog.parent == parent, "the dialog must become the parent's child")
            H.check(parent.parent == other, "the parent stays on its own workspace")
            H.check(desk().active ~= other, "showing the dialog must not switch workspace")
        end,
    },
}
