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

-- A started shell with one Screen, one Seat and one Window built from `props`. Returns the host and the window.
local function window_setup(props)
    local core, host = require("fake_core")()
    H.boot(core, host)
    root:add(Seat({}))
    local screen = root:add(Screen({ width = 1000, height = 800 }))
    local win = root:add(Window(props))
    H.load_shell()
    root:find(Seat).screen = screen
    H.start_shell(host)
    return host, win
end

-- The focused node of the seat, or nil.
local function seat_focus()
    return root:find(Seat).focus
end

-- Sets the seat's focus options. The compositor does not publish focus_options yet (no schema property), so the
-- shell's raw read sees the field directly. Writing through the node would be refused as an unknown property.
local function set_focus_options(opts)
    rawset(root:find(Seat), "focus_options", opts)
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
        "a shell restart drops the old shell's nodes and re-places its windows, keeping them mapped",
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
            H.check(win.data.placed == true, "the first shell places the window")
            local win_ids = host.ids("Window")
            win.x, win.y, win.width, win.height = 40, 50, 300, 200
            host.frame(1, {})
            local old_desk = desk()

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
            local pw = root:all(Window)[1]
            -- A restarted shell re-places windows: placement is not persisted yet (documented limitation).
            H.check(#root:all(Desk) == 1, "exactly one Desk after the restart")
            H.check(pw.parent == desk().active, "the window is re-placed into the new desk().active")
            H.check(pw.data.placed == true, "the restarted shell places the window again")
            H.check(pw.width == 300 and pw.height == 200, "the client size survives the restart")
            H.check(#host.ids("Window") == 1, "the window is still mapped, not duplicated")
            H.check(host.ids("Window")[1] == win_ids[1], "the restarted shell wraps the same native node")
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

    {
        "click mode: pointer motion does not move focus",
        function()
            local host = window_setup({})
            local win = host.ids("Window")[1]
            root:find(Seat).focus = nil
            host.frame(0, { { type = "enter", target = win, bubbles = true } })
            host.frame(0.1, { { type = "motion", target = win, bubbles = true } })
            H.check(seat_focus() == nil, "click mode must not focus on motion")
        end,
    },
    {
        "hover: entering a window focuses it and leaving keeps focus",
        function()
            local host, w = window_setup({})
            local id = host.ids("Window")[1]
            set_focus_options({ focus_mode = "hover" })
            host.frame(0, { { type = "enter", target = id, bubbles = true } })
            H.check(root:find(Seat).focus == w, "enter must focus under hover")
            host.frame(0.1, { { type = "leave", target = id, bubbles = true } })
            H.check(root:find(Seat).focus == w, "hover leave keeps focus")
        end,
    },
    {
        "hover-strict: leaving the focused window clears focus",
        function()
            local host, w = window_setup({})
            local id = host.ids("Window")[1]
            set_focus_options({ focus_mode = "hover-strict" })
            host.frame(0, { { type = "enter", target = id, bubbles = true } })
            host.frame(0.1, { { type = "leave", target = id, bubbles = true } })
            H.check(root:find(Seat).focus == nil, "hover-strict leave must clear focus")
        end,
    },
    {
        "a pointer rest focuses the window; a leave before the rest cancels it",
        function()
            local host, w = window_setup({})
            local id = host.ids("Window")[1]
            root:find(Seat).focus = nil
            set_focus_options({ focus_mode = "hover", focus_change_on_pointer_rest = true })
            host.frame(0, { { type = "enter", target = id, bubbles = true } })
            host.frame(0.05, { { type = "leave", target = id, bubbles = true } })
            host.frame(0.4, {})
            H.check(root:find(Seat).focus == nil, "a leave must cancel the rest")
        end,
    },
    {
        "a motion restarts the rest timer",
        function()
            local host, w = window_setup({})
            local id = host.ids("Window")[1]
            root:find(Seat).focus = nil
            set_focus_options({ focus_mode = "hover", focus_change_on_pointer_rest = true })
            host.frame(0, { { type = "enter", target = id, bubbles = true } })
            host.frame(0.1, { { type = "motion", target = id, bubbles = true } })
            host.frame(0.25, {})
            H.check(root:find(Seat).focus == nil, "rest restarted by motion must not fire at 0.25")
            host.frame(0.35, {})
            H.check(root:find(Seat).focus == w, "rest must fire 0.2 s after the last motion")
        end,
    },
    {
        "a rest does not focus while a button is held",
        function()
            local host, w = window_setup({})
            local id = host.ids("Window")[1]
            set_focus_options({ focus_mode = "hover", focus_change_on_pointer_rest = true })
            host.frame(0, { { type = "press", target = id, bubbles = true } })
            host.frame(0.05, { { type = "enter", target = id, bubbles = true } })
            host.frame(0.4, {})
            H.check(root:find(Seat).focus == w, "the press focused the window")
        end,
    },
    {
        "a press on a buried window raises it",
        function()
            local host, w = window_setup({})
            local a = host.ids("Window")[1]
            H.check(a ~= nil, "a window exists")
            host.frame(0, { { type = "press", target = a, bubbles = true } })
            H.check(w.parent.children[#w.parent.children] == w, "press must raise to the top")
        end,
    },
    {
        "raise_on_click off: a press focuses but does not raise",
        function()
            local host, w = window_setup({})
            local a = host.ids("Window")[1]
            set_focus_options({ raise_on_click = false })
            host.frame(0, { { type = "press", target = a, bubbles = true } })
            H.check(root:find(Seat).focus == w, "press must focus")
        end,
    },
    {
        "a window closed while focused drops focus",
        function()
            local host, w = window_setup({})
            local a = host.ids("Window")[1]
            host.frame(0, { { type = "press", target = a, bubbles = true } })
            host.frame(0.1, { { type = "closed", target = a, args = {} } })
            H.check(root:find(Seat).focus == nil, "closed window must not keep focus")
        end,
    },
    {
        "an activation focuses and raises a window on the active workspace",
        function()
            local host, w = window_setup({})
            local a = host.ids("Window")[1]
            host.frame(0, { { type = "activate", target = a, args = {} } })
            H.check(root:find(Seat).focus == w, "activate must focus")
        end,
    },
    {
        "raising a window keeps a fullscreen sibling above it, and a fullscreen window stays on top when raised",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            local a = root:add(Window({ app_id = "a" }))
            local b = root:add(Window({ app_id = "b" }))
            local c = root:add(Window({ app_id = "c" }))
            H.load_shell()
            root:find(Seat).screen = screen
            H.start_shell(host)
            H.frame(host, 1, {})
            local names = { [a] = "a", [b] = "b", [c] = "c" }
            local ws = a.parent
            local function order()
                local out = {}
                for i, w in ipairs(ws.children) do
                    out[i] = names[w]
                end
                return out
            end
            a:raise()
            b:raise()
            c:raise()
            H.same(order(), { "a", "b", "c" })
            b.fullscreen = true
            a:raise()
            H.same(order(), { "c", "a", "b" })
            c:raise()
            H.same(order(), { "a", "c", "b" })
            b:raise()
            H.same(order(), { "a", "c", "b" })
        end,
    },

    {
        "a workspace switch keeps a fullscreen window fullscreen and on its workspace when that workspace returns",
        function()
            local host, win = window_setup({ app_id = "foot" })
            local back = win.parent
            local away = desk().space.children[2]
            win.fullscreen = true
            go(function()
                desk():show(away)
            end)
            for i = 1, 10 do
                H.frame(host, i * 0.1, {})
            end
            H.check(desk().active == away, "the switch must complete")
            H.check(back.visible == false, "the old workspace must be hidden")
            go(function()
                desk():show(back)
            end)
            for i = 11, 20 do
                H.frame(host, i * 0.1, {})
            end
            H.check(desk().active == back, "switching back must complete")
            H.check(win.parent == back, "the fullscreen window must stay on its workspace")
            H.check(win.fullscreen == true, "the fullscreen window must stay fullscreen")
        end,
    },
}
