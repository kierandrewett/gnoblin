-- tests/lua/alerts_test.lua: the stock "not responding" alert (data/lua/gnoblin/shell/alerts.lua) over the fake core.
-- The compositor marks a window `unresponsive`; the alert looks once a second and offers Wait and Force quit.
local H = require("harness")

local clock = 1

local function setup(n)
    local core, host = require("fake_core")()
    H.boot(core, host)
    root:add(Seat({}))
    root:add(Screen({ width = 1000, height = 800 }))
    root:add(Lock({}))
    local wins = {}
    for i = 1, n or 1 do
        wins[i] = root:add(Window({ app_id = "foot", title = "win" .. i }))
    end
    H.load_shell()
    H.publish_seat(host, { screen = host.ids("Screen")[1] })
    H.start_shell(host)
    clock = 1
    H.frame(host, clock, {})
    return host, wins
end

-- Runs frames, one second apart by default, so each look at the windows happens.
local function seconds(host, n, step)
    for _ = 1, n do
        clock = clock + (step or 1)
        H.frame(host, clock, {})
    end
end

local function stuck(host, w, value)
    host.external(w.id, "unresponsive", value)
    seconds(host, 2)
end

local function card()
    return desk():all(NotRespondingDialog)[1]
end

local function key(node, name)
    node.key:emit({ key = name, text = "", pressed = true })
end

local function kills(host, w)
    local n = 0
    for _, c in ipairs(host.calls) do
        if c.method == "kill" and c.id == w.id then
            n = n + 1
        end
    end
    return n
end

return {
    {
        "a responsive window gets no alert",
        function()
            local host = setup()
            seconds(host, 3)
            H.check(card() == nil, "no alert")
        end,
    },
    {
        "an unresponsive window gets an alert that names it, holds the keyboard, and defaults to Wait",
        function()
            local host, wins = setup()
            local before = seat.focus
            stuck(host, wins[1], true)
            local c = card()
            H.check(c ~= nil, "an alert opens")
            local found = false
            for _, t in ipairs(c:all(Text)) do
                found = found or t.text == "win1 is not responding"
            end
            H.check(found, "it names the window")
            H.check(seat.focus == c, "the keys go to it")
            H.check(c.secure == false, "an ordinary window's alert is not secure")
            H.check(c.index == 1, "Wait is the default")
            key(c, "Escape")
            H.check(card() == nil and kills(host, wins[1]) == 0, "Escape waits and kills nothing")
            H.check(seat.focus == before, "the keyboard goes back")
        end,
    },
    {
        "the alert for a secure window is secure",
        function()
            local host, wins = setup()
            wins[1].secure = true
            stuck(host, wins[1], true)
            H.check(card().secure == true, "it shows the title, so it is secure")
        end,
    },
    {
        "Wait keeps the alert away until the window answers and stops answering again",
        function()
            local host, wins = setup()
            stuck(host, wins[1], true)
            key(card(), "Escape")
            seconds(host, 3)
            H.check(card() == nil, "no second alert while it stays unresponsive")
            stuck(host, wins[1], false)
            stuck(host, wins[1], true)
            H.check(card() ~= nil, "an alert again once it went and came back")
        end,
    },
    {
        "Force quit does nothing until the alert is armed, then kills the window's process and closes",
        function()
            local host, wins = setup()
            host.external(wins[1].id, "unresponsive", true)
            seconds(host, 1)
            local c = card()
            key(c, "Tab")
            key(c, "Return")
            H.check(card() == c and kills(host, wins[1]) == 0, "Force quit ignores an early press")
            seconds(host, 2, 0.6)
            c:select(2)
            key(c, "Return")
            H.check(kills(host, wins[1]) == 1, "an armed Force quit calls kill once")
            H.check(card() == nil, "and the alert is gone")
            seconds(host, 3)
            H.check(card() == nil, "and does not come back")
        end,
    },
    {
        "a press on a button works like the key, and a press on the card keeps the keyboard on it",
        function()
            local host, wins = setup()
            stuck(host, wins[1], true)
            local c = card()
            seat.focus = nil
            H.frame(
                host,
                clock + 0.1,
                { { type = "press", bubbles = true, target = c.id, button = 272, x = 5, y = 5 } }
            )
            H.check(seat.focus == c, "a press on the card focuses it")
            seconds(host, 2, 0.6)
            H.frame(host, clock + 0.1, {
                { type = "press", bubbles = true, target = c.choices[2].id, button = 272, x = 5, y = 5 },
            })
            H.check(kills(host, wins[1]) == 1 and card() == nil, "a click on Force quit kills")
        end,
    },
    {
        "the alert closes when the window answers again or goes",
        function()
            local host, wins = setup(2)
            stuck(host, wins[1], true)
            H.check(card() ~= nil, "open")
            stuck(host, wins[1], false)
            H.check(card() == nil, "closed when it answers")
            stuck(host, wins[2], true)
            H.check(card() ~= nil, "open for the other")
            host.close(wins[2].id)
            seconds(host, 2)
            H.check(card() == nil, "closed when the window goes")
        end,
    },
    {
        "no alert opens while the session is locked, and one opens once it is unlocked",
        function()
            local host, wins = setup()
            lock.engaged = true
            stuck(host, wins[1], true)
            H.check(card() == nil, "nothing over the lock screen")
            lock.engaged = false
            seconds(host, 2)
            H.check(card() ~= nil, "the alert comes once it is unlocked")
            lock.engaged = true
            seconds(host, 2)
            H.check(card() == nil, "and goes when the session locks")
        end,
    },
    {
        "seat.data.alerts = false turns the alert off",
        function()
            local host, wins = setup()
            seat.data.alerts = false
            stuck(host, wins[1], true)
            H.check(card() == nil, "no alert")
        end,
    },
    {
        "the alert closes an open window menu, and a window title with control characters is shown as text",
        function()
            local host, wins = setup()
            local Menus = require("gnoblin.shell.menus")
            Menus.open(wins[1], 10, 10)
            H.check(Menus.current() ~= nil, "a menu is open")
            host.external(wins[1].id, "title", "a\nb")
            stuck(host, wins[1], true)
            H.check(Menus.current() == nil, "the alert closed the menu")
            local found = false
            for _, t in ipairs(card():all(Text)) do
                found = found or t.text == "a b is not responding"
            end
            H.check(found, "control characters are replaced")
        end,
    },
}
