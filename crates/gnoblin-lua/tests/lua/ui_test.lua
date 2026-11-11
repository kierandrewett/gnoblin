-- tests/lua/ui_test.lua: the stock lock, overview and panel modules (data/lua/gnoblin/shell/) over the fake core.
-- The lock answers the native core through slow calls: the host answers each token with an event named after it.
local H = require("harness")

-- A started shell with one Screen, one Seat and one Lock. Returns the host, the lock's id and its node.
local function setup()
    local core, host = require("fake_core")()
    H.boot(core, host)
    root:add(Seat({}))
    local lock = root:add(Lock({}))
    local screen = root:add(Screen({ width = 1000, height = 800 }))
    H.load_shell()
    root:find(Seat).screen = screen
    H.start_shell(host)
    return host, host.ids("Lock")[1], lock
end

-- The token the lock's last authenticate call got back from the native core.
local function last_token(host)
    local call = host.calls[#host.calls]
    H.check(call and call.method == "authenticate", "the last native call must be authenticate")
    return call.result
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

-- The native ids of a node's children, read from the host's dump.
local function children_of(host, id)
    local out = {}
    for line in host.dump():gmatch("[^\n]+") do
        local child, parent = line:match("^(%d+) %S+ parent=(%S+)")
        if parent == tostring(id) then
            out[#out + 1] = tonumber(child)
        end
    end
    return out
end

-- The native ids of the titlebar (the window's first child) and its Icon (the titlebar's second child).
local function titlebar_ids(host)
    local win_id = host.ids("Window")[1]
    local bar = children_of(host, win_id)[1]
    return bar, children_of(host, bar)[2]
end

return {
    {
        "the lock engages when the host requests it",
        function()
            local host, id, lock = setup()
            H.check(not lock.engaged, "the lock must start disengaged")
            H.frame(host, 1, { { type = "requested", target = id, args = {} } })
            H.check(lock.engaged, "requested must engage the lock")
        end,
    },
    {
        "unlock sends the password to the native core",
        function()
            local host = setup()
            local lock = root:find(Lock)
            go(function()
                lock:unlock("hunter2")
            end)
            local call = host.calls[#host.calls]
            H.check(call.method == "authenticate", "unlock must call authenticate")
            H.check(call.args[1] == "hunter2", "the password must reach the native core")
        end,
    },
    {
        "unlock waits for the native answer and disengages on ok",
        function()
            local host, id, lock = setup()
            H.frame(host, 1, { { type = "requested", target = id, args = {} } })
            local result, err
            go(function()
                result, err = lock:unlock("pw")
            end)
            H.check(result == nil, "unlock must wait until the native core answers")
            H.frame(host, 2, { { type = last_token(host), target = id, args = { "ok", "" } } })
            H.check(result == "ok", "unlock must return the native answer, got " .. tostring(result))
            H.check(err == "", "an ok answer carries no error, got " .. tostring(err))
            H.check(not lock.engaged, "an ok answer must disengage the lock")
        end,
    },
    {
        "a wrong password keeps the lock engaged",
        function()
            local host, id, lock = setup()
            H.frame(host, 1, { { type = "requested", target = id, args = {} } })
            local result, err
            go(function()
                result, err = lock:unlock("nope")
            end)
            H.frame(host, 2, { { type = last_token(host), target = id, args = { "", "denied" } } })
            H.check(result == "" and err == "denied", "unlock must return the denial")
            H.check(lock.engaged, "a denial must keep the lock engaged")
        end,
    },
    {
        "a rate-limited attempt keeps the lock engaged and returns the wait",
        function()
            local host, id, lock = setup()
            H.frame(host, 1, { { type = "requested", target = id, args = {} } })
            local result, err
            go(function()
                result, err = lock:unlock("again")
            end)
            H.frame(host, 2, { { type = last_token(host), target = id, args = { "1500", "rate-limited" } } })
            H.check(result == "1500" and err == "rate-limited", "unlock must return the wait and the reason")
            H.check(lock.engaged, "a rate-limited attempt must keep the lock engaged")
        end,
    },
    {
        "the overview keeps every workspace at full strength while it is on",
        function()
            setup()
            local d = desk()
            H.check(d.active.opacity == 1, "the active workspace starts at full strength")
            d:toggle_overview()
            for _, ws in ipairs(d.space.children) do
                H.check(ws.opacity == 1, "every workspace must be shown in the overview")
            end
        end,
    },
    {
        "toggling the overview off dims the workspaces that are not active",
        function()
            setup()
            local d = desk()
            d:toggle_overview()
            d:toggle_overview()
            for _, ws in ipairs(d.space.children) do
                local want = ws == d.active and 1 or 0.4
                H.check(ws.opacity == want, ("workspace opacity %s, want %s"):format(ws.opacity, want))
            end
        end,
    },
    {
        "toast adds one box to the desk",
        function()
            setup()
            local d = desk()
            local before = #d.children
            toast("saved")
            H.check(#d.children == before + 1, "toast must add one box to the desk")
        end,
    },
    {
        "a window gets a titlebar when it is ready",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            local win = root:add(Window({ app_id = "foot", title = "term" }))
            H.load_shell()
            root:find(Seat).screen = screen
            H.start_shell(host)
            H.check(#win:all(Titlebar) == 1, "a ready window must get exactly one titlebar")
        end,
    },
    {
        "Super+Space opens the launcher and Escape closes it",
        function()
            sys = {
                spawn = function() end,
                date = function()
                    return ""
                end,
            }
            setup()
            seat.data.keys["Super+Space"]()
            H.check(#desk():all(Launcher) == 1, "Super+Space must open one launcher")
            desk():all(Launcher)[1].key:emit({ key = "Escape" })
            H.check(#desk():all(Launcher) == 0, "Escape must close the launcher")
        end,
    },
    {
        "Return in the launcher spawns the typed command and closes it",
        function()
            local spawned
            sys = {
                spawn = function(cmd)
                    spawned = cmd
                end,
                date = function()
                    return ""
                end,
            }
            setup()
            seat.data.keys["Super+Space"]()
            local l = desk():all(Launcher)[1]
            l.text = "foot"
            l.key:emit({ key = "Return" })
            H.check(spawned == "foot", "Return must spawn the typed command")
            H.check(#desk():all(Launcher) == 0, "Return must close the launcher")
        end,
    },
    {
        "Super+3 makes workspace 3 the active one",
        function()
            setup()
            seat.data.keys["Super+3"]()
            H.check(desk().active == desk().space.children[3], "Super+3 must show workspace 3")
        end,
    },
    {
        "a closing window fades out before it is destroyed",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            local win = root:add(Window({ app_id = "foot" }))
            H.load_shell()
            root:find(Seat).screen = screen
            H.start_shell(host)
            go(function()
                win:vanish()
            end)
            H.check(#host.ids("Window") == 1, "the window must still exist while it fades")
            H.frame(host, 1, {})
            H.frame(host, 2, {})
            H.check(#host.ids("Window") == 0, "the window must be destroyed after the fade")
        end,
    },
    {
        "the titlebar width follows the window width",
        function()
            local host, win = window_setup({ app_id = "foot", title = "term" })
            H.frame(host, 1, {})
            win.width = 420
            H.frame(host, 2, {})
            H.check(win:all(Titlebar)[1].width == 420, "the titlebar must be as wide as its window")
        end,
    },
    {
        "the titlebar fill follows whether the window is activated",
        function()
            local host, win = window_setup({ app_id = "foot" })
            H.frame(host, 1, {})
            seat.focus = win
            H.frame(host, 2, {})
            H.check(win:all(Titlebar)[1].fill == "#414559", "an activated window must have the active fill")
            seat.focus = nil
            H.frame(host, 3, {})
            H.check(win:all(Titlebar)[1].fill == "#232634", "an inactive window must have the idle fill")
        end,
    },
    {
        "the titlebar text follows the window title",
        function()
            local host, win = window_setup({ app_id = "foot", title = "term" })
            H.frame(host, 1, {})
            win.title = "vim"
            H.frame(host, 2, {})
            H.check(win:all(Titlebar)[1]:all(Text)[1].text == "vim", "the titlebar text must show the new title")
        end,
    },
    {
        "the titlebar close icon closes its window and stops the press",
        function()
            local host, win = window_setup({ app_id = "foot" })
            H.frame(host, 1, {})
            local _, icon = titlebar_ids(host)
            local dragged, real_drag = nil, drag
            drag = function(_, w)
                dragged = w
            end
            H.frame(host, 2, { { type = "press", bubbles = true, target = icon, args = {} } })
            drag = real_drag
            local closed = false
            for _, c in ipairs(host.calls) do
                closed = closed or (c.method == "close" and c.id == host.ids("Window")[1])
            end
            H.check(closed, "the close icon must close its window")
            H.check(dragged == nil, "a stopped press must not reach the titlebar drag")
        end,
    },
    {
        "a press on the titlebar starts a drag of its window",
        function()
            local host, win = window_setup({ app_id = "foot" })
            H.frame(host, 1, {})
            local bar = titlebar_ids(host)
            local dragged, real_drag = nil, drag
            drag = function(_, w)
                dragged = w
            end
            H.frame(
                host,
                2,
                { { type = "press", bubbles = true, target = bar, button = 272, x = 5, y = 5, args = {} } }
            )
            drag = real_drag
            H.check(dragged == win, "a press on the titlebar must drag its window")
        end,
    },
    {
        "a titlebar middle click lowers its window",
        function()
            local host, win = window_setup({ app_id = "foot" })
            H.frame(host, 1, {})
            local bar = titlebar_ids(host)
            local a = root:add(Window({ app_id = "other" }))
            H.frame(host, 2, {})
            H.check(win.parent:children()[#win.parent:children()] == a, "a starts above win")
            H.frame(
                host,
                3,
                { { type = "press", bubbles = true, target = bar, button = 274, x = 5, y = 5, args = {} } }
            )
            H.check(win.parent:children()[1] == win, "a titlebar middle click must lower the window to the bottom")
        end,
    },
    {
        "a titlebar right click runs the seat's right_click action",
        function()
            local host, win = window_setup({ app_id = "foot" })
            H.frame(host, 1, {})
            local bar = titlebar_ids(host)
            local a = root:add(Window({ app_id = "other" }))
            H.frame(host, 2, {})
            local seat_id = host.ids("Seat")[1]
            H.frame(
                host,
                3,
                { { type = "press", bubbles = true, target = bar, button = 273, x = 40, y = 9, args = {} } }
            )
            H.check(
                win.parent:children()[#win.parent:children()] == a,
                "right_click = menu is a no-op until the core has a menu"
            )
            host.external(seat_id, "right_click", "lower")
            H.frame(
                host,
                4,
                { { type = "press", bubbles = true, target = bar, button = 273, x = 40, y = 9, args = {} } }
            )
            H.check(win.parent:children()[1] == win, "right_click = lower must lower the window")
        end,
    },
    {
        "a titlebar double click toggles maximize, and a slow second click does not",
        function()
            local host, win = window_setup({ app_id = "foot" })
            H.frame(host, 1, {})
            local bar = titlebar_ids(host)
            H.frame(
                host,
                2,
                { { type = "press", bubbles = true, target = bar, button = 272, x = 5, y = 5, args = {} } }
            )
            H.frame(
                host,
                2.1,
                { { type = "press", bubbles = true, target = bar, button = 272, x = 5, y = 5, args = {} } }
            )
            H.check(win.maximised == true, "a double click must maximise the window")
            H.frame(
                host,
                5,
                { { type = "press", bubbles = true, target = bar, button = 272, x = 5, y = 5, args = {} } }
            )
            H.frame(
                host,
                6,
                { { type = "press", bubbles = true, target = bar, button = 272, x = 5, y = 5, args = {} } }
            )
            H.check(win.maximised == true, "two presses 4 s apart are not a double click")
        end,
    },
    {
        "the seat's double_click property picks the titlebar action",
        function()
            local host, win = window_setup({ app_id = "foot" })
            H.frame(host, 1, {})
            local bar = titlebar_ids(host)
            host.external(host.ids("Seat")[1], "double_click", "none")
            H.frame(
                host,
                2,
                { { type = "press", bubbles = true, target = bar, button = 272, x = 5, y = 5, args = {} } }
            )
            H.frame(
                host,
                2.1,
                { { type = "press", bubbles = true, target = bar, button = 272, x = 5, y = 5, args = {} } }
            )
            H.check(win.maximised ~= true, "double_click = none must leave the window alone")
        end,
    },
    {
        "an unknown titlebar action on the seat raises",
        function()
            local host, win = window_setup({ app_id = "foot" })
            H.frame(host, 1, {})
            local bar = titlebar_ids(host)
            host.external(host.ids("Seat")[1], "middle_click", "explode")
            local ok = pcall(
                H.frame,
                host,
                2,
                { { type = "press", bubbles = true, target = bar, button = 274, x = 5, y = 5, args = {} } }
            )
            H.check(not ok, "an unknown titlebar action must raise, not be ignored")
        end,
    },
    {
        "switching workspace slides the new one in and leaves only it visible",
        function()
            local host = setup()
            local d = desk()
            local old, ws3 = d.active, d.space.children[3]
            local ws3_id, old_id = host.ids("Workspace")[3], host.ids("Workspace")[1]
            seat.data.keys["Super+3"]()
            H.check(ws3.visible, "the new workspace must be shown as the slide starts")
            H.check(old.visible and old.x == -d.width, "the old workspace must slide out")
            H.frame(host, 1, {})
            H.frame(host, 2, {})
            local function tweened(id, value)
                for _, batch in ipairs(host.batches) do
                    for _, o in ipairs(batch) do
                        if o.op == "set" and o.id == id and o.key == "x" and o.value == value and o.seconds then
                            return true
                        end
                    end
                end
                return false
            end
            H.check(tweened(ws3_id, 0), "the new workspace must slide to 0 over time")
            H.check(tweened(old_id, -d.width), "the old workspace must slide out over time")
            H.check(ws3.x == 0, "the slide must end with the new workspace in place")
            H.check(not old.visible, "the old workspace must be hidden after the slide")
            H.check(ws3.visible, "the new workspace must be visible after the slide")
        end,
    },
}
