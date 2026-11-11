-- tests/lua/snap_test.lua: edge tiling of data/lua/gnoblin/shell/snap.lua. Pure: a pointer point and a work area in,
-- a zone and a rectangle out. Dragging a window to an edge tiles it only when edge_tiling is on (default off).
local H = require("harness")

local function snap()
    return dofile(GNOBLIN_LUA_DIR .. "/shell/snap.lua")
end

local AREA = { x = 0, y = 32, w = 1920, h = 1048 }

-- A 1000 by 800 shell with one window on its Desk, placed by the stock floating placement. edge_tiling sets the seat.
local function shell_window(props, edge_tiling)
    local core, host = require("fake_core")()
    H.boot(core, host)
    local seat = root:add(Seat({}))
    local screen = root:add(Screen({ width = 1000, height = 800 }))
    local win = root:add(Window(props))
    H.load_shell()
    H.publish_seat(host, { screen = host.ids("Screen")[1], edge_tiling = edge_tiling })
    H.start_shell(host)
    H.frame(host, 1, {})
    return host, win
end

local function rect(w)
    return { x = w.x, y = w.y, w = w.width, h = w.height }
end

local function same(a, b)
    return a.x == b.x and a.y == b.y and a.w == b.w and a.h == b.h
end

return {
    {
        "edge tiling is off by default",
        function()
            H.check(snap().enabled({}) == false, "edge_tiling defaults to false")
            H.check(snap().enabled({ edge_tiling = true }) == true, "edge_tiling = true enables it")
        end,
    },

    {
        "a non-boolean edge_tiling raises",
        function()
            H.check(not pcall(snap().enabled, { edge_tiling = "yes" }), "a string must raise")
        end,
    },

    {
        "a pointer within the margin of an edge names that edge",
        function()
            local s = snap()
            H.check(s.edge_zone(2, 500, AREA, 4) == "left", "left edge")
            H.check(s.edge_zone(1917, 500, AREA, 4) == "right", "right edge")
            H.check(s.edge_zone(900, 33, AREA, 4) == "top", "top edge")
        end,
    },

    {
        "a pointer away from every edge names no zone",
        function()
            H.check(snap().edge_zone(960, 500, AREA, 4) == nil, "the middle of the area has no zone")
        end,
    },

    {
        "left and right tile to the matching half, top fills the area",
        function()
            local s = snap()
            local l = s.tile_rect("left", AREA)
            H.check(l.x == 0 and l.y == 32 and l.w == 960 and l.h == 1048, "left half")
            local r = s.tile_rect("right", AREA)
            H.check(r.x == 960 and r.w == 960 and r.h == 1048, "right half")
            local t = s.tile_rect("top", AREA)
            H.check(t.x == 0 and t.y == 32 and t.w == 1920 and t.h == 1048, "top fills the work area")
        end,
    },

    {
        "an unknown zone has no rectangle",
        function()
            H.check(snap().tile_rect("bottom", AREA) == nil, "bottom does not tile")
        end,
    },

    {
        "snapping tiles a window to an edge and unsnapping restores its floating rectangle",
        function()
            local _, win = shell_window({ app_id = "foot" }, true)
            local floating = rect(win)
            H.check(win:snap("left"), "a snap with edge tiling on must be taken")
            local a = work_area()
            H.check(same(rect(win), { x = 0, y = a.y, w = a.w // 2, h = a.h }), "left tiles to the left half")
            H.check(win.data.snapped == "left", "the snapped zone must be kept")
            H.check(win:unsnap(), "unsnap must restore the window")
            H.check(same(rect(win), floating), "unsnap restores the floating rectangle")
            H.check(not win:unsnap(), "a second unsnap does nothing")
        end,
    },

    {
        "snapping a maximised window floats it first, and unsnapping restores the floating rectangle",
        function()
            local _, win = shell_window({ app_id = "foot" }, true)
            local floating = rect(win)
            H.check(win:maximise(), "maximise must be taken")
            H.check(win:snap("left"), "snap must be taken")
            H.check(win.maximised == false, "snapping must float a maximised window")
            H.check(win.data.snapped == "left", "the snapped zone must be kept")
            win:unsnap()
            H.check(same(rect(win), floating), "unsnap restores the rectangle the window had before it was maximised")
        end,
    },

    {
        "snapping is refused when edge tiling is off and leaves the window where it was",
        function()
            local _, win = shell_window({ app_id = "foot" }, false)
            local before = rect(win)
            H.check(win:snap("left") == false, "a snap with edge tiling off must be refused")
            H.check(win.data.snapped == nil, "a refused snap must not mark the window snapped")
            H.check(same(rect(win), before), "a refused snap must not move the window")
        end,
    },

    {
        "a client maximise request fills the work area, and is refused while the window is fullscreen",
        function()
            local host, win = shell_window({ app_id = "foot" }, true)
            local floating = rect(win)
            H.frame(
                host,
                2,
                { { type = "request", bubbles = true, target = host.ids("Window")[1], request = "maximize" } }
            )
            H.check(win.maximised == true, "a maximise request must be granted")
            local a = work_area()
            H.check(same(rect(win), { x = 0, y = a.y, w = a.w, h = a.h }), "maximised fills the work area")
            H.frame(
                host,
                3,
                { { type = "request", bubbles = true, target = host.ids("Window")[1], request = "unmaximize" } }
            )
            H.check(same(rect(win), floating), "unmaximise restores the floating rectangle")
            win.fullscreen = true
            H.frame(
                host,
                4,
                { { type = "request", bubbles = true, target = host.ids("Window")[1], request = "maximize" } }
            )
            H.check(win.maximised == false, "a maximise request must be refused while fullscreen")
        end,
    },

    {
        "leaving fullscreen returns the floating rectangle, and a maximised window stays maximised through fullscreen",
        function()
            local _, win = shell_window({ app_id = "foot" }, true)
            local floating = rect(win)
            win.fullscreen = true
            win.fullscreen = false
            H.check(same(rect(win), floating), "leaving fullscreen must return the floating rectangle")
            H.check(win:maximise(), "maximise must be taken")
            local maximised = rect(win)
            win.fullscreen = true
            H.check(win.maximised == true, "fullscreen must not clear maximise")
            win.fullscreen = false
            H.check(same(rect(win), maximised), "leaving fullscreen must keep the maximised rectangle")
            win:unmaximise()
            H.check(same(rect(win), floating), "unmaximise must return the floating rectangle")
        end,
    },
}
