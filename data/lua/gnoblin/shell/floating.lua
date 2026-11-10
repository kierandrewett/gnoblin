-- data/lua/gnoblin/shell/floating.lua: free-space, cascade and centred placement of new windows.
-- Replaces the removed Rust engine (crates/gnoblin-wm/src/placement.rs, last at commit 370564286). Loaded by
-- shell/init.lua after node.lua. `M.place` is the pure function; the three classes call it. The golden cases
-- (tests/shell/placement_cases.toml) and tests/lua/floating_test.lua test both.
local M = {}

local SCAN_STEP, CASCADE_STEP = 16, 32
local MAX_SCAN_STEPS, MAX_CASCADE_STEPS = 20000, 32
local DEFAULT_W, DEFAULT_H = 800, 600

local function rect(x, y, w, h)
    return { x = x, y = y, w = w, h = h }
end

-- Two rectangles overlap; an empty one overlaps nothing.
local function intersects(a, b)
    return a.w > 0
        and a.h > 0
        and b.w > 0
        and b.h > 0
        and a.x < b.x + b.w
        and b.x < a.x + a.w
        and a.y < b.y + b.h
        and b.y < a.y + a.h
end

local function hits(r, occupied)
    for _, o in ipairs(occupied) do
        if intersects(r, o) then
            return true
        end
    end
    return false
end

-- Moves `r` the least distance inside `area`. A rectangle at least as large as the area goes to its origin.
local function clamp_inside(r, area)
    local x = r.w >= area.w and area.x or math.min(math.max(r.x, area.x), area.x + area.w - r.w)
    local y = r.h >= area.h and area.y or math.min(math.max(r.y, area.y), area.y + area.h - r.h)
    return rect(x, y, r.w, r.h)
end

-- Steps diagonally from `origin` until the window overlaps nothing, or the bound is hit.
local function cascade(origin, area, occupied)
    local c = clamp_inside(origin, area)
    for step = 1, MAX_CASCADE_STEPS do
        if not hits(c, occupied) then
            break
        end
        local d = CASCADE_STEP * step
        c = clamp_inside(rect(origin.x + d, origin.y + d, origin.w, origin.h), area)
    end
    return c
end

-- The first grid spot in the area that overlaps nothing; cascades from the top-left when none fits.
local function free_space(w, h, area, occupied)
    local steps, y = 0, area.y
    while y + h <= area.y + area.h and steps < MAX_SCAN_STEPS do
        local x = area.x
        while x + w <= area.x + area.w and steps < MAX_SCAN_STEPS do
            local c = rect(x, y, w, h)
            if not hits(c, occupied) then
                return c
            end
            x, steps = x + SCAN_STEP, steps + 1
        end
        y = y + SCAN_STEP
    end
    return cascade(rect(area.x, area.y, w, h), area, occupied)
end

-- Centred; cascades from the centre only when that spot is exactly taken.
local function centred(w, h, area, occupied)
    local c =
        clamp_inside(rect(area.x + math.max(0, area.w - w) // 2, area.y + math.max(0, area.h - h) // 2, w, h), area)
    for _, o in ipairs(occupied) do
        if o.x == c.x and o.y == c.y and o.w == c.w and o.h == c.h then
            return cascade(c, area, occupied)
        end
    end
    return c
end

local MODES = {
    free_space = free_space,
    center = centred,
    cascade = function(w, h, area, occupied)
        return cascade(rect(area.x, area.y, w, h), area, occupied)
    end,
}

-- The rectangle for a new window of w by h in `area`, avoiding `occupied` where possible. `mode` is "free_space",
-- "center" or "cascade". A size below 1 on an axis is raised to 1, so the window covers something.
function M.place(mode, w, h, area, occupied)
    local place = MODES[mode] or error("unknown placement mode " .. tostring(mode), 2)
    return place(math.max(1, w), math.max(1, h), area, occupied)
end

-- A placement is a method, so a mode is a subclass of Floating: Floating, Cascade and Centred.
Workspace:extend("Floating")({})
function Floating:place(w, h, area, occupied)
    return M.place("free_space", w, h, area, occupied)
end

Floating:extend("Cascade")({})
function Cascade:place(w, h, area, occupied)
    return M.place("cascade", w, h, area, occupied)
end

Floating:extend("Centred")({})
function Centred:place(w, h, area, occupied)
    return M.place("center", w, h, area, occupied)
end

-- A new window is placed once, when it first joins a workspace; `data.placed` stays true after that. A window
-- that has not committed a size yet opens at the default size.
function Floating:on_child_added(win)
    if not win:is(Window) or win.data.placed then
        return
    end
    -- Until this shell writes them, `width` and `height` hold the size the client committed (the compositor publishes it before ShellReady), so they are read before the rectangle below overwrites them.
    local w = win.width > 0 and win.width or DEFAULT_W
    local h = win.height > 0 and win.height or DEFAULT_H
    local occupied = {}
    for _, other in ipairs(self:all("> Window")) do
        if other ~= win and other.visible then
            occupied[#occupied + 1] = { x = other.x, y = other.y, w = other.width, h = other.height }
        end
    end
    local r = self:place(w, h, rect(0, 0, self.width, self.height), occupied)
    win.x, win.y, win.width, win.height = r.x, r.y, r.w, r.h
    win.data.placed = true
end

return M
