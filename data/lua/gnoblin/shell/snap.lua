-- data/lua/gnoblin/shell/snap.lua: edge tiling. Dragging a window to a screen edge tiles it to that part of the work
-- area. Off unless edge_tiling is true (window_management docs: default false). Pure functions over plain tables.
local M = {}

-- Whether edge tiling is on. Anything but a boolean raises.
function M.enabled(opts)
    local v = opts.edge_tiling
    if v == nil then
        return false
    end
    if type(v) ~= "boolean" then
        error(("edge_tiling must be a boolean, got %s"):format(tostring(v)), 2)
    end
    return v
end

-- The edge the pointer is within `margin` pixels of, or nil. Left and right win over top at corners.
function M.edge_zone(x, y, area, margin)
    if x - area.x <= margin then
        return "left"
    end
    if area.x + area.w - x <= margin then
        return "right"
    end
    if y - area.y <= margin then
        return "top"
    end
    return nil
end

-- The rectangle a window tiles to in `zone`, or nil when the zone does not tile.
function M.tile_rect(zone, area)
    local half = area.w // 2
    if zone == "left" then
        return { x = area.x, y = area.y, w = half, h = area.h }
    elseif zone == "right" then
        return { x = area.x + half, y = area.y, w = area.w - half, h = area.h }
    elseif zone == "top" then
        return { x = area.x, y = area.y, w = area.w, h = area.h }
    end
    return nil
end

return M
