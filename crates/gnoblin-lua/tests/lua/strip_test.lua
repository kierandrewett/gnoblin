-- tests/lua/strip_test.lua: the Strip and Carousel3d layouts of data/lua/gnoblin/shell/strip.lua over the fake core.
-- Not yet loaded by the compositor: these cases run the stock library directly, as the other shell cases do.
local H = require("harness")

-- A shell over a 1000 by 800 screen whose workspaces are `class`, with `n` windows that map before the shell starts,
-- so they are placed in order and the last one is focused. Returns host, the active workspace and the windows.
local function shell_with(class, n, extra)
    local core, host = require("fake_core")()
    H.boot(core, host)
    root:add(Seat({}))
    root:add(Lock({}))
    local screen = root:add(Screen({ width = 1000, height = 800 }))
    local wins = {}
    for i = 1, n do
        wins[i] = root:add(Window({ app_id = "w" .. i }))
    end
    H.load_shell()
    root:find(Seat).screen = screen
    class = _G[class] -- the stock library defines the layouts when it loads
    function Desk:workspace(name)
        local spec = { name = name }
        if class == Strip then
            spec.column = extra and extra.column or 0.5
        end
        return class(spec)
    end
    H.start_shell(host)
    H.check(#host.reports == 0, "the shell reported an error: " .. table.concat(host.reports, " | "))
    local ws = desk().active
    H.check(ws:is(class), "the active workspace must be the chosen layout")
    ws:arrange()
    return host, ws, wins
end

-- The app ids of the windows in the workspace, in order.
local function order(ws)
    local names = {}
    for _, w in ipairs(ws:all("> Window")) do
        names[#names + 1] = w.app_id
    end
    return names
end

return {
    {
        "a column fraction clamps to 0.1..1 and a non-number or NaN is an error",
        function()
            local column = H.strip.column
            H.check(column(0.5) == 0.5, "0.5 stays")
            H.check(column(0) == 0.1, "0 clamps up to 0.1")
            H.check(column(-math.huge) == 0.1, "-inf clamps up to 0.1")
            H.check(column(5) == 1, "5 clamps down to 1")
            H.check(column(math.huge) == 1, "inf clamps down to 1")
            H.check(not pcall(column, "wide"), "a string is an error")
            H.check(not pcall(column, nil), "nil is an error")
            H.check(not pcall(column, 0 / 0), "NaN is an error")
        end,
    },

    {
        "a strip lays out one column per window and scrolls to keep the focused one in view",
        function()
            local _, strip, w = shell_with("Strip", 3)
            H.check(seat.focus == w[3], "the last window opened is focused")
            H.check(w[1].width == 500 and w[1].height == 800, "a column is half the strip wide, the full height")
            -- Column 3 spans 1000..1500, off the 1000-wide view: it is centred (view 750), then clamped to the
            -- 3 * 500 - 1000 = 500 the strip can scroll.
            H.check(w[3].x == 500, "column 3 must sit at the right edge, got " .. tostring(w[3].x))
            H.check(w[2].x == 0 and w[1].x == -500, "columns 2 and 1 sit left of column 3")
            H.check(w[1].y == 0, "columns sit at the top with no margin")
        end,
    },

    {
        "a new window opens after the focused column, not at the end",
        function()
            local _, strip, w = shell_with("Strip", 3)
            seat.focus = w[1]
            root:add(Window({ app_id = "new" }))
            H.same(order(strip), { "w1", "new", "w2", "w3" })
        end,
    },

    {
        "focus_left and focus_right move through the columns and stop at the ends",
        function()
            local _, strip, w = shell_with("Strip", 3)
            strip:focus_left()
            H.check(seat.focus == w[2], "focus_left goes to column 2")
            strip:focus_left()
            strip:focus_left()
            H.check(seat.focus == w[1], "focus_left stops at column 1")
            strip:focus_right()
            H.check(seat.focus == w[2], "focus_right goes to column 2")
            strip:focus_right()
            strip:focus_right()
            H.check(seat.focus == w[3], "focus_right stops at column 3")
        end,
    },

    {
        "scroll_to pins the view until the focused window changes",
        function()
            local _, strip, w = shell_with("Strip", 3)
            strip:scroll_to(0)
            H.check(w[1].x == 0, "scroll_to(0) puts column 1 at the left edge")
            -- Focus is unchanged, so the pin holds even though column 3 is off view.
            strip:arrange()
            H.check(w[1].x == 0, "a pin holds while the focus stays on the same window")
            -- Moving focus to column 2 (already in view) clears the pin without a follow.
            strip:focus_left()
            strip:arrange()
            H.check(w[1].x == 0, "a focus change inside the view does not scroll")
            -- Moving focus to column 3 (off view) follows again: centred, clamped to 500.
            strip:focus_right()
            strip:arrange()
            H.check(w[3].x == 500, "a focus change off view scrolls the focused column in")
        end,
    },

    {
        "scroll_to rejects a non-finite offset",
        function()
            local _, strip = shell_with("Strip", 1)
            H.check(not pcall(strip.scroll_to, strip, 0 / 0), "NaN is an error")
            H.check(not pcall(strip.scroll_to, strip, math.huge), "inf is an error")
            H.check(not pcall(strip.scroll_to, strip, "left"), "a string is an error")
        end,
    },

    {
        "closing the focused column moves focus to its neighbour",
        function()
            local host, strip, w = shell_with("Strip", 3)
            seat.focus = w[2]
            H.frame(host, 1, { { type = "closed", target = host.ids("Window")[2] } })
            H.check(
                seat.focus == w[3] or seat.focus == w[1],
                "focus must move to a neighbour, not stay on the closed one"
            )
            H.check(seat.focus ~= w[2], "the closed column must not keep focus")
        end,
    },

    {
        "the ring puts the focused card at the front, unscaled and fully opaque",
        function()
            local area = { x = 0, y = 0, width = 1000, height = 800 }
            local s = H.strip.ring(4, 1, area, {})[1]
            H.check(s.rotation == 0, "the front card has no rotation")
            H.check(math.abs(s.opacity - 1) < 1e-9, "the front card is fully opaque")
            H.check(math.abs(s.width - 1000 * 0.36) < 1e-6, "the front card is the card width, unscaled")
            H.check(math.abs(s.x + s.width / 2 - 500) < 1e-6, "the front card is centred")
        end,
    },

    {
        "the ring is symmetric about the front card and fades toward the back",
        function()
            local area = { x = 0, y = 0, width = 1000, height = 800 }
            local slots = H.strip.ring(4, 1, area, {})
            -- With focus at 1 of 4: slot 2 is the right neighbour, slot 4 the left, slot 3 the back.
            local left, right, back = slots[4], slots[2], slots[3]
            H.check(math.abs(left.rotation + right.rotation) < 1e-9, "neighbours rotate in opposite directions")
            H.check(
                math.abs((left.x + left.width / 2) + (right.x + right.width / 2) - 1000) < 1e-6,
                "mirror about centre"
            )
            H.check(math.abs(left.opacity - right.opacity) < 1e-9, "neighbours are equally faded")
            H.check(back.opacity < left.opacity, "the back card is fainter than a side card")
            H.check(back.width < right.width, "the back card is smaller than a side card")
            H.check(math.abs(math.abs(back.rotation) - 180) < 1e-9, "the back card is turned half way round")
        end,
    },

    {
        "the ring with no windows is empty, and a bad focus index is an error",
        function()
            local area = { x = 0, y = 0, width = 1000, height = 800 }
            H.check(#H.strip.ring(0, 1, area, {}) == 0, "no windows, no slots")
            H.check(not pcall(H.strip.ring, 3, 0, area, {}), "focus 0 is an error")
            H.check(not pcall(H.strip.ring, 3, 4, area, {}), "focus past the end is an error")
            H.check(not pcall(H.strip.ring, 3, 1.5, area, {}), "a fractional focus is an error")
        end,
    },

    {
        "a Carousel3d shows the focused window at the front and rotate wraps round the ring",
        function()
            local _, car, w = shell_with("Carousel3d", 2)
            H.check(seat.focus == w[2], "the last window opened is focused")
            H.check(math.abs(w[2].opacity - 1) < 1e-9, "the front card is opaque")
            car:rotate(1)
            H.check(seat.focus == w[1], "rotate(1) wraps from the last card to the first")
            car:rotate(-1)
            H.check(seat.focus == w[2], "rotate(-1) goes back")
            H.check(not pcall(car.rotate, car, "one"), "a non-number step is an error")
        end,
    },
}
