-- tests/lua/drag_test.lua: the rules of a drag in data/lua/gnoblin/shell/drag.lua. Pure: the rectangle a dragged
-- window may take under the seat's constraint, and what a drag does to a window in its current state.
local H = require("harness")

local function drag()
    return dofile(GNOBLIN_LUA_DIR .. "/shell/drag.lua")
end

local AREA = { x = 0, y = 32, w = 1920, h = 1048 }

local function at(x, y, w, h)
    return { x = x, y = y, w = w, h = h }
end

local function is(r, x, y, w, h)
    return r.x == x and r.y == y and r.w == w and r.h == h
end

return {
    {
        "the constraint is work_area unless minimum_strip is named, and any other value raises",
        function()
            local d = drag()
            H.check(d.mode(nil) == "work_area", "absent is work_area")
            H.check(d.mode("work_area") == "work_area", "work_area")
            H.check(d.mode("minimum_strip") == "minimum_strip", "minimum_strip")
            H.check(not pcall(d.mode, "loose"), "an unknown name must raise")
            H.check(not pcall(d.mode, true), "a non-string must raise")
        end,
    },

    {
        "under work_area a dragged window is pulled back inside the work area",
        function()
            local d = drag()
            local m = d.mode(nil)
            H.check(is(d.constrain(at(-400, -400, 600, 400), AREA, m), 0, 32, 600, 400), "off the top-left")
            H.check(is(d.constrain(at(1800, 1000, 600, 400), AREA, m), 1320, 680, 600, 400), "off the bottom-right")
            H.check(is(d.constrain(at(100, 200, 600, 400), AREA, m), 100, 200, 600, 400), "inside is not moved")
        end,
    },

    {
        "a window larger than the work area is pinned to its origin",
        function()
            local d = drag()
            H.check(
                is(d.constrain(at(500, 500, 2000, 1200), AREA, "work_area"), 0, 32, 2000, 1200),
                "an oversized window sits at the area's origin"
            )
        end,
    },

    {
        "under minimum_strip a window may leave the area but 75 pixels of it stay inside",
        function()
            local d = drag()
            H.check(
                is(d.constrain(at(-5000, -5000, 600, 400), AREA, "minimum_strip"), -525, -293, 600, 400),
                "a strip of 75 pixels remains on screen at the top-left"
            )
            H.check(
                is(d.constrain(at(5000, 5000, 600, 400), AREA, "minimum_strip"), 1845, 1005, 600, 400),
                "and at the bottom-right"
            )
            H.check(
                is(d.constrain(at(100, 200, 600, 400), AREA, "minimum_strip"), 100, 200, 600, 400),
                "a window fully inside is not moved"
            )
        end,
    },

    {
        "a fullscreen window refuses a drag, and a maximised window floats first",
        function()
            local d = drag()
            H.check(d.start({ fullscreen = true, maximised = false }) == "refuse", "fullscreen refuses")
            H.check(d.start({ fullscreen = true, maximised = true }) == "refuse", "fullscreen wins over maximised")
            H.check(d.start({ fullscreen = false, maximised = true }) == "float", "maximised floats")
            H.check(d.start({ fullscreen = false, maximised = false }) == "move", "a floating window moves")
        end,
    },
}
