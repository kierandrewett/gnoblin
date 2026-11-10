-- data/lua/gnoblin/shell/strip.lua: two workspace layouts made of plain windows.
-- Strip: a row of columns, one window per column, scrolled so the focused column stays in view.
-- Carousel3d: windows on a ring seen in perspective, the focused one at the front.
-- Loaded before init.lua, so `Window:on_closed` here runs before the shell's `vanish`.
-- Not yet loaded by the compositor: the stock library is not wired into the shell VM (design/node-api-refactor.md).
local M = {}

local TAU = 2 * math.pi
local MIN_COLUMN, MAX_COLUMN = 0.1, 1

-- A column's width as a fraction of the strip's width, clamped to 0.1..1. A non-number or NaN is an error.
function M.column(v)
    if type(v) ~= "number" or v ~= v then
        error("column must be a number, got " .. tostring(v), 2)
    end
    return math.min(math.max(v, MIN_COLUMN), MAX_COLUMN)
end

-- Per-Strip scroll state, kept off the node: view offset, whether the user pinned it, and the focus last seen.
local state = setmetatable({}, { __mode = "k" })
local function st(self)
    local s = state[self]
    if not s then
        s = { view = 0, pinned = false, focus = nil }
        state[self] = s
    end
    return s
end

-- Columns sit edge to edge with no gap or margin. The view follows focus unless `scroll_to` pinned it; a pin
-- ends when the focused window changes.
Workspace:extend("Strip")({
    column = 0.5,
})

function Strip:arrange()
    local s, wins = st(self), self:all("> Window")
    local w, vw = M.column(self.column) * self.width, self.width
    local f = seat.focus
    if f ~= s.focus then
        s.focus, s.pinned = f, false
    end
    if not s.pinned then
        for i, win in ipairs(wins) do
            if win == f then
                local x = (i - 1) * w
                if x < s.view or x + w > s.view + vw then
                    s.view = x + w / 2 - vw / 2 -- centre the focused column
                end
            end
        end
    end
    s.view = math.min(math.max(s.view, 0), math.max(0, #wins * w - vw))
    for i, win in ipairs(wins) do
        win.x, win.y, win.width, win.height = (i - 1) * w - s.view, 0, w, self.height
    end
end

-- A new window opens right after the focused column, not at the end. A window already in place stays put.
function Strip:on_child_added(win)
    if not win:is(Window) then
        return
    end
    local others, at, fpos = {}, nil, nil
    for i, c in ipairs(self.children) do
        if c == win then
            at = i
        else
            others[#others + 1] = c
            if c == seat.focus then
                fpos = #others
            end
        end
    end
    local want = fpos and fpos + 1 or #others + 1
    if at ~= want then
        self:add(win, want) -- emits child_added again; the second time `at == want` and this stops
    end
end

local function step(self, d)
    local wins = self:all("> Window")
    for i, win in ipairs(wins) do
        if win == seat.focus then
            if wins[i + d] then
                seat.focus = wins[i + d]
            end
            return
        end
    end
end
function Strip:focus_left()
    step(self, -1)
end
function Strip:focus_right()
    step(self, 1)
end

-- Scrolls the view to `x` pixels and keeps it there until the focused window changes.
function Strip:scroll_to(x)
    if type(x) ~= "number" or x ~= x or math.abs(x) == math.huge then
        error("scroll_to needs a finite number, got " .. tostring(x), 2)
    end
    local s = st(self)
    s.view, s.pinned = x, true
    self:arrange()
end

-- Focus moves to a neighbour before a focused window closes, so the strip keeps a focus.
function Window:on_closed()
    local p = self.parent
    if seat.focus ~= self or not p or not p:is(Strip) then
        return
    end
    local wins = p:all("> Window")
    for i, win in ipairs(wins) do
        if win == self then
            seat.focus = wins[i + 1] or wins[i - 1] or seat.focus
            return
        end
    end
end

-- The pure ring layout. `n` windows, `focus` the 1-based index at the front, `area` {x, y, width, height}.
-- Returns one slot per window: x, y, width, height (the projected top-left and size), opacity, rotation (degrees),
-- and z (depth, negative in front of the camera). The arrange applies only x, y, width, height and opacity; rotation
-- and z are for a renderer that can use them.
function M.ring(n, focus, area, opts)
    opts = opts or {}
    if n == 0 then
        return {}
    end
    if math.type(focus) ~= "integer" or focus < 1 or focus > n then
        error("focus must be an index in 1.." .. n .. ", got " .. tostring(focus), 2)
    end
    local cw = area.width * (opts.card_width or 0.36)
    local ch = area.height * (opts.card_height or 0.6)
    local R = math.max(cw * 1.25, n * cw * 1.25 / TAU)
    local F = math.max(1, (ch / 2) / math.tan(math.rad(opts.fov or 40) / 2))
    local zc = -(F + R)
    local cx, cy = area.x + area.width / 2, area.y + area.height / 2
    local out = {}
    for i = 1, n do
        local k = (i - focus) % n
        if k > n / 2 then
            k = k - n -- signed offset from the front, in (-n/2, n/2]
        end
        local a = k * TAU / n
        local X, Z = R * math.sin(a), zc + R * math.cos(a)
        local scale = F / -Z
        local w, h = cw * scale, ch * scale
        out[i] = {
            x = cx + X * scale - w / 2,
            y = cy - h / 2,
            width = w,
            height = h,
            opacity = 0.4 + 0.6 * (1 + math.cos(a)) / 2,
            rotation = math.deg(a),
            z = Z,
        }
    end
    return out
end

Workspace:extend("Carousel3d")({
    fov = 40,
    card_width = 0.36,
    card_height = 0.6,
})

-- The focused window is the front card; with no focus in the workspace, the first window is.
function Carousel3d:arrange()
    local wins = self:all("> Window")
    local focus = 1
    for i, win in ipairs(wins) do
        if win == seat.focus then
            focus = i
        end
    end
    local slots = M.ring(#wins, focus, { x = 0, y = 0, width = self.width, height = self.height }, {
        fov = self.fov,
        card_width = self.card_width,
        card_height = self.card_height,
    })
    for i, win in ipairs(wins) do
        local s = slots[i]
        win.x, win.y, win.width, win.height, win.opacity = s.x, s.y, s.width, s.height, s.opacity
    end
end

-- Moves focus `step` cards round the ring, wrapping at both ends.
function Carousel3d:rotate(step)
    if type(step) ~= "number" then
        error("rotate needs a number of steps, got " .. tostring(step), 2)
    end
    local wins = self:all("> Window")
    local n = #wins
    for i, win in ipairs(wins) do
        if win == seat.focus then
            seat.focus = wins[(i - 1 + step) % n + 1]
            return
        end
    end
end

return M
