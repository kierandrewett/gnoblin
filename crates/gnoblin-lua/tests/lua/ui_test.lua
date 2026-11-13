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
    H.publish_seat(host, { screen = host.ids("Screen")[1] })
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
    H.publish_seat(host, { screen = host.ids("Screen")[1] })
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
            host.external(id, "state", "pending")
            H.frame(host, 1, { { type = "requested", target = id, args = {} } })
            H.check(lock.engaged, "requested must engage the lock")
        end,
    },
    {
        "a lock requested before the shell has a desk shows its screen once the desk comes up",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            local lock = root:add(Lock({}))
            root:add(Screen({ width = 1000, height = 800 }))
            H.load_shell()
            H.start_shell(host) -- the screen is in the tree, but the seat does not name it yet: there is no desk
            local id = host.ids("Lock")[1]
            host.external(id, "state", "pending") -- the compositor takes the lock, then sends `requested` once
            H.frame(host, 1, { { type = "requested", target = id, args = {} } })
            H.check(not lock.engaged, "no lock screen can be up before the desk exists")
            H.publish_seat(host, { screen = host.ids("Screen")[1] })
            H.frame(host, 2, {})
            H.frame(host, 3, {})
            H.check(#root:all(Entry) == 1, "the request must wait for the desk, got " .. #root:all(Entry) .. " entries")
            H.check(lock.engaged, "the lock must be engaged once its screen is up")
        end,
    },
    {
        "a shell that attaches while the session is locked shows the lock screen",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            local lock = root:add(Lock({}))
            root:add(Screen({ width = 1000, height = 800 }))
            host.external(host.ids("Lock")[1], "state", "locked") -- taken before this shell attached: `requested` went elsewhere
            H.load_shell()
            H.publish_seat(host, { screen = host.ids("Screen")[1] })
            H.start_shell(host)
            H.frame(host, 1, {})
            H.frame(host, 2, {})
            H.check(
                #root:all(Entry) == 1,
                "a locked session must show its lock screen on attach, got " .. #root:all(Entry)
            )
            H.check(lock.engaged, "the attached shell must engage the lock")
        end,
    },
    {
        "a lock that unlocks before the desk comes up leaves no screen behind",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            local lock = root:add(Lock({}))
            root:add(Screen({ width = 1000, height = 800 }))
            H.load_shell()
            H.start_shell(host)
            local id = host.ids("Lock")[1]
            host.external(id, "state", "pending")
            H.frame(host, 1, { { type = "requested", target = id, args = {} } })
            host.external(id, "state", "not-locked") -- unlocked before there was a desk to lock
            H.publish_seat(host, { screen = host.ids("Screen")[1] })
            H.frame(host, 2, {})
            H.frame(host, 3, {})
            H.check(#root:all(Entry) == 0, "an unlocked session must not get a lock screen")
            H.check(not lock.engaged, "an unlocked session must not be engaged")
        end,
    },
    {
        "an external unlock takes the screen down, and a later request draws it again",
        function()
            local host, id, lock = setup()
            host.external(id, "state", "pending")
            H.frame(host, 1, { { type = "requested", target = id, args = {} } })
            H.check(#root:all(Entry) == 1 and lock.engaged, "the request must draw the screen")
            host.external(id, "state", "not-locked") -- unlocked through the protocol while the screen is up
            H.frame(host, 2, {})
            H.check(#root:all(Entry) == 0, "an unlock must take the screen down")
            H.check(not lock.engaged, "an unlock must disengage the lock")
            host.external(id, "state", "pending")
            H.frame(host, 3, { { type = "requested", target = id, args = {} } })
            H.check(#root:all(Entry) == 1 and lock.engaged, "a later request must draw the screen again")
        end,
    },
    {
        "Enter on the lock screen sends the typed password to the native core",
        function()
            local host, id = setup()
            host.external(id, "state", "pending")
            H.frame(host, 1, { { type = "requested", target = id, args = {} } })
            local entry = root:all(Entry)[1]
            H.check(entry ~= nil, "the request must draw the screen")
            entry.text = "hunter2"
            entry.key:emit({ key = "Return", text = "", pressed = true })
            H.frame(host, 2, {})
            local call -- the frame's measures of the entry's text come after the authenticate it sent
            for _, c in ipairs(host.calls) do
                if c.method == "authenticate" then
                    call = c
                end
            end
            H.check(call ~= nil, "Enter must call authenticate")
            H.check(
                call.text == "hunter2",
                "the typed password must reach the native core, got " .. tostring(call.text)
            )
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
            H.check(call.text == "hunter2", "the password must reach the native core as the call's secret text")
            H.check(call.args[1] == nil, "the password is never an argument")
        end,
    },
    {
        "unlock waits for the native answer and disengages on ok",
        function()
            local host, id, lock = setup()
            host.external(id, "state", "pending")
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
            host.external(id, "state", "pending")
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
            host.external(id, "state", "pending")
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
            H.publish_seat(host, { screen = host.ids("Screen")[1] })
            H.start_shell(host)
            H.check(#win:all(Titlebar) == 1, "a ready window must get exactly one titlebar")
        end,
    },
    {
        "every chord the stock shell binds is in the compositor's angle-bracket form",
        function()
            setup()
            local count = 0
            for chord in pairs(seat.data.keys) do
                count = count + 1
                -- The compositor refuses `Super+Space` and logs `a seat claim's chord does not parse`, so the binding
                -- never fires: a modifier is `<Super>`, and the key is an XKB keysym name (`space`, `Left`, `3`).
                local key = chord
                while key:match("^<%a+>") do
                    key = key:gsub("^<%a+>", "", 1)
                end
                H.check(key ~= "" and key:match("^[%w_]+$") ~= nil, "not a chord the compositor parses: " .. chord)
            end
            H.check(count >= 12, "the stock shell binds Super+Left and Right, Super+Space and Super+1 to 9: " .. count)
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
            seat.data.keys["<Super>space"]()
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
            seat.data.keys["<Super>space"]()
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
            seat.data.keys["<Super>3"]()
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
            H.publish_seat(host, { screen = host.ids("Screen")[1] })
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
            H.check(win.parent.children[#win.parent.children] == a, "a starts above win")
            H.frame(
                host,
                3,
                { { type = "press", bubbles = true, target = bar, button = 274, x = 5, y = 5, args = {} } }
            )
            H.check(win.parent.children[1] == win, "a titlebar middle click must lower the window to the bottom")
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
                win.parent.children[#win.parent.children] == a,
                "right_click = menu is a no-op until the core has a menu"
            )
            host.external(seat_id, "right_click", "lower")
            H.frame(
                host,
                4,
                { { type = "press", bubbles = true, target = bar, button = 273, x = 40, y = 9, args = {} } }
            )
            H.check(win.parent.children[1] == win, "right_click = lower must lower the window")
        end,
    },
    {
        "a titlebar double click toggles maximize",
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
        end,
    },
    {
        "a slow second click is not a double click",
        function()
            local host, win = window_setup({ app_id = "foot" })
            H.frame(host, 1, {})
            local bar = titlebar_ids(host)
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
            H.check(win.maximised ~= true, "two presses 1 s apart are not a double click")
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
        "an unknown titlebar action on the seat is reported",
        function()
            local host, win = window_setup({ app_id = "foot" })
            H.frame(host, 1, {})
            local bar = titlebar_ids(host)
            host.external(host.ids("Seat")[1], "middle_click", "explode")
            H.frame(
                host,
                2,
                { { type = "press", bubbles = true, target = bar, button = 274, x = 5, y = 5, args = {} } }
            )
            H.check(#host.reports == 1, "an unknown titlebar action must be reported, got " .. #host.reports)
        end,
    },
    {
        "switching workspace slides the new one in and leaves only it visible",
        function()
            local host = setup()
            local d = desk()
            local old, ws3 = d.active, d.space.children[3]
            local ws3_id, old_id = host.ids("Workspace")[3], host.ids("Workspace")[1]
            seat.data.keys["<Super>3"]()
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
    {
        "motion(seconds) is the duration, or 0 while the seat asks for reduced motion",
        function()
            local host = setup()
            H.check(motion(0.25) == 0.25, "the seat has not asked for reduced motion")
            H.publish_seat(host, { reduced_motion = true })
            H.frame(host, 1, {})
            H.check(motion(0.25) == 0, "reduced motion shortens every duration to 0")
            H.publish_seat(host, { reduced_motion = false })
            H.frame(host, 2, {})
            H.check(motion(0.25) == 0.25, "the next call follows the seat when it changes back")
        end,
    },
    {
        "with reduced motion on, switching workspace writes no tween and still ends in place",
        function()
            local host = setup()
            H.publish_seat(host, { reduced_motion = true })
            H.frame(host, 1, {})
            local d = desk()
            local old, ws3 = d.active, d.space.children[3]
            seat.data.keys["<Super>3"]()
            H.frame(host, 2, {})
            H.frame(host, 3, {})
            H.check(#host.timed_sets() == 0, "no write may carry a duration, got " .. #host.timed_sets())
            H.check(ws3.x == 0, "the new workspace must be in place")
            H.check(ws3.visible and d.active == ws3, "the new workspace must be shown")
            H.check(not old.visible, "the old workspace must be hidden after the switch")
        end,
    },
    {
        "with reduced motion on, a closing window is destroyed without a fade",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            local win = root:add(Window({ app_id = "foot" }))
            H.load_shell()
            H.publish_seat(host, { screen = host.ids("Screen")[1], reduced_motion = true })
            H.start_shell(host)
            go(function()
                win:vanish()
            end)
            H.frame(host, 1, {})
            H.check(#host.ids("Window") == 0, "the window must be destroyed by the next frame")
            H.check(#host.timed_sets() == 0, "the fade must carry no duration")
        end,
    },
    {
        "with reduced motion on, a toast appears at once",
        function()
            local host = setup()
            H.publish_seat(host, { reduced_motion = true })
            H.frame(host, 1, {})
            toast("saved")
            H.frame(host, 2, {})
            H.check(#host.timed_sets() == 0, "no write may carry a duration")
            local shown = false
            for _, batch in ipairs(host.batches) do
                for _, o in ipairs(batch) do
                    shown = shown or (o.op == "set" and o.key == "opacity" and o.value == 1)
                end
            end
            H.check(shown, "the toast must be written fully opaque")
        end,
    },
    {
        "a Text takes the width the compositor answers a measure with",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            H.load_shell()
            local t = root:add(Text({ text = "Hello", size = 10 }))
            H.check(t.width == 25, "before the answer the width is the estimate, got " .. tostring(t.width))
            H.check(t.height == 15 and t.ascent == 11, "so are the height and the ascent")
            local call = host.calls[#host.calls]
            H.check(call and call.method == "measure", "a new Text must ask the compositor to measure it")
            H.check(call.text == "sans\nHello", "the call carries the font and the text, got " .. tostring(call.text))
            H.check(call.args[1] == string.pack("<d", 10), "the call carries the size as a double")
            H.frame(
                host,
                1,
                { { type = tostring(call.result), target = host.ids("Canvas")[1], args = { "31 15 12", "" } } }
            )
            H.check(t.width == 31, "the answered width must replace the estimate, got " .. tostring(t.width))
            H.check(t.height == 15 and t.ascent == 12, "the answered height and ascent land too")
        end,
    },
    {
        "changing the text falls back to the estimate and asks again",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            H.load_shell()
            local t = root:add(Text({ text = "Hello", size = 10 }))
            local id = host.ids("Canvas")[1]
            local first = host.calls[#host.calls]
            H.frame(host, 1, { { type = tostring(first.result), target = id, args = { "31 15 12", "" } } })
            H.check(t.width == 31, "the first answer must land")
            t.text = "Hello, world"
            H.check(t.width == 60, "while the new answer is out the width is the estimate, got " .. tostring(t.width))
            local second = host.calls[#host.calls]
            H.check(second ~= first and second.method == "measure", "new text must ask again")
            -- The old question's answer arrives late: it must not be taken for the new text.
            H.frame(host, 2, { { type = tostring(first.result), target = id, args = { "99 15 12", "" } } })
            H.check(t.width == 60, "a stale answer must be ignored, got " .. tostring(t.width))
            H.frame(host, 3, { { type = tostring(second.result), target = id, args = { "74 15 12", "" } } })
            H.check(t.width == 74, "the new answer must land, got " .. tostring(t.width))
        end,
    },
    {
        "a refused measure keeps the estimate and is reported",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            H.load_shell()
            local t = root:add(Text({ text = "Hi", size = 10 }))
            local call = host.calls[#host.calls]
            H.frame(
                host,
                1,
                { { type = tostring(call.result), target = host.ids("Canvas")[1], args = { "", "unavailable" } } }
            )
            H.check(t.width == 10, "a refusal keeps the estimate, got " .. tostring(t.width))
            local reported = false
            for _, r in ipairs(host.reports) do
                reported = reported or r:find("unavailable", 1, true) ~= nil
            end
            H.check(reported, "the refusal must be reported")
            local calls = #host.calls
            H.frame(host, 2, {})
            H.check(#host.calls == calls, "a refusal must not be asked again for the same text")
        end,
    },
    {
        "a multi-line Text is as tall as its lines and as wide as its widest",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            H.load_shell()
            local t = root:add(Text({ text = "a\nbbbb\ncc", size = 10 }))
            H.check(t.width == 20, "the widest line sets the width, got " .. tostring(t.width))
            H.check(t.height == 45, "three line boxes of fifteen, got " .. tostring(t.height))
        end,
    },
    {
        "a Text that ellipsizes ends in an ellipsis when its row gives it less room than it measured",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            H.load_shell()
            local function newest()
                local ids = host.ids("Canvas")
                return core.get(ids[#ids], "content")
            end
            local roomy = root:add(Text({ text = "A long window title", size = 10, ellipsize = true }))
            H.check(newest():find("A long window title", 1, true), "with room enough the whole text is painted")
            root:add(Text({ text = "A long window title", size = 10, ellipsize = true, width = 50 }))
            local content = newest()
            H.check(content:find("A long wi\u{2026}", 1, true), "a text over its room is cut and ends in an ellipsis")
            H.check(not content:find("A long win", 1, true), "none of the cut text is painted")
            local plain = root:add(Text({ text = "A long window title", size = 10, width = 50 }))
            H.check(
                newest():find("A long window title", 1, true),
                "a text that does not ellipsize paints all of it, and the node's size clips it"
            )
            H.check(roomy.ascent > 0 and plain.ascent > 0, "every text knows its ascent")
        end,
    },
    {
        "a Box paints its fill into its content",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            H.load_shell()
            local b = root:add(Box({ width = 10, height = 5, fill = "#ff0000", radius = 2 }))
            local expected = "GNPL"
                .. string.pack("<BI4", 1, 1)
                .. string.pack("<Bfffff", 1, 0, 0, 10, 5, 2)
                .. string.pack("<I4I4f", 0xff0000ff, 0, 0)
            H.check(core.get(host.ids("Canvas")[1], "content") == expected, "the content is the encoded rect")
            b.fill = "#00ff0080"
            H.frame(host, 1, {})
            local changed = "GNPL"
                .. string.pack("<BI4", 1, 1)
                .. string.pack("<Bfffff", 1, 0, 0, 10, 5, 2)
                .. string.pack("<I4I4f", 0x00ff0080, 0, 0)
            H.check(core.get(host.ids("Canvas")[1], "content") == changed, "a new fill must be painted again")
        end,
    },
    {
        "a Text and an Icon paint themselves with their colours",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            H.load_shell()
            root:add(Text({ text = "ab", size = 13, color = "#c6d0f5", font = "mono" }))
            root:add(Icon({ icon = "window-close-symbolic", size = 16, color = "#fff" }))
            local ids = host.ids("Canvas")
            local text = "GNPL"
                .. string.pack("<BI4", 1, 1)
                .. string.pack("<BfffI4s2s4", 3, 0, 0, 13, 0xc6d0f5ff, "mono", "ab")
            local icon = "GNPL"
                .. string.pack("<BI4", 1, 1)
                .. string.pack("<BfffI4s2", 4, 0, 0, 16, 0xffffffff, "window-close-symbolic")
            H.check(core.get(ids[1], "content") == text, "the text op must carry the font, size and colour")
            H.check(core.get(ids[2], "content") == icon, "the icon op must carry the name, size and colour")
        end,
    },
    {
        "an op the encoder cannot write is reported and leaves the content alone",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            H.load_shell()
            local c = root:add(Canvas({ draw = { { rect = { 0, 0, 4, 4 }, fill = "red" } } }))
            H.check(core.get(host.ids("Canvas")[1], "content") == nil, "nothing is written for a bad op")
            local reported = false
            for _, r in ipairs(host.reports) do
                reported = reported or r:find("draw op 1", 1, true) ~= nil
            end
            H.check(reported, "the bad op must be named in a report, got " .. table.concat(host.reports, "; "))
            c.draw = {
                { rect = { 0, 0, 4, 4 }, fill = "#000" },
                { clip = { 0, 0, 2, 2 } },
                { clip = false },
                { opacity = 0.5 },
                { opacity = false },
            }
            H.frame(host, 1, {})
            H.check(core.get(host.ids("Canvas")[1], "content") ~= nil, "a good list is written once it is fixed")
        end,
    },
    {
        "a Text squeezed shorter than its line is centred in its node",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            H.load_shell()
            root:add(Text({ text = "ab", size = 13, color = "#fff", height = 13 }))
            local expected = "GNPL"
                .. string.pack("<BI4", 1, 1)
                .. string.pack("<BfffI4s2s4", 3, 0, -3, 13, 0xffffffff, "sans", "ab")
            H.check(core.get(host.ids("Canvas")[1], "content") == expected, "the line box is centred: y is -3")
        end,
    },
    {
        "a window has one titlebar even when the compositor's ready event follows the creation of its node",
        function()
            local host, win = window_setup({ app_id = "foot" })
            H.frame(host, 1, {})
            H.check(#win:all(Titlebar) == 1, "one titlebar after creation")
            H.frame(host, 2, { { type = "ready", target = host.ids("Window")[1], args = {} } })
            H.check(
                #win:all(Titlebar) == 1,
                "a node is ready once, whatever tells the shell, got " .. #win:all(Titlebar)
            )
        end,
    },
}
