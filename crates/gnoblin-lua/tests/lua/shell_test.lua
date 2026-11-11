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
    H.publish_seat(host, { screen = host.ids("Screen")[1] })
    H.start_shell(host)
    return host, win
end

-- Every write the shell made to a node's `x`, in order: the value, and whether it was a tween (it carried a duration).
local function x_writes(host, id)
    local out = {}
    for _, batch in ipairs(host.batches) do
        for _, o in ipairs(batch) do
            if o.op == "set" and o.id == id and o.key == "x" then
                out[#out + 1] = { value = o.value, tweened = o.seconds ~= nil }
            end
        end
    end
    return out
end

-- The position of the first write of `value` that is (or is not) a tween, or nil.
local function x_write_index(writes, value, tweened)
    for i, w in ipairs(writes) do
        if w.value == value and w.tweened == tweened then
            return i
        end
    end
end

local function dump_writes(writes)
    local parts = {}
    for _, w in ipairs(writes) do
        parts[#parts + 1] = tostring(w.value) .. (w.tweened and " (tween)" or "")
    end
    return table.concat(parts, ", ")
end

-- The focused node of the seat, or nil.
local function seat_focus()
    return root:find(Seat).focus
end

-- Publishes the seat's focus options as the compositor does: readonly seat props, changed the way the native side
-- reports its own changes (`host.external`), so the next frame sees them.
local function set_focus_options(host, opts)
    local seat = host.ids("Seat")[1]
    for k, v in pairs(opts) do
        host.external(seat, k, v)
    end
end

-- A second window, added after the shell started. The shell places it on its next frame.
local function second_window(host)
    root:add(Window({}))
    host.frame(0, {})
end

-- Whether `n` is the top of its parent's stack.
local function is_top(n)
    local c = n.parent.children
    return c[#c] == n
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
            H.publish_seat(host, { screen = host.ids("Screen")[1] })
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
            H.publish_seat(host, { screen = host.ids("Screen")[1] })
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
            H.publish_seat(host, { screen = host.ids("Screen")[1] })
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
            H.publish_seat(host, { screen = host.ids("Screen")[1] })
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
            set_focus_options(host, { focus_mode = "hover" })
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
            set_focus_options(host, { focus_mode = "hover-strict" })
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
            set_focus_options(host, { focus_mode = "hover", focus_change_on_pointer_rest = true })
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
            set_focus_options(host, { focus_mode = "hover", focus_change_on_pointer_rest = true })
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
            set_focus_options(host, { focus_mode = "hover", focus_change_on_pointer_rest = true })
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
            set_focus_options(host, { raise_on_click = false })
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
            H.publish_seat(host, { screen = host.ids("Screen")[1] })
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
    {
        "showing a later workspace starts it one width to the right and tweens it in while the old one tweens out to the left",
        function()
            local host, win = window_setup({ app_id = "foot" })
            local d = desk()
            local from, to = d.active, d.space.children[2]
            local from_id, to_id = host.ids("Workspace")[1], host.ids("Workspace")[2]
            H.frame(host, 1, {})
            H.check(d.width == 1000, "the desk is as wide as its screen: " .. tostring(d.width))
            go(function()
                d:show(to)
            end)
            H.check(to.visible and to.active and not from.active, "the new workspace is shown at once")
            H.frame(host, 1.1, {})
            H.frame(host, 1.5, {})
            local entering, leaving = x_writes(host, to_id), x_writes(host, from_id)
            H.check(
                x_write_index(entering, 1000, false),
                "the new workspace starts one output width to the right: " .. dump_writes(entering)
            )
            H.check(
                x_write_index(entering, 0, true),
                "the new workspace is tweened to the origin: " .. dump_writes(entering)
            )
            H.check(
                x_write_index(entering, 1000, false) < x_write_index(entering, 0, true),
                "it starts before it is tweened"
            )
            H.check(
                x_write_index(leaving, -1000, true),
                "the old workspace is tweened out to the left: " .. dump_writes(leaving)
            )
            H.check(to.x == 0 and to.visible, "the new workspace settles at the origin")
            H.check(from.visible == false and from.x == 0, "the old workspace is hidden and reset once it has left")
            H.check(win.parent == from, "the window stays on its workspace")
        end,
    },
    {
        "showing an earlier workspace starts it one width to the left and tweens it in while the old one tweens out to the right",
        function()
            local host = window_setup({ app_id = "foot" })
            local d = desk()
            local first, second = d.space.children[1], d.space.children[2]
            local first_id, second_id = host.ids("Workspace")[1], host.ids("Workspace")[2]
            H.frame(host, 1, {})
            go(function()
                d:show(second)
            end)
            H.frame(host, 1.5, {})
            H.frame(host, 1.6, {})
            H.check(d.active == second and second.x == 0, "the later workspace is shown first")
            local before = #x_writes(host, first_id)
            go(function()
                d:show(first)
            end)
            H.frame(host, 2.0, {})
            H.frame(host, 2.5, {})
            local entering, leaving = x_writes(host, first_id), x_writes(host, second_id)
            local started = {}
            for i = before + 1, #entering do
                started[#started + 1] = entering[i]
            end
            H.check(
                x_write_index(started, -1000, false),
                "the new workspace starts one output width to the left: " .. dump_writes(started)
            )
            H.check(
                x_write_index(started, 0, true),
                "the new workspace is tweened to the origin: " .. dump_writes(started)
            )
            H.check(
                x_write_index(leaving, 1000, true),
                "the old workspace is tweened out to the right: " .. dump_writes(leaving)
            )
            H.check(first.x == 0 and first.visible, "the new workspace settles at the origin")
            H.check(second.visible == false and second.x == 0, "the old workspace is hidden and reset once it has left")
        end,
    },
    {
        "a maximised window refits when a bar reserves space at the top (failure mode 9, maximised)",
        function()
            local host, win = window_setup({})
            win:maximise()
            H.check(win.x == 0 and win.y == 0 and win.width == 1000 and win.height == 800, "maximised fills the screen")
            desk():add(Bar({ edge = "top", height = 32 }))
            H.frame(host, 1, {})
            H.check(desk().space.y == 32, "the space must move below the bar, got " .. tostring(desk().space.y))
            H.check(win.x == 0 and win.y == 32, ("the window must move below the bar, got %d,%d"):format(win.x, win.y))
            H.check(
                win.width == 1000 and win.height == 768,
                ("the window must refit, got %dx%d"):format(win.width, win.height)
            )
        end,
    },
    {
        "a snapped window refits when a bar reserves space at the top (failure mode 9, snapped)",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            local win = root:add(Window({ app_id = "foot" }))
            H.load_shell()
            H.publish_seat(host, { screen = host.ids("Screen")[1], edge_tiling = true })
            H.start_shell(host)
            H.check(win:snap("right"), "the snap must tile")
            H.check(
                win.x == 500 and win.y == 0 and win.width == 500 and win.height == 800,
                "the right half must be tiled"
            )
            desk():add(Bar({ edge = "top", height = 32 }))
            H.frame(host, 1, {})
            H.check(
                win.x == 500 and win.y == 32,
                ("the snapped window must move below the bar, got %d,%d"):format(win.x, win.y)
            )
            H.check(
                win.width == 500 and win.height == 768,
                ("the snapped window must refit, got %dx%d"):format(win.width, win.height)
            )
        end,
    },
    {
        "a client's activate request is decided by the shell as an activation",
        function()
            local host, w = window_setup({})
            local a = host.ids("Window")[1]
            host.frame(0, { { type = "request", bubbles = true, target = a, request = "activate" } })
            H.check(root:find(Seat).focus == w, "an activate request must focus")
        end,
    },
    {
        "an armed auto-raise raises a buried window once its delay passes",
        function()
            local host, w = window_setup({})
            local a = host.ids("Window")[1]
            second_window(host)
            H.check(not is_top(w), "a starts buried under b")
            set_focus_options(host, { focus_mode = "hover", auto_raise = true, auto_raise_delay = 500 })
            host.frame(0.1, { { type = "enter", target = a, bubbles = true } })
            host.frame(0.9, {})
            H.check(is_top(w), "auto-raise must raise a after 0.5 s")
        end,
    },
    {
        "the seat's auto_raise_delay sets how long an armed auto-raise waits",
        function()
            local host, w = window_setup({})
            local a = host.ids("Window")[1]
            second_window(host)
            set_focus_options(host, { focus_mode = "hover", auto_raise = true, auto_raise_delay = 1500 })
            host.frame(0.1, { { type = "enter", target = a, bubbles = true } })
            host.frame(0.9, {})
            H.check(not is_top(w), "a 1.5 s delay must not raise after 0.8 s")
            host.frame(1.7, {})
            H.check(is_top(w), "auto-raise must raise a after the 1.5 s delay")
        end,
    },
    {
        "a leave cancels an armed auto-raise",
        function()
            local host, w = window_setup({})
            local a = host.ids("Window")[1]
            second_window(host)
            set_focus_options(host, { focus_mode = "hover", auto_raise = true, auto_raise_delay = 500 })
            host.frame(0.1, { { type = "enter", target = a, bubbles = true } })
            host.frame(0.2, { { type = "leave", target = a, bubbles = true } })
            host.frame(0.9, {})
            H.check(not is_top(w), "a leave must cancel the armed raise")
        end,
    },
    {
        "closing an armed window cancels its auto-raise",
        function()
            local host, w = window_setup({})
            local a = host.ids("Window")[1]
            second_window(host)
            set_focus_options(host, { focus_mode = "hover", auto_raise = true, auto_raise_delay = 500 })
            host.frame(0.1, { { type = "enter", target = a, bubbles = true } })
            host.frame(0.2, { { type = "closed", target = a, args = {} } })
            host.frame(0.9, {})
            H.check(w.parent == nil, "the closed window must have left the tree, so no raise can fire")
        end,
    },
    {
        "a closed window cancels its pending rest focus",
        function()
            local host, w = window_setup({})
            local a = host.ids("Window")[1]
            root:find(Seat).focus = nil
            set_focus_options(host, { focus_mode = "hover", focus_change_on_pointer_rest = true })
            host.frame(0, { { type = "enter", target = a, bubbles = true } })
            host.frame(0.05, { { type = "closed", target = a, args = {} } })
            host.frame(0.4, {})
            H.check(root:find(Seat).focus == nil, "a closed window must not take focus on its rest")
        end,
    },
    {
        "a focus change while a button is held is ignored",
        function()
            local host, w = window_setup({})
            local a = host.ids("Window")[1]
            second_window(host)
            local b = host.ids("Window")[2]
            root:find(Seat).focus = nil
            set_focus_options(host, { focus_mode = "hover-strict" })
            host.frame(0, { { type = "press", target = a, bubbles = true } })
            host.frame(0.05, { { type = "enter", target = b, bubbles = true } })
            H.check(root:find(Seat).focus == w, "a held button keeps focus on the pressed window")
        end,
    },
    {
        "activating a window on a hidden workspace is refused and leaves focus alone",
        function()
            local host, w = window_setup({})
            local a = host.ids("Window")[1]
            root:find(Seat).focus = nil
            desk().space.children[2]:add(w)
            host.frame(0, { { type = "activate", target = a, args = {} } })
            H.check(root:find(Seat).focus == nil, "a hidden window must not take focus")
        end,
    },
    {
        "activating the focused window again changes nothing",
        function()
            local host, w = window_setup({})
            local a = host.ids("Window")[1]
            host.frame(0, { { type = "activate", target = a, args = {} } })
            host.frame(0.1, { { type = "activate", target = a, args = {} } })
            H.check(root:find(Seat).focus == w, "repeat activation keeps the focus on the window")
        end,
    },
    {
        "moving a window to another workspace takes it there",
        function()
            local host, w = window_setup({})
            local other = desk().space.children[2]
            w:move_to(other)
            H.check(w.parent == other, "the window must sit on the target workspace")
        end,
    },
    {
        "moving a window to a workspace that is not in any desk is rejected",
        function()
            local host, w = window_setup({})
            local home = w.parent
            local stray = Floating({})
            w:move_to(stray)
            H.check(w.parent == home, "an unknown workspace must leave the window where it was")
        end,
    },
    {
        "moving a window to its current workspace is a no-op",
        function()
            local host, w = window_setup({})
            local home = w.parent
            local before = #home.children
            w:move_to(home)
            H.check(w.parent == home and #home.children == before, "moving to the current workspace changes nothing")
        end,
    },
    {
        "a focused window moved to another workspace hands focus on",
        function()
            local host, w = window_setup({})
            second_window(host)
            local other_win
            for _, n in ipairs(root:all(Window)) do
                if n ~= w then
                    other_win = n
                end
            end
            root:find(Seat).focus = w
            w:move_to(desk().space.children[2])
            H.check(root:find(Seat).focus ~= w, "the moved window must give up focus")
            H.check(root:find(Seat).focus == other_win, "focus must pass to the window left on the active workspace")
        end,
    },
    {
        "moving a window to an output that is not a screen is rejected",
        function()
            local host, w = window_setup({})
            local home = w.parent
            local stray = Screen({ width = 500, height = 500 })
            w:move_to_output(stray)
            H.check(w.parent == home, "an unknown output must leave the window where it was")
        end,
    },
    {
        "moving a window to another output reshapes it into that output's work area",
        function()
            local host, w = window_setup({})
            local second = root:add(Screen({ width = 600, height = 400 }))
            w:move_to_output(second)
            H.check(w.x >= 0 and w.x + w.width <= 600, "the window must fit the new output horizontally")
            H.check(w.y >= 0 and w.y + w.height <= 400, "the window must fit the new output vertically")
        end,
    },
    {
        "a maximised window fills the new output when it moves",
        function()
            local host, w = window_setup({})
            w:maximise()
            local second = root:add(Screen({ width = 600, height = 400 }))
            w:move_to_output(second)
            H.check(w.maximised, "the window must stay maximised")
            H.check(w.x == 0 and w.y == 0 and w.width == 600 and w.height == 400, "it must fill the new output")
        end,
    },
    {
        "a fullscreen window does not move to another workspace",
        function()
            local host, w = window_setup({})
            w.fullscreen = true
            local home = w.parent
            w:move_to(desk().space.children[2])
            H.check(w.parent == home, "a fullscreen window must stay on its workspace")
        end,
    },
    {
        "a fullscreen window does not move to another output",
        function()
            local host, w = window_setup({})
            local home = w.parent
            w.fullscreen = true
            local second = root:add(Screen({ width = 600, height = 400 }))
            H.check(w:move_to_output(second) == false, "a fullscreen window must refuse an output move")
            H.check(w.parent == home, "a refused window must stay on its workspace")
            H.check(w.fullscreen, "a refused window must stay fullscreen")
        end,
    },
    {
        "a dialog does not move to another workspace on its own",
        function()
            local host, w = window_setup({})
            local dialog = root:add(Window({ app_id = "dialog", parent_window = w }))
            H.check(dialog.parent == w, "the dialog must be the parent window's child")
            H.check(dialog:move_to(desk().space.children[2]) == false, "a dialog move must be refused")
            H.check(dialog.parent == w, "a refused dialog must stay with its parent")
        end,
    },
    {
        "moving a parent window takes its dialog with it",
        function()
            local host, w = window_setup({})
            local dialog = root:add(Window({ app_id = "dialog", parent_window = w }))
            local target = desk().space.children[2]
            w:move_to(target)
            H.check(dialog:closest(Workspace) == target, "the dialog must follow its parent")
        end,
    },
    {
        "a dialog follows its parent to another output",
        function()
            local host, w = window_setup({})
            local dialog = root:add(Window({ app_id = "dialog", parent_window = w }))
            local second = root:add(Screen({ width = 600, height = 400 }))
            w:move_to_output(second)
            H.check(dialog:closest(Desk) == desk_of(second), "the dialog must follow its parent to the new output")
        end,
    },
    {
        "unplugging the last output keeps its windows hidden and drops focus",
        function()
            local host, w = window_setup({})
            root:find(Seat).focus = w
            host.close(host.ids("Screen")[1])
            H.frame(host, 1, {})
            H.frame(host, 2, {})
            H.check(w.parent ~= nil, "the window must survive the unplug")
            H.check(w:closest(Workspace).visible == false, "the window's workspace must be hidden")
            H.check(seat_focus() == nil, "a window with no output cannot hold focus")
        end,
    },
    {
        "reconnecting an output adopts the parked windows and focuses one",
        function()
            local host, w = window_setup({})
            root:find(Seat).focus = w
            host.close(host.ids("Screen")[1])
            H.frame(host, 1, {})
            H.frame(host, 2, {})
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            H.frame(host, 3, {})
            H.check(w:closest(Workspace).visible, "the adopted workspace must show")
            H.check(w:closest(Desk).screen == screen, "the window must be on the new output")
            H.check(seat_focus() == w, "reconnecting must focus the window again")
        end,
    },
    {
        "unplugging one of two outputs moves its windows to the other output",
        function()
            local host, w = window_setup({})
            local second = root:add(Screen({ width = 600, height = 400 }))
            host.close(host.ids("Screen")[1])
            H.frame(host, 1, {})
            H.frame(host, 2, {})
            H.check(w:closest(Desk) == desk_of(second), "the window must move to the remaining output")
        end,
    },
    {
        "removing a workspace moves its windows to the neighbour before it",
        function()
            local host, w = window_setup({})
            local d = desk()
            local ws1, ws2 = d.space.children[1], d.space.children[2]
            ws2:add(w)
            H.check(d:remove_workspace(ws2) == true, "a workspace with windows must be removable")
            H.check(w:closest(Workspace) == ws1, "the window must move to the neighbour before the removed workspace")
            H.check(d.active == ws1, "removing an inactive workspace must keep the active one")
            H.check(#d.space.children == 8, "the removed workspace must leave the desk")
        end,
    },
    {
        "removing the active workspace shows its neighbour and refocuses the moved window",
        function()
            local host, w = window_setup({})
            root:find(Seat).focus = w
            local d = desk()
            local ws2 = d.space.children[2]
            H.check(d:remove_workspace(d.active) == true, "the active workspace must be removable")
            H.check(d.active == ws2, "the neighbour must become active")
            H.check(ws2.visible, "the neighbour must be visible")
            H.check(w:closest(Workspace) == ws2, "the window must move to the neighbour")
            H.check(seat_focus() == w, "the moved window must keep focus")
        end,
    },
    {
        "removing an inactive workspace keeps focus on the focused window and moves the stray window beside it",
        function()
            local host, w = window_setup({})
            local d = desk()
            local ws1, ws2 = d.space.children[1], d.space.children[2]
            local stray = ws2:add(Window({ app_id = "stray" }))
            root:find(Seat).focus = w
            H.check(d:remove_workspace(ws2) == true, "an inactive workspace must be removable")
            H.check(stray:closest(Workspace) == ws1, "the stray window must move to the neighbour before it")
            H.check(d.active == ws1, "the active workspace must stay")
            H.check(seat_focus() == w, "the focused window must keep focus when another workspace goes")
        end,
    },
    {
        "the last workspace of a desk cannot be removed",
        function()
            local host, w = window_setup({})
            local d = desk()
            while #d.space.children > 1 do
                H.check(d:remove_workspace(d.space.children[#d.space.children]) == true, "a spare workspace must go")
            end
            H.check(d:remove_workspace(d.active) == false, "the last workspace must be refused")
            H.check(#d.space.children == 1, "the refused workspace must stay")
        end,
    },
    {
        "a Desk builds the workspaces the seat declares, named by id, and nine when none are declared",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            root:add(Screen({ width = 800, height = 600 }))
            H.load_shell()
            H.publish_seat(host, { screen = host.ids("Screen")[1], workspaces = { "code:Code", "web:Web" } })
            H.start_shell(host)
            local d = desk()
            H.check(#d.space.children == 2, "two workspaces, got " .. #d.space.children)
            H.check(d.space.children[1].name == "code" and d.space.children[2].name == "web", "named by id")
            H.check(d.space.children[1].data.title == "Code", "the title is the declared name")
            H.check(d.active == d.space.children[1], "the first is active")
        end,
    },
    {
        "a Desk with no declared workspaces builds nine, named 1 to 9",
        function()
            local host = window_setup({})
            local d = desk()
            H.check(#d.space.children == 9, "nine workspaces, got " .. #d.space.children)
            H.check(d.space.children[9].name == "9", "named by position")
        end,
    },
}
