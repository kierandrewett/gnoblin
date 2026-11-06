-- master_stack: a master column (or row) holding the first windows, and a stack holding
-- the rest. Gaps: `outer` is the inset from ctx.area's edges, `inner` the space between
-- tiles.
--
-- arrange: master windows share the master region top to bottom (or left to right);
--          stack windows share the rest. With no stack the master fills the area.
-- on_request: declines every geometry request.
-- options:
--   master_count  windows in the master region (default 1; 0 puts all in the stack)
--   ratio         master region share of the free width, 0.1..0.9 (default 0.5)
--   orientation   "left" | "right" | "top" | "bottom" (default "left")
-- commands:
--   inc            one more master window
--   dec            one fewer master window (not below 0)
--   ratio          args.delta added to the ratio, clamped to 0.1..0.9
--   swap_master    swap args.window (or the focused window) with the first window
--   rotate         shift the window order by args.step (default 1)
-- state: master_count and ratio overrides from commands, and the window order.
--
-- Usage: select it by name, "master_stack", in the layout settings.

local M = {}
M.kind = "tiling"

M.options = {
    master_count = 1,
    ratio = 0.5,
    orientation = "left",
}

local ORIENTATIONS = { left = true, right = true, top = true, bottom = true }

local function clamp(value, low, high)
    return math.max(low, math.min(high, value))
end

-- Keeps the known windows in their stored order and appends new ones at the end.
local function reconcile(ctx)
    local state = ctx.state
    local live, seen, order = {}, {}, {}
    for _, window in ipairs(ctx.windows) do
        live[window.id] = true
    end
    for _, id in ipairs(state.order or {}) do
        if live[id] and not seen[id] then
            order[#order + 1] = id
            seen[id] = true
        end
    end
    for _, window in ipairs(ctx.windows) do
        if not seen[window.id] then
            order[#order + 1] = window.id
            seen[window.id] = true
        end
    end
    state.order = order
    return order
end

-- Rectangles in a frame where the master column sits at u = 0 and the frame is W x H.
-- Each entry is { u, v, p, q }: p along u, q along v.
local function frame_rects(W, H, n, master, ratio, inner)
    local rects = {}
    if n == 0 then
        return rects
    end
    local m = math.min(master, n)
    local s = n - m
    local master_w = 0
    if m > 0 then
        if s == 0 then
            master_w = W
        else
            master_w = math.max(0, (W - inner) * ratio)
        end
    end
    if m > 0 then
        local h = math.max(0, (H - (m - 1) * inner) / m)
        for i = 1, m do
            rects[i] = { 0, (i - 1) * (h + inner), master_w, h }
        end
    end
    if s > 0 then
        local start = m > 0 and (master_w + inner) or 0
        local w = math.max(0, W - start)
        local h = math.max(0, (H - (s - 1) * inner) / s)
        for i = 1, s do
            rects[m + i] = { start, (i - 1) * (h + inner), w, h }
        end
    end
    return rects
end

-- Maps a frame rectangle into the real area for the chosen orientation.
local function to_area(orientation, area_x, area_y, w0, h0, rect)
    local u, v, p, q = rect[1], rect[2], rect[3], rect[4]
    if orientation == "left" then
        return area_x + u, area_y + v, p, q
    elseif orientation == "right" then
        return area_x + (w0 - u - p), area_y + v, p, q
    elseif orientation == "top" then
        return area_x + v, area_y + u, q, p
    else
        return area_x + v, area_y + (h0 - u - p), q, p
    end
end

local function settings(ctx)
    local params = ctx.params
    local state = ctx.state
    local orientation = params.orientation
    if not ORIENTATIONS[orientation] then
        error("master_stack orientation must be left, right, top or bottom")
    end
    local master = state.master_count or params.master_count
    local ratio = clamp(state.ratio or params.ratio, 0.1, 0.9)
    return math.max(0, math.floor(master)), ratio, orientation
end

function M.arrange(ctx)
    local order = reconcile(ctx)
    local master, ratio, orientation = settings(ctx)
    local area = ctx.area
    local outer = ctx.params.outer or 0
    local inner = ctx.params.inner or 0

    local x0, y0 = area.x + outer, area.y + outer
    local w0 = math.max(0, area.width - 2 * outer)
    local h0 = math.max(0, area.height - 2 * outer)
    local W, H = w0, h0
    if orientation == "top" or orientation == "bottom" then
        W, H = h0, w0
    end

    local rects = frame_rects(W, H, #order, master, ratio, inner)
    local placements = {}
    for index, id in ipairs(order) do
        local x, y, w, h = to_area(orientation, x0, y0, w0, h0, rects[index])
        placements[index] = { window = id, x = x, y = y, width = w, height = h }
    end
    return { placements = placements }
end

function M.on_request(_ctx)
    return nil
end

local function index_of(order, id)
    for index, value in ipairs(order) do
        if value == id then
            return index
        end
    end
    return nil
end

local function effective_master(ctx)
    local master = settings(ctx)
    return master
end

M.command = {
    inc = function(ctx, _args)
        ctx.state.master_count = effective_master(ctx) + 1
    end,
    dec = function(ctx, _args)
        ctx.state.master_count = math.max(0, effective_master(ctx) - 1)
    end,
    ratio = function(ctx, args)
        local _, current = settings(ctx)
        ctx.state.ratio = clamp(current + (args.delta or 0), 0.1, 0.9)
    end,
    swap_master = function(ctx, args)
        local order = reconcile(ctx)
        local target = args.window or ctx.focus
        local index = target and index_of(order, target)
        if index and index ~= 1 then
            order[1], order[index] = order[index], order[1]
        end
        ctx.state.order = order
    end,
    rotate = function(ctx, args)
        local order = reconcile(ctx)
        local count = #order
        if count > 1 then
            local step = (args.step or 1) % count
            local rotated = {}
            for index = 1, count do
                rotated[index] = order[(index - 1 + step) % count + 1]
            end
            ctx.state.order = rotated
        end
    end,
}

return M
