-- data/lua/gnoblin/shell/decorations.lua: the stock titlebar and the fade on close. Loaded after init.lua, so it
-- chains onto the stock Window:on_ready rather than replacing it. Design section 6.4.
Box:extend("Titlebar")({
    height = 28,
    padding = 6,
    gap = 8,
    direction = "row",
    input = true,
    fill = "#232634",
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
function Titlebar:on_press(e)
    e:stop()
    drag(seat, self.window)
end

local stock_ready = Window.on_ready
function Window:on_ready()
    stock_ready(self)
    self:add(Titlebar({ window = self }))
end

-- Replaces the stock method: fade out, then go.
function Window:vanish()
    await(self:to(0.18, { opacity = 0, scale = 0.92 }, ease.in_cubic))
    self:destroy()
end
