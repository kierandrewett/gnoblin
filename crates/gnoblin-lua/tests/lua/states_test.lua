-- tests/lua/states_test.lua: what the stock shell does when a client or a taskbar asks for a window state change
-- (data/lua/gnoblin/shell/init.lua, `Window:on_request`). The shell decides and answers by writing the window's
-- properties; the compositor turns those into the client's configure and runs `close`.
local H = require("harness")

-- A started shell with one Screen (1000 by 800), one Seat and `n` windows. Returns the host and the windows, in
-- creation order, which is the order of `host.ids("Window")`.
local function setup(n)
    local core, host = require("fake_core")()
    H.boot(core, host)
    root:add(Seat({}))
    root:add(Screen({ width = 1000, height = 800 }))
    local wins = {}
    for i = 1, n do
        wins[i] = root:add(Window({ app_id = "foot" }))
    end
    H.load_shell()
    H.publish_seat(host, { screen = host.ids("Screen")[1] })
    H.start_shell(host)
    H.frame(host, 1, {})
    return host, wins
end

-- Delivers a client's request for window number `i`.
local function request(host, now, i, name)
    H.frame(host, now, { { type = "request", bubbles = true, target = host.ids("Window")[i], request = name } })
end

local function rect(w)
    return { x = w.x, y = w.y, w = w.width, h = w.height }
end

local function same(a, b)
    return a.x == b.x and a.y == b.y and a.w == b.w and a.h == b.h
end

