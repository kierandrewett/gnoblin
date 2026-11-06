-- monocle: one window at a time, full area, decorations kept. The shown window is the
-- focused one until the user cycles; cycling keeps the chosen window shown until
-- focus moves somewhere else.
--
-- arrange: the shown window fills the outer-inset area; the rest are visible = false.
-- on_request: declines every geometry request (the window keeps its rect).
-- options: none.
-- commands:
--   next: show the next window in stacking order (wraps).
--   prev: show the previous window in stacking order (wraps).
--
-- Usage: gnoblin.layout.register("monocle", "gnoblin.layouts.monocle")

local M = {}
M.kind = "tiling"

M.options = {}

local function index_of(ctx, id)
    for index, window in ipairs(ctx.windows) do
        if window.id == id then
            return index
        end
    end
    return nil
end

-- A focus change takes over the shown window; otherwise the last shown one stays.
local function shown_index(ctx)
    local state = ctx.state
    if ctx.focus ~= nil and ctx.focus ~= state.focus_seen then
        local index = index_of(ctx, ctx.focus)
        if index then
            state.focus_seen = ctx.focus
            state.shown = ctx.focus
        end
    end
    local index = state.shown ~= nil and index_of(ctx, state.shown) or nil
    if index == nil then
        index = 1
        local first = ctx.windows[1]
        state.shown = first and first.id or nil
    end
    return index
end

function M.arrange(ctx)
    local area = ctx.area
    local width = math.max(0, area.width)
    local height = math.max(0, area.height)
    local shown = shown_index(ctx)
    local placements = {}
    for index, window in ipairs(ctx.windows) do
        placements[index] = {
            window = window.id,
            x = area.x,
            y = area.y,
            width = width,
            height = height,
            visible = index == shown,
        }
    end
    return { placements = placements }
end

function M.on_request(_ctx)
    return nil
end

local function step(ctx, delta)
    local count = #ctx.windows
    if count == 0 then
        return
    end
    local index = shown_index(ctx)
    index = (index - 1 + delta) % count + 1
    ctx.state.shown = ctx.windows[index].id
end

M.command = {
    next = function(ctx, _args)
        step(ctx, 1)
    end,
    prev = function(ctx, _args)
        step(ctx, -1)
    end,
}

return M
