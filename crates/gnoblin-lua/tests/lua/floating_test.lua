-- tests/lua/floating_test.lua: the stock placement of data/lua/gnoblin/shell/floating.lua. Failure modes 3 to 6 and
-- 8 to 12 of design section 8.2 (1, 2 and 7 are in shell_test.lua). The golden table is checked from Rust
-- (tests/floating.rs), which calls the three classes; these cases cover what the table does not.
local H = require("harness")

local WORK = { x = 0, y = 0, w = 1920, h = 1080 }

-- The pure placement function, from a fresh runtime with the stock shell loaded.
local function place_fn()
    H.fresh()
    H.load_shell()
    return H.floating
end

-- A shell over a 1000 by 800 screen with its one Desk ready. Returns the core, host.
local function shell()
    local core, host = require("fake_core")()
    H.boot(core, host)
    root:add(Seat({}))
    root:add(Lock({}))
    local screen = root:add(Screen({ width = 1000, height = 800 }))
    H.load_shell()
    root:find(Seat).screen = screen
    H.start_shell(host)
    return core, host
end

local function overlaps(a, b)
    return a.x < b.x + b.w and b.x < a.x + a.w and a.y < b.y + b.h and b.y < a.y + a.h
end

local function inside(r, area)
    return r.x >= area.x and r.y >= area.y and r.x + r.w <= area.x + area.w and r.y + r.h <= area.y + area.h
end