local function is_last(n)
    local c = n.parent.children
    return c[#c] == n
end

local function focus()
    return root:find(Seat).focus
end

return {
    {
        "a fullscreen request covers the whole output and the window goes last in its workspace",
        function()
            local host, wins = setup(2)
            request(host, 2, 1, "fullscreen")
            H.check(wins[1].fullscreen == true, "a fullscreen request must be granted")
            H.check(
                same(rect(wins[1]), { x = 0, y = 0, w = 1000, h = 800 }),
                "fullscreen covers the output, got " .. wins[1].width .. "x" .. wins[1].height
            )
            H.check(is_last(wins[1]), "the fullscreen window must be last in tree order")
            wins[2]:raise()
            H.check(is_last(wins[1]), "a raised sibling must not cover a fullscreen window")
        end,
    },

    {
        "an unfullscreen request returns the rectangle the window had",
        function()
            local host, wins = setup(1)
            local before = rect(wins[1])
            request(host, 2, 1, "fullscreen")
            request(host, 3, 1, "unfullscreen")
            H.check(wins[1].fullscreen == false, "an unfullscreen request must be granted")
            H.check(same(rect(wins[1]), before), "leaving fullscreen returns the rectangle the window had")
        end,
    },

    {
        "a fullscreen window covers the bars, which come back when it leaves",
        function()
            local host, wins = setup(1)
            local bar = desk():add(Bar({ edge = "top", height = 32 }))
            H.frame(host, 2, {})
            H.check(desk().space.y == 32, "the bar must reserve the top")
            request(host, 3, 1, "fullscreen")
            H.check(same(rect(wins[1]), { x = 0, y = -32, w = 1000, h = 800 }), "the window starts at the screen's top")
            H.check(bar.visible == false, "the bar must give way to a fullscreen window")
            H.check(desk().space.clip == false, "the workspace must not clip a fullscreen window")
            request(host, 4, 1, "unfullscreen")
            H.check(bar.visible == true, "the bar must come back")
            H.check(desk().space.clip == true, "the workspace clips again")
        end,
    },

    {
        "a fullscreen window keeps covering the output when a bar reserves space",
        function()
            local host, wins = setup(1)
            request(host, 2, 1, "fullscreen")
            desk():add(Bar({ edge = "top", height = 32 }))
            H.frame(host, 3, {})
            H.check(
                same(rect(wins[1]), { x = 0, y = -32, w = 1000, h = 800 }),
                "the fullscreen window must still cover the whole output"
            )
        end,
    },

    {
        "fullscreen over a maximised window keeps it maximised, and leaving returns the maximised rectangle",
        function()
            local host, wins = setup(1)
            local floating = rect(wins[1])
            request(host, 2, 1, "maximize")
            local maximised = rect(wins[1])
            request(host, 3, 1, "fullscreen")
            H.check(wins[1].maximised == true, "fullscreen must not clear maximise")
            request(host, 4, 1, "unfullscreen")
            H.check(same(rect(wins[1]), maximised), "leaving fullscreen returns the maximised rectangle")
            request(host, 5, 1, "unmaximize")
            H.check(same(rect(wins[1]), floating), "unmaximise returns the floating rectangle")
        end,
    },

    {
        "closing a fullscreen window brings the bars back",
        function()
            local host, wins = setup(1)
            local bar = desk():add(Bar({ edge = "top", height = 32 }))
            H.frame(host, 2, {})
            request(host, 3, 1, "fullscreen")
            H.check(bar.visible == false, "the bar gives way")
            host.frame(4, { { type = "closed", target = host.ids("Window")[1], args = {} } })
            H.check(bar.visible == true, "the bar must come back when the fullscreen window closes")
        end,
    },

    {
        "a minimise request hides the window and hands focus to the topmost window left",
        function()
            local host, wins = setup(2)
            root:find(Seat).focus = wins[1]
            request(host, 2, 1, "minimize")
            H.check(wins[1].minimised == true, "a minimise request must be granted")
            H.check(wins[1].visible == false, "a minimised window is hidden")
            H.check(focus() == wins[2], "focus must pass to the window that stays")
        end,
    },

    {
        "minimising the last window leaves the seat with no focus, and minimising an unfocused window leaves it alone",
        function()
            local host, wins = setup(2)
            root:find(Seat).focus = wins[2]
            request(host, 2, 1, "minimize")
            H.check(focus() == wins[2], "minimising another window must not move focus")
            request(host, 3, 2, "minimize")
            H.check(focus() == nil, "no window is left to take focus")
        end,
    },

    {
        "an unminimise request shows the window, and focuses it only when nothing holds focus",
        function()
            local host, wins = setup(2)
            root:find(Seat).focus = wins[2]
            request(host, 2, 1, "minimize")
            request(host, 3, 1, "unminimize")
            H.check(wins[1].minimised == false and wins[1].visible == true, "the window must be shown again")
            H.check(focus() == wins[2], "focus stays where it was")
            request(host, 4, 2, "minimize")
            request(host, 5, 1, "minimize")
            H.check(focus() == nil, "both windows are hidden")
            request(host, 6, 2, "unminimize")
            H.check(focus() == wins[2], "a restored window takes focus when nothing holds it")
        end,
    },

    {
        "an activate request brings a minimised window back, focused and on top",
        function()
            local host, wins = setup(2)
            request(host, 2, 1, "minimize")
            request(host, 3, 1, "activate")
            H.check(
                wins[1].minimised == false and wins[1].visible == true,
                ("activating shows the window: minimised %s, visible %s, reports %s"):format(
                    tostring(wins[1].minimised),
                    tostring(wins[1].visible),
                    table.concat(host.reports, " | ")
                )
            )
            H.check(focus() == wins[1], "the window must be focused")
            H.check(is_last(wins[1]), "the window must be raised")
        end,
    },

    {
        "a close request calls the window's close, which the compositor turns into the client's close",
        function()
            local host, wins = setup(2)
            local id = host.ids("Window")[2]
            request(host, 2, 2, "close")
            local closes = 0
            for _, call in ipairs(host.calls) do
                if call.method == "close" then
                    closes = closes + 1
                    H.check(call.id == id, "the call must name the window that was asked to close")
                end
            end
            H.check(closes == 1, "one close call, got " .. closes)
            H.check(wins[2].parent ~= nil, "the window stays until the client destroys it")
        end,
    },

    {
        "raise and lower requests restack the window among its siblings",
        function()
            local host, wins = setup(2)
            H.check(is_last(wins[2]), "the second window starts on top")
            request(host, 2, 1, "raise")
            H.check(is_last(wins[1]), "a raise request puts the window on top")
            request(host, 3, 1, "lower")
            H.check(wins[1].parent.children[1] == wins[1], "a lower request puts the window at the bottom")
        end,
    },
}
