-- kiosk: one window at a time, full area, no decorations. The shown window is the
-- focused one (or the last shown, or the first). Every other window is hidden.
--
-- arrange: the shown window fills ctx.area with decorated = false; the rest are
--          placed on the same rect with visible = false, so they stay tiled.
-- on_request: refuses every geometry change by answering with the fill rect.
-- options: none.
-- commands: none.
--
-- Usage: gnoblin.layout.register("kiosk", "gnoblin.layouts.kiosk")

local M = {}
M.kind = "kiosk"

M.options = {}

local function has(ctx, id)
    for _, window in ipairs(ctx.windows) do
        if window.id == id then
            return true
        end
    end
    return false
end

-- The shown window: a focus change takes over, otherwise the last shown one stays.
local function shown_id(ctx)
    local state = ctx.state
    if ctx.focus ~= nil and ctx.focus ~= state.focus_seen and has(ctx, ctx.focus) then
        state.focus_seen = ctx.focus
        state.shown = ctx.focus
    end
    if state.shown ~= nil and has(ctx, state.shown) then
        return state.shown
    end
    local first = ctx.windows[1]
    state.shown = first and first.id or nil
    return state.shown
end

function M.arrange(ctx)
    local area = ctx.area
    local shown = shown_id(ctx)
    local placements = {}
    for index, window in ipairs(ctx.windows) do
        placements[index] = {
            window = window.id,
            x = area.x,
            y = area.y,
            width = math.max(0, area.width),
            height = math.max(0, area.height),
            visible = window.id == shown,
            decorated = false,
        }
    end
    return { placements = placements }
end

function M.on_request(ctx)
    return M.arrange(ctx)
end

return M
