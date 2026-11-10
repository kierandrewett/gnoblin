-- tests/lua/snap_test.lua: edge tiling of data/lua/gnoblin/shell/snap.lua. Pure: a pointer point and a work area in,
-- a zone and a rectangle out. Dragging a window to an edge tiles it only when edge_tiling is on (default off).
local H = require("harness")

local function snap()
    return dofile(GNOBLIN_LUA_DIR .. "/shell/snap.lua")
end

local AREA = { x = 0, y = 32, w = 1920, h = 1048 }

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
}
