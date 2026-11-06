-- grid: a near-square grid of the windows in stacking order. The grid is as close to
-- square as the count allows; a partly filled last row or column is left-aligned.
--
-- arrange: each window gets one cell; `outer` insets the area, `inner` separates cells.
-- on_request: declines every geometry request.
-- options:
--   fill   "row" (fill left to right, then down) or "column" (fill top to bottom,
--          then right). Default "row".
-- commands: none.
--
-- Usage: select it by name, "grid", in the layout settings.

local M = {}
M.kind = "tiling"

M.options = {
    fill = "row",
}

-- Column and row counts for `n` windows in the given fill order.
local function dimensions(n, fill)
    local side = math.ceil(math.sqrt(n))
    if fill == "column" then
        local rows = side
        return math.ceil(n / rows), rows
    elseif fill == "row" then
        return side, math.ceil(n / side)
    end
    error("grid fill must be row or column")
end

function M.arrange(ctx)
    local n = #ctx.windows
    local placements = {}
    if n == 0 then
        return { placements = placements }
    end

    local area = ctx.area
    local outer = ctx.params.outer or 0
    local inner = ctx.params.inner or 0
    local x0, y0 = area.x + outer, area.y + outer
    local W = math.max(0, area.width - 2 * outer)
    local H = math.max(0, area.height - 2 * outer)

    local cols, rows = dimensions(n, ctx.params.fill)
    local cell_w = math.max(0, (W - (cols - 1) * inner) / cols)
    local cell_h = math.max(0, (H - (rows - 1) * inner) / rows)

    for index, window in ipairs(ctx.windows) do
        local col, row
        if ctx.params.fill == "column" then
            col = math.floor((index - 1) / rows)
            row = (index - 1) % rows
        else
            col = (index - 1) % cols
            row = math.floor((index - 1) / cols)
        end
        placements[index] = {
            window = window.id,
            x = x0 + col * (cell_w + inner),
            y = y0 + row * (cell_h + inner),
            width = cell_w,
            height = cell_h,
        }
    end
    return { placements = placements }
end

function M.on_request(_ctx)
    return nil
end

return M
