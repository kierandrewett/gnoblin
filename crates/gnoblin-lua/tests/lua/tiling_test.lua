-- tests/lua/tiling_test.lua: the stock tiled layouts of data/lua/gnoblin/shell/tiling.lua (Tiling, Monocle, GridLayout,
-- Dwindle, Kiosk). Each class is a Workspace whose arrange() is a binding over its window children. Expected
-- rectangles are workspace-relative, integral, and use the defaults of the old layouts in data/lua/gnoblin/layouts.
local H = require("harness")

local CLASSES = { "Tiling", "Monocle", "GridLayout", "Dwindle", "Kiosk" }
local FULL = { 0, 0, 1000, 800 }

-- A fresh runtime with the stock shell and the tiling classes, and one workspace of `class` at 1000 by 800.
local function workspace(class, spec)
    local core, host = require("fake_core")()
    H.boot(core, host)
    root:add(Seat({}))
    root:add(Lock({}))
    local screen = root:add(Screen({ width = 1000, height = 800 }))
    H.load_shell()
    dofile(GNOBLIN_LUA_DIR .. "/shell/tiling.lua")
    root:find(Seat).screen = screen
    spec = spec or {}
    -- The desk builds its nine workspaces through Desk:workspace; every one is `class`, and desk().active is the first.
    function Desk:workspace(name)
        local ws = _G[class]({ name = name, width = 1000, height = 800 })
        for k, v in pairs(spec) do
            ws[k] = v
        end
        return ws
    end
    H.start_shell(host)
    HOST = host
    return desk().active
end

