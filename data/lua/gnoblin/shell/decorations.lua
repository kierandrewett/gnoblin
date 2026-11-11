-- data/lua/gnoblin/shell/decorations.lua: the stock titlebar and the fade on close. Loaded after init.lua, so it
-- chains onto the stock Window:on_ready rather than replacing it. Design section 6.4.
Box:extend("Titlebar")({
    window = Window,
    height = 28,
    padding = 6,
    gap = 8,
    direction = "row",
    input = true,
    width = function(t)
        return t.window.width
    end,
    fill = function(t)
        return seat.focus == t.window and "#414559" or "#232634"
    end,
})
function Titlebar:build()
    local w = self.window
    return {
        Text({
            grow = 1,
            size = 13,
            color = "#c6d0f5",
            text = function()
                return w.title
            end,
        }),
        Icon({
            icon = "window-close-symbolic",
            on_press = function(_, e)
                e:stop()
                w:close()
            end,
        }),
    }
end
-- The titlebar gestures. A left press drags, unless it is the second press of a double click within DOUBLE_CLICK
-- seconds of the first on the same window. Each gesture's action is the seat property of the same name.
local LEFT, RIGHT, MIDDLE = 272, 273, 274
local DOUBLE_CLICK = 0.5
local last_press = setmetatable({}, { __mode = "k" })
-- What each action does to its window. `menu`, `minimize` and the one-axis maximise have no Lua surface yet, so they
-- do nothing; `none` does nothing too. An action the schema does not name raises.
local ACTIONS = {
    ["toggle-maximize"] = function(w)
        if w.maximised then
            w:unmaximise()
        else
            w:maximise()
        end
    end,
    lower = function(w)
        w:lower()
    end,
    none = function() end,
    menu = function() end,
    minimize = function() end,
    ["toggle-maximize-horizontally"] = function() end,
    ["toggle-maximize-vertically"] = function() end,
}
local function gesture(w, property)
    local run = ACTIONS[seat[property]]
    if not run then
        error(("%s must name a titlebar action, got %s"):format(property, tostring(seat[property])), 2)
    end
    run(w)
end
function Titlebar:on_press(e)
    e:stop()
    local w = self.window
    if e.button == MIDDLE then
        gesture(w, "middle_click")
    elseif e.button == RIGHT then
        gesture(w, "right_click")
    elseif e.button == LEFT then
        local now, prev = gnoblin.timers.now(), last_press[w]
        last_press[w] = now
        if prev and now - prev <= DOUBLE_CLICK then
            last_press[w] = nil
            gesture(w, "double_click")
        else
            drag(seat, w)
        end
    end
end

-- Signal handlers chain: this runs after init.lua's Window:on_ready, so the stock placement still happens first.
function Window:on_ready()
    self:add(Titlebar({ window = self }))
end

-- Replaces the stock method: fade out, then go.
function Window:vanish()
    await(self:to(motion(0.18), { opacity = 0, scale = { 0.92, 0.92, 1 } }, ease.in_cubic))
    self:destroy()
end
