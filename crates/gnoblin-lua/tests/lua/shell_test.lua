-- tests/lua/shell_test.lua: the stock shell (data/lua/gnoblin/shell/*.lua) over the fake core.
-- Failure modes 1, 2 and 7 of design section 8.2. Placement modes are in floating_test.lua.
-- The shell is loaded into the same Lua state as node.lua, as the native host does. Its Desk workspaces are the
-- stock Floating class, from shell/floating.lua.
local H = require("harness")

-- The stock shell over a fake core with one Seat, as floating_test sets it up (no Screen needed for claims).
local function seat_shell()
    local core, host = require("fake_core")()
    H.boot(core, host)
    root:add(Seat({}))
    H.load_shell()
    H.start_shell(host)
    return core, host
end

return {
    {
        "the stock shell loads through require in a bare VM and reads no global the host does not define",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            -- Everything the test needs is captured before strict mode, so only the shell's own reads are checked.
            local req, dir = require, GNOBLIN_LUA_DIR
            sys = {
                spawn = function() end,
                date = function()
                    return ""
                end,
            }
            package.path = dir .. "/../?.lua;" .. dir .. "/../?/init.lua;" .. package.path
            for name in pairs(package.loaded) do
                if name == "gnoblin.shell" or name:find("^gnoblin%.shell%.") then
                    package.loaded[name] = nil
                end
            end
            local missing = {}
            setmetatable(_G, {
                __index = function(_, k)
                    missing[#missing + 1] = tostring(k)
                    return nil
                end,
            })
            req("gnoblin.shell")
            setmetatable(_G, nil)
            H.check(#missing == 0, "the shell read undefined globals: " .. table.concat(missing, ", "))
        end,
    },
    {
        "a window that maps before the shell waits under root, then is placed when the shell starts",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            root:add(Lock({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            local win = root:add(Window({ app_id = "foot" }))
            H.check(win.parent == root, "before the shell the window must wait under root")

            H.load_shell()
            root:find(Seat).screen = screen
            H.start_shell(host)
            H.check(win.parent ~= root, "the shell must place the window when it starts")
            H.check(win.parent == desk().active, "the window must go into desk().active")
            H.check(desk().active.name == "1", "the first workspace is active, got " .. tostring(desk().active.name))
        end,
    },

    {
        "a shell restart drops the old shell's nodes and keeps windows at their geometry",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            root:add(Lock({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            local win = root:add(Window({ app_id = "foot" }))
            H.load_shell()
            root:find(Seat).screen = screen
            H.start_shell(host)

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

            H.boot(core, host)
            H.load_shell()
            H.start_shell(host)
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
            H.boot(core, host)
            root:add(Seat({}))
            root:add(Lock({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            H.load_shell()
            root:find(Seat).screen = screen
            H.start_shell(host)

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
            H.boot(core, host)
            root:add(Seat({}))
            root:add(Lock({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            -- Made before the shell, so its one `ready` comes at start and it lands in workspace 1. Moving
            -- it afterwards is not a first add, so `ready` does not place it again.
            local parent = root:add(Window({ app_id = "editor" }))
            H.load_shell()
            root:find(Seat).screen = screen
            H.start_shell(host)

            local other = desk().space.children[2]
            other:add(parent)
            local dialog = Window({ app_id = "editor", parent_window = parent })
            root:add(dialog)
            H.check(dialog.parent == parent, "the dialog must become the parent's child")
            H.check(parent.parent == other, "the parent stays on its own workspace")
            H.check(desk().active ~= other, "showing the dialog must not switch workspace")
        end,
    },
    -- Seat:claim returns a fresh id per claim and sends the chord and grab to the compositor.
    {
        "seat claim returns increasing ids and queues a claim call carrying the chord",
        function()
            local core, host = seat_shell()
            local s = root:find(Seat)
            local a = s:claim("Super+q", true)
            local b = s:claim("Super+w", false)
            H.check(b == a + 1, "ids " .. tostring(a) .. ", " .. tostring(b))
            local c = host.calls[#host.calls]
            H.check(c.method == "claim", "last call " .. c.method)
            H.check(c.text == "Super+w", "chord text " .. tostring(c.text))
            local id, grab = string.unpack("<i8i8", c.args[1])
            H.check(id == b and grab == 0, "args id " .. id .. " grab " .. grab)
        end,
    },

    -- A chord event reaches the fn bound under its claim id on press only; unbind unclaims.
    {
        "a chord event runs the bound fn once on press and unbind stops later events",
        function()
            local _, host = seat_shell()
            local s = root:find(Seat)
            local hits = {}
            s:bind({
                ["Super+q"] = function(e)
                    hits[#hits + 1] = e.state
                end,
            })
            local id
            for _, c in ipairs(host.calls) do
                if c.method == "claim" and c.text == "Super+q" then
                    id = string.unpack("<i8", c.args[1])
                end
            end
            H.check(id ~= nil, "bind sent no claim for Super+q")
            local function chord_event(state)
                local chord = { claim = id, chord = "Super+q", state = state }
                return { target = 0, type = "chord", bubbles = false, chord = chord }
            end
            host.frame(0, { chord_event("press") })
            host.frame(1, { chord_event("release") })
            H.check(#hits == 1 and hits[1] == "press", "hits " .. #hits)
            s:unbind(id)
            local last = host.calls[#host.calls]
            H.check(last.method == "unclaim", "unbind sent " .. last.method)
            local uid = string.unpack("<i8", last.args[1])
            H.check(uid == id, "unclaim id " .. uid)
            host.frame(2, { chord_event("press") })
            H.check(#hits == 1, "fn ran after unbind: " .. #hits)
        end,
    },
}