return {
    {
        "an unknown placement mode is an error, not a silent default",
        function()
            local place = place_fn().place
            H.check(not pcall(place, "spiral", 400, 300, WORK, {}), "an unknown mode must raise")
        end,
    },

    {
        "a size below one pixel on an axis is raised to one (failure mode 6)",
        function()
            local place = place_fn().place
            local r = place("free_space", 0, -4, WORK, {})
            H.check(r.w == 1 and r.h == 1, ("want 1x1, got %dx%d"):format(r.w, r.h))
        end,
    },

    {
        "a window larger than the work area aligns to the area's origin (failure mode 5)",
        function()
            local place = place_fn().place
            local area = { x = 64, y = 32, w = 1920, h = 1080 }
            local r = place("free_space", 2560, 1440, area, {})
            H.check(r.x == 64 and r.y == 32, ("want 64,32, got %d,%d"):format(r.x, r.y))
            H.check(r.w == 2560 and r.h == 1440, "an oversized window keeps its size")
        end,
    },

    {
        "a full work area still gives the window a rectangle inside it (failure mode 4)",
        function()
            local place = place_fn().place
            for _, mode in ipairs({ "free_space", "center", "cascade" }) do
                local r = place(mode, 400, 300, WORK, { WORK })
                H.check(inside(r, WORK), mode .. " must stay inside the area")
            end
        end,
    },

    {
        "the same inputs give the same rectangle (failure mode 11)",
        function()
            local place = place_fn().place
            local occupied = { { x = 0, y = 0, w = 400, h = 300 }, { x = 400, y = 0, w = 400, h = 300 } }
            for _, mode in ipairs({ "free_space", "center", "cascade" }) do
                local a = place(mode, 400, 300, WORK, occupied)
                local b = place(mode, 400, 300, WORK, occupied)
                H.check(a.x == b.x and a.y == b.y and a.w == b.w and a.h == b.h, mode .. " must be deterministic")
            end
        end,
    },

    {
        "every coordinate is an integer, including for odd sizes and offset areas (failure mode 12)",
        function()
            local place = place_fn().place
            local areas = { WORK, { x = 3, y = -7, w = 1001, h = 777 }, { x = 0, y = 0, w = 1, h = 1 } }
            local sizes = { { 1, 1 }, { 399, 301 }, { 2560, 1440 }, { 0, 0 } }
            for _, mode in ipairs({ "free_space", "center", "cascade" }) do
                for _, area in ipairs(areas) do
                    for _, s in ipairs(sizes) do
                        local r = place(mode, s[1], s[2], area, { { x = area.x, y = area.y, w = 50, h = 50 } })
                        for _, k in ipairs({ "x", "y", "w", "h" }) do
                            H.check(math.type(r[k]) == "integer", mode .. " " .. k .. " must be an integer")
                        end
                    end
                end
            end
        end,
    },

    {
        "each mode is a class method that ignores self, so Cascade.place(nil, ...) is the cascade rule",
        function()
            local fn = place_fn()
            local occupied = { { x = 0, y = 0, w = 400, h = 300 } }
            local want = fn.place("cascade", 400, 300, WORK, occupied)
            local got = Cascade.place(nil, 400, 300, WORK, occupied)
            H.check(got.x == want.x and got.y == want.y, "Cascade.place must match the cascade mode")
            H.check(Centred.place(nil, 400, 300, WORK, {}).x == 760, "Centred.place must centre on the area")
            H.check(Floating.place(nil, 400, 300, WORK, {}).x == 0, "Floating.place must scan from the origin")
        end,
    },

    {
        "two windows that map in one batch do not overlap: the second sees the first (failure mode 8)",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            root:add(Lock({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            local a = root:add(Window({ app_id = "a" }))
            local b = root:add(Window({ app_id = "b" }))
            for _, id in ipairs(host.ids("Window")) do
                host.external(id, "width", 400)
                host.external(id, "height", 300)
            end
            H.load_shell()
            root:find(Seat).screen = screen
            H.start_shell(host)
            local ws = desk().active
            H.check(a.parent == ws and b.parent == ws, "both windows must be in the active workspace")
            H.check(
                not overlaps(
                    { x = a.x, y = a.y, w = a.width, h = a.height },
                    { x = b.x, y = b.y, w = b.width, h = b.height }
                ),
                ("the second window overlaps the first: %d,%d and %d,%d"):format(a.x, a.y, b.x, b.y)
            )
        end,
    },

    {
        "a panel shrinking the workspace does not move the windows already placed (failure mode 9)",
        function()
            local _, host = shell()
            local ws = desk().active
            local win = ws:add(Window({ app_id = "a" }))
            local x, y = win.x, win.y
            ws.width, ws.height = 300, 200
            host.frame(1, {})
            H.check(win.x == x and win.y == y, "the window must not move when the workspace shrinks")
        end,
    },

    {
        "a client resize after placement does not place the window again (failure mode 10)",
        function()
            local _, host = shell()
            local ws = desk().active
            local win = ws:add(Window({ app_id = "a" }))
            local x, y = win.x, win.y
            local id = host.ids("Window")[#host.ids("Window")]
            host.external(id, "width", 700)
            host.external(id, "height", 500)
            host.frame(1, {})
            H.check(win.data.placed, "the window stays placed")
            H.check(win.x == x and win.y == y, "a resize must not move the window")
        end,
    },

    {
        "a window is placed in the same call that adds it, never at the origin first (failure mode 3)",
        function()
            shell()
            local ws = desk().active
            ws:add(Window({ app_id = "a" }))
            local second = ws:add(Window({ app_id = "b" }))
            H.check(second.data.placed, "the window must be placed as soon as it joins the workspace")
            H.check(second.x ~= 0 or second.y ~= 0, "the second window must avoid the origin the first took")
        end,
    },

    {
        "a window that has not committed a size opens at the default 800 by 600",
        function()
            shell()
            local win = desk().active:add(Window({ app_id = "a" }))
            H.check(win.width == 800 and win.height == 600, ("want 800x600, got %dx%d"):format(win.width, win.height))
        end,
    },

    {
        "a window committed before the shell starts keeps its committed size",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            root:add(Lock({}))
            local screen = root:add(Screen({ width = 1000, height = 800 }))
            root:add(Window({ app_id = "a" }))
            local id = host.ids("Window")[1]
            host.external(id, "width", 500)
            host.external(id, "height", 400)
            H.load_shell()
            root:find(Seat).screen = screen
            H.start_shell(host)
            local win = desk().active.children[1]
            H.check(win.width == 500 and win.height == 400, ("want 500x400, got %dx%d"):format(win.width, win.height))
        end,
    },

    {
        "the same map sequence replayed in a fresh runtime gives the same mirror (failure mode 11)",
        function()
            local function run()
                local _, host = shell()
                local ws = desk().active
                ws:add(Window({ app_id = "a" }))
                host.frame(1, {})
                ws:add(Window({ app_id = "b" }))
                host.frame(2, {})
                return host.dump()
            end
            local first = run()
            local second = run()
            H.check(first == second, "the two runs must produce the same mirror")
        end,
    },
}