-- Adds `n` windows to `ws` and returns them in order.
local function add(ws, n)
    local out = {}
    for i = 1, n do
        out[i] = ws:add(Window({ app_id = "w" .. (#ws:all("> Window") + 1) }))
    end
    return out
end

-- The window rectangles of `ws` in child order, as { x, y, w, h } lists. A frame is pumped first: placement is
-- applied when the host frame runs, as in floating_test.
local frame_now = 0
local function rects(ws)
    frame_now = frame_now + 1
    HOST.frame(frame_now, {})
    local out = {}
    for i, w in ipairs(ws:all("> Window")) do
        out[i] = { w.x, w.y, w.width, w.height }
    end
    return out
end

local function check_rects(ws, want)
    local got = rects(ws)
    H.check(#got == #want, ("got %d windows, want %d"):format(#got, #want))
    for i, r in ipairs(want) do
        local g = got[i]
        H.check(
            g[1] == r[1] and g[2] == r[2] and g[3] == r[3] and g[4] == r[4],
            ("window %d: got %s, want %s"):format(i, table.concat(g, ","), table.concat(r, ","))
        )
    end
end

local function check_integral(ws)
    for _, w in ipairs(ws:all("> Window")) do
        for _, k in ipairs({ "x", "y", "width", "height" }) do
            local v = w[k]
            H.check(v == v and v > -math.huge and v < math.huge, k .. " must be finite")
            H.check(math.type(v) == "integer", k .. " must be integral, got " .. tostring(v))
        end
    end
end

local function shown_count(ws)
    local n = 0
    for _, w in ipairs(ws:all("> Window")) do
        if w.visible then
            n = n + 1
        end
    end
    return n
end

local cases = {}

local function case(name, fn)
    cases[#cases + 1] = { name, fn }
end

-- Every class: 0, 1, 2 and 3 windows, and 200 windows, give integral finite rectangles.
for _, class in ipairs(CLASSES) do
    case(class .. ": 0 windows give no rectangles", function()
        check_rects(workspace(class), {})
    end)

    case(class .. ": 1 window fills the workspace", function()
        local ws = workspace(class)
        add(ws, 1)
        check_rects(ws, { FULL })
        check_integral(ws)
    end)

    case(class .. ": 200 windows give finite integral rectangles", function()
        local ws = workspace(class)
        add(ws, 200)
        H.check(#ws:all("> Window") == 200, "all 200 windows are children")
        check_integral(ws)
    end)
end

case("Tiling: 2 and 3 windows give the master and stack rectangles", function()
    local ws = workspace("Tiling")
    add(ws, 2)
    check_rects(ws, { { 0, 0, 500, 800 }, { 500, 0, 500, 800 } })
    ws:add(Window({ app_id = "c" }))
    check_rects(ws, { { 0, 0, 500, 800 }, { 500, 0, 500, 400 }, { 500, 400, 500, 400 } })
end)

case("Tiling: 200 windows share the stack in whole pixels", function()
    local ws = workspace("Tiling")
    add(ws, 200)
    local got = rects(ws)
    H.check(got[1][1] == 0 and got[1][3] == 500 and got[1][4] == 800, "the master fills the left half")
    H.check(got[2][1] == 500 and got[2][2] == 0 and got[2][4] == 4, "the first stack window is 4 pixels high")
    H.check(got[200][2] == 795 and got[200][4] == 4, "the last stack window sits at 795")
end)

case("Tiling: the gap separates the master from the stack and the stack rows", function()
    local ws = workspace("Tiling", { gap = 10 })
    add(ws, 3)
    check_rects(ws, { { 0, 0, 495, 800 }, { 505, 0, 495, 395 }, { 505, 405, 495, 395 } })
end)

case("Tiling: orientation top puts the master across the top", function()
    local ws = workspace("Tiling", { orientation = "top" })
    add(ws, 2)
    check_rects(ws, { { 0, 0, 1000, 400 }, { 0, 400, 1000, 400 } })
end)

case("Tiling: ratio, gap, width and height re-arrange with no handler", function()
    local ws = workspace("Tiling", { ratio = 0.25, gap = 12 })
    add(ws, 2)
    check_rects(ws, { { 0, 0, 247, 800 }, { 259, 0, 741, 800 } })
    ws.width = 500
    check_rects(ws, { { 0, 0, 122, 800 }, { 134, 0, 366, 800 } })
    ws.height = 400
    check_rects(ws, { { 0, 0, 122, 400 }, { 134, 0, 366, 400 } })
    ws.gap = 0
    check_rects(ws, { { 0, 0, 125, 400 }, { 125, 0, 375, 400 } })
end)

case("Tiling: adding, removing and reordering windows re-arranges", function()
    local ws = workspace("Tiling")
    local a, b = table.unpack(add(ws, 2))
    local c = ws:add(Window({ app_id = "c" }))
    check_rects(ws, { { 0, 0, 500, 800 }, { 500, 0, 500, 400 }, { 500, 400, 500, 400 } })
    c:destroy()
    check_rects(ws, { { 0, 0, 500, 800 }, { 500, 0, 500, 800 } })
    ws:add(b, 1)
    H.check(ws:all("> Window")[1] == b, "b moved to the front")
    rects(ws)
    H.check(b.x == 0 and b.width == 500 and a.x == 500, "the master is now b")
    check_rects(ws, { { 0, 0, 500, 800 }, { 500, 0, 500, 800 } })
end)

case("GridLayout: 1, 2 and 3 windows give the expected cells", function()
    local ws = workspace("GridLayout")
    add(ws, 2)
    check_rects(ws, { { 0, 0, 500, 800 }, { 500, 0, 500, 800 } })
    ws:add(Window({ app_id = "c" }))
    check_rects(ws, { { 0, 0, 500, 400 }, { 500, 0, 500, 400 }, { 0, 400, 500, 400 } })
end)

case("GridLayout: the gap separates cells and a partly filled last row is left aligned", function()
    local ws = workspace("GridLayout", { gap = 10 })
    add(ws, 3)
    check_rects(ws, { { 0, 0, 495, 395 }, { 505, 0, 495, 395 }, { 0, 405, 495, 395 } })
end)

case("GridLayout: fill column goes top to bottom then right", function()
    local ws = workspace("GridLayout", { fill = "column" })
    add(ws, 3)
    check_rects(ws, { { 0, 0, 500, 400 }, { 0, 400, 500, 400 }, { 500, 0, 500, 400 } })
end)

case("GridLayout: gap changes re-arrange with no handler", function()
    local ws = workspace("GridLayout")
    add(ws, 2)
    ws.gap = 20
    check_rects(ws, { { 0, 0, 490, 800 }, { 510, 0, 490, 800 } })
end)

case("Dwindle: 2 and 3 windows split the last cell by aspect", function()
    local ws = workspace("Dwindle")
    add(ws, 2)
    check_rects(ws, { { 0, 0, 500, 800 }, { 500, 0, 500, 800 } })
    ws:add(Window({ app_id = "c" }))
    check_rects(ws, { { 0, 0, 500, 800 }, { 500, 0, 500, 400 }, { 500, 400, 500, 400 } })
end)

case("Dwindle: split horizontal puts every window side by side", function()
    local ws = workspace("Dwindle", { split = "horizontal" })
    add(ws, 3)
    check_rects(ws, { { 0, 0, 500, 800 }, { 500, 0, 250, 800 }, { 750, 0, 250, 800 } })
end)

case("Dwindle: ratio changes re-arrange with no handler", function()
    local ws = workspace("Dwindle")
    add(ws, 2)
    ws.ratio = 0.25
    check_rects(ws, { { 0, 0, 250, 800 }, { 250, 0, 750, 800 } })
end)

case("Monocle: the focused window fills the workspace and the rest are hidden", function()
    local ws = workspace("Monocle")
    add(ws, 1)
    check_rects(ws, { FULL })
    add(ws, 2)
    check_rects(ws, { FULL, FULL, FULL })
    H.check(shown_count(ws) == 1, "exactly one window is shown")
    seat.focus = ws:all("> Window")[2]
    rects(ws)
    local wins = ws:all("> Window")
    H.check(wins[2].visible and not wins[1].visible and not wins[3].visible, "the focused window is the one shown")
end)

case("Monocle: refocusing shows another window and hides the rest", function()
    local ws = workspace("Monocle")
    local a, b = table.unpack(add(ws, 2))
    seat.focus = a
    rects(ws)
    H.check(a.visible and not b.visible, "a is shown")
    seat.focus = b
    rects(ws)
    H.check(b.visible and not a.visible, "b is shown after the focus moves")
end)

case("Kiosk: 1, 2 and 3 windows show one full-area window and hide the rest", function()
    local ws = workspace("Kiosk")
    add(ws, 1)
    check_rects(ws, { FULL })
    add(ws, 2)
    check_rects(ws, { FULL, FULL, FULL })
    H.check(shown_count(ws) == 1, "exactly one window is shown")
end)

case("Tiling: windows outside the workspace's children are untouched", function()
    local ws = workspace("Tiling")
    local other = root:add(Window({ app_id = "other" }))
    other.x, other.y, other.width, other.height = 7, 9, 33, 44
    add(ws, 3)
    H.check(other.x == 7 and other.y == 9 and other.width == 33 and other.height == 44, "a window outside is untouched")
end)

case("GridLayout: a window with visible false is still placed, as the old layouts did", function()
    local ws = workspace("GridLayout")
    local _, b = table.unpack(add(ws, 2))
    b.visible = false
    check_rects(ws, { { 0, 0, 500, 800 }, { 500, 0, 500, 800 } })
end)

case("each class: the same sequence replayed twice gives identical rectangles", function()
    local function run(class)
        local ws = workspace(class)
        local list = add(ws, 3)
        if class ~= "Monocle" and class ~= "Kiosk" then
            ws.gap = 4
        end
        list[2]:destroy()
        add(ws, 2)
        local out = {}
        for i, r in ipairs(rects(ws)) do
            out[i] = table.concat(r, ",")
        end
        return table.concat(out, ";")
    end
    for _, class in ipairs(CLASSES) do
        H.check(run(class) == run(class), class .. " must be deterministic")
    end
end)

case("each class: the options have the old layouts' defaults", function()
    local t = workspace("Tiling")
    H.check(t.ratio == 0.5 and t.gap == 0 and t.master_count == 1 and t.orientation == "left", "Tiling defaults")
    local g = workspace("GridLayout")
    H.check(g.fill == "row", "GridLayout fill default is row")
    local d = workspace("Dwindle")
    H.check(d.split == "aspect" and d.ratio == 0.5 and d.gap == 0, "Dwindle defaults")
end)

return cases
