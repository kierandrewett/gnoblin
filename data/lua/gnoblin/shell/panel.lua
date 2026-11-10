-- data/lua/gnoblin/shell/panel.lua: the stock panel. `toast` shows a short message on the Desk and removes it.
function toast(text)
    go(function()
        local d = desk()
        local t = d:add(Box({
            x = d.width - 340,
            y = 44,
            width = 320,
            height = 64,
            radius = 10,
            padding = 14,
            direction = "row",
            fill = "#45475a",
            opacity = 0,
            text,
        }))
        await(t:to(0.12, { opacity = 1 }))
        await(4)
        await(t:to(0.3, { opacity = 0 }))
        t:destroy()
    end)
end
