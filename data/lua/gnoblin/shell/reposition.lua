-- data/lua/gnoblin/shell/reposition.lua: moving and resizing a window with the keyboard. The window menu's Move and
-- Resize start it, so a window can be placed without a pointer.
--
--   Window:move_with_keys()     the arrow keys move the window by STEP pixels, inside the work area
--   Window:resize_with_keys()   Right and Down grow the window by STEP pixels, Left and Up shrink it
--
-- Return, Enter and Space keep the new place; Escape puts the window back where it was. The mode takes the keyboard
-- while it runs and gives it back to the window when it ends. It ends when the window closes and when the session locks.
-- Only a floating window can be moved this way: a maximised, fullscreen or tiled window is refused (false), and a
-- snapped one is unsnapped first, as a drag does. One window is repositioned at a time.
local Drag = require("gnoblin.shell.drag")
local M = {}

M.STEP = 16
M.MIN_WIDTH, M.MIN_HEIGHT = 120, 80

-- The mode now running, or nil.
local current

-- The node that holds the keyboard while the mode runs. It draws nothing itself; its child is the one-line hint.
Box:extend("RepositionGrab")({
    window = Window,
    before = Node,
    mode = "move",
    from = {},
    snapped = "",
    width = 1,
    height = 1,
})

local ARROWS = {
    Left = { -1, 0 },
    Right = { 1, 0 },
    Up = { 0, -1 },
    Down = { 0, 1 },
}

function RepositionGrab:step(dx, dy)
    local w, area = self.window, work_area()
    if self.mode == "move" then
        local r = Drag.constrain(
            { x = w.x + dx, y = w.y + dy, w = w.width, h = w.height },
            area,
            Drag.mode(seat.drag_constraint)
        )
        w.x, w.y = r.x, r.y
    else
        local widest, tallest = area.x + area.w - w.x, area.y + area.h - w.y
        w.width = math.max(M.MIN_WIDTH, math.min(w.width + dx, widest))
        w.height = math.max(M.MIN_HEIGHT, math.min(w.height + dy, tallest))
    end
end

function RepositionGrab:on_key(e)
    if e.pressed == false then
        return
    end
    local key = e.key
    local arrow = ARROWS[key]
    if arrow then
        self:step(arrow[1] * M.STEP, arrow[2] * M.STEP)
    elseif key == "Return" or key == "KP_Enter" or key == "space" then
        M.finish(true)
    elseif key == "Escape" then
        M.finish(false)
    end
    if e.stop then
        e:stop()
    end
end

-- Ends the mode. `keep` keeps where the window is; otherwise it goes back to where the mode found it.
function M.finish(keep)
    local grab = current
    if not grab then
        return false
    end
    current = nil
    local w = grab.window
    if not keep then
        local r = grab.from
        w.x, w.y, w.width, w.height = r.x, r.y, r.w, r.h
        if grab.snapped ~= "" then
            w.data.snapped = grab.snapped
        end
    end
    pcall(function()
        grab:destroy()
    end)
    pcall(function()
        seat.focus = w
    end)
    return true
end

-- Whether the mode is running, for which window.
function M.current()
    return current and current.window or nil
end

local function begin(w, mode)
    M.finish(true)
    local d = w:closest(Desk)
    if not d or w.fullscreen or w.maximised or w.tiled or w:closest(Workspace) ~= d.active then
        return false
    end
    local snapped = w.data.snapped
    local from = { x = w.x, y = w.y, w = w.width, h = w.height }
    if snapped then
        w:unsnap()
    end
    local grab = d:add(RepositionGrab({
        name = "reposition",
        window = w,
        before = seat.focus,
        mode = mode,
        from = from,
        snapped = snapped or "",
    }))
    local what = mode == "move" and "Move" or "Resize"
    grab:add(Box({
        x = 20,
        y = 20,
        width = 520,
        height = 36,
        radius = 8,
        fill = "#1e1e2e",
        padding = 8,
        direction = "row",
        secure = w.secure,
        Text({ text = what .. ": arrow keys, Return to keep it, Escape to cancel", size = 14, grow = 1 }),
    }))
    current = grab
    seat.focus = grab
    return true
end

function Window:move_with_keys()
    return begin(self, "move")
end
function Window:resize_with_keys()
    return begin(self, "resize")
end

local function ends_with(w)
    if current and current.window == w then
        M.finish(false)
    end
end
function Window:on_closed()
    ends_with(self)
end
function Window:on_destroyed()
    ends_with(self)
end
function Lock:on_requested()
    M.finish(true)
end

return M
