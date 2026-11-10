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
            local result
            go(function()
                result = lock:unlock("pw")
            end)
            H.check(result == nil, "unlock must wait until the native core answers")
            H.frame(host, 2, { { type = last_token(host), target = id, args = { "ok" } } })
            H.check(result == "ok", "unlock must return the native answer, got " .. tostring(result))
            H.check(not lock.engaged, "an ok answer must disengage the lock")
        end,
    },
    {
        "a wrong password keeps the lock engaged",
        function()
            local host, id, lock = setup()
            H.frame(host, 1, { { type = "requested", target = id, args = {} } })
            local result
            go(function()
                result = lock:unlock("nope")
            end)
            H.frame(host, 2, { { type = last_token(host), target = id, args = { "denied" } } })
            H.check(result == "denied", "unlock must return the denial")
            H.check(lock.engaged, "a denial must keep the lock engaged")
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
}
