-- tests/lua/reposition_test.lua: moving and resizing a window with the keyboard
-- (data/lua/gnoblin/shell/reposition.lua), which the window menu's Move and Resize start.
local H = require("harness")

local function setup()
    local core, host = require("fake_core")()
    H.boot(core, host)
    root:add(Seat({}))
    root:add(Screen({ width = 1000, height = 800 }))
    root:add(Lock({}))
    local w = root:add(Window({ app_id = "foot", title = "win" }))
    H.load_shell()
    H.publish_seat(host, { screen = host.ids("Screen")[1] })
    H.start_shell(host)
    H.frame(host, 1, {})
    return host, w, require("gnoblin.shell.reposition")
end

local function key(node, name)
    node.key:emit({ key = name, text = "", pressed = true })
end

return {
    {
        "an arrow key moves the window by a step, Return keeps it and the keyboard goes back to the window",
        function()
            local _, w, R = setup()
            local x, y = w.x, w.y
            H.check(w:move_with_keys() == true, "a floating window can be moved")
            local grab = R.current() and desk():all(RepositionGrab)[1]
            H.check(grab ~= nil and seat.focus == grab, "the mode holds the keyboard")
            key(grab, "Right")
            key(grab, "Down")
            H.check(w.x == x + R.STEP and w.y == y + R.STEP, "the window moved one step on each axis")
            key(grab, "Return")
            H.check(R.current() == nil and #desk():all(RepositionGrab) == 0, "the mode is over")
            H.check(w.x == x + R.STEP, "the new place is kept")
            H.check(seat.focus == w, "the keyboard is the window's")
        end,
    },
    {
        "Escape puts the window back where it was",
        function()
            local _, w, R = setup()
            local x, y, width, height = w.x, w.y, w.width, w.height
            w:resize_with_keys()
            local grab = desk():all(RepositionGrab)[1]
            key(grab, "Left")
            key(grab, "Up")
            H.check(w.width == width - R.STEP and w.height == height - R.STEP, "Left and Up shrink")
            key(grab, "Escape")
            H.check(w.x == x and w.y == y and w.width == width and w.height == height, "Escape restores the rectangle")
        end,
    },
    {
        "a move stays inside the work area and a resize keeps a minimum size",
        function()
            local _, w, R = setup()
            w:move_with_keys()
            local grab = desk():all(RepositionGrab)[1]
            for _ = 1, 200 do
                key(grab, "Left")
            end
            H.check(w.x >= 0, "the window does not leave the left edge, got " .. w.x)
            key(grab, "Return")
            w:resize_with_keys()
            grab = desk():all(RepositionGrab)[1]
            for _ = 1, 200 do
                key(grab, "Left")
                key(grab, "Up")
            end
            H.check(w.width == R.MIN_WIDTH and w.height == R.MIN_HEIGHT, "the window keeps its least size")
        end,
    },
    {
        "a maximised, fullscreen or tiled window is refused",
        function()
            local _, w = setup()
            w:maximise()
            H.check(w:move_with_keys() == false, "a maximised window is not moved")
            w:unmaximise()
            w.tiled = true
            H.check(w:resize_with_keys() == false, "a tiled window is not resized")
            H.check(#desk():all(RepositionGrab) == 0, "and no mode started")
        end,
    },
    {
        "a snapped window is unsnapped to move, and Escape snaps it back",
        function()
            local _, w = setup()
            w.data.snapped = "left"
            w:move_with_keys()
            H.check(w.data.snapped == nil, "moving unsnaps")
            key(desk():all(RepositionGrab)[1], "Escape")
            H.check(w.data.snapped == "left", "Escape puts the snap back")
        end,
    },
    {
        "the mode ends when the window goes and when the session locks",
        function()
            local host, w, R = setup()
            w:move_with_keys()
            host.close(w.id)
            H.frame(host, 2, {})
            H.check(R.current() == nil and #desk():all(RepositionGrab) == 0, "the window going ends it")
            local w2 = root:add(Window({ app_id = "foot", title = "two" }))
            H.frame(host, 3, {})
            w2:move_with_keys()
            host.external(lock.id, "state", "pending")
            H.frame(host, 4, { { type = "requested", target = lock.id } })
            H.check(R.current() == nil, "the lock ends it")
        end,
    },
}
