-- data/lua/gnoblin/shell/drag.lua: where a dragged window may go, and what a drag does to a window first. Pure functions
-- over plain tables. By default a window stays inside the work area; under `minimum_strip` it may be pushed partly off
-- screen but a STRIP-pixel edge stays inside, so it can never become unreachable.
local M = {}

-- The visible strip of a window kept inside the work area under minimum_strip.
M.STRIP = 75

-- The constraint named by the seat's drag_constraint. Absent is work_area; any other value raises.
function M.mode(v)
    if v == nil or v == "work_area" then
        return "work_area"
    end
    if v == "minimum_strip" then
        return "minimum_strip"
    end
    error(("drag_constraint must be work_area or minimum_strip, got %s"):format(tostring(v)), 2)
end

-- One axis: a window of extent `size` at `pos`, against an area starting at `start` and `span` long.
local function clamp_axis(pos, size, start, span, mode)
    if mode == "work_area" then
        if size >= span then
            return start
        end
        return math.min(math.max(pos, start), start + span - size)
    end
    local visible = math.min(M.STRIP, size)
    local low = start - (size - visible)
    local high = start + span - visible
    return math.min(math.max(pos, low), high)
end

-- The rectangle `r` moved to the place `mode` allows, keeping its size.
function M.constrain(r, area, mode)
    return {
        x = clamp_axis(r.x, r.w, area.x, area.w, mode),
        y = clamp_axis(r.y, r.h, area.y, area.h, mode),
        w = r.w,
        h = r.h,
    }
end

-- What a drag does to a window in its current state: "refuse" (fullscreen windows do not move), "float" (a maximised
-- window returns to its floating rectangle first, then moves) or "move".
function M.start(w)
    if w.fullscreen then
        return "refuse"
    end
    if w.maximised then
        return "float"
    end
    return "move"
end

return M
