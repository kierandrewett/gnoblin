-- data/lua/gnoblin/shell/launcher.lua: Super+Space opens a command entry on the desk. Return spawns the command and
-- closes the entry; Escape closes it. Design section 6.5.
Entry:extend("Launcher")({
    width = 600,
    height = 48,
    radius = 12,
    fill = "#313244",
    size = 18,
})
function Launcher:on_key(e)
    if e.key == "Escape" then
        self:destroy()
    elseif e.key == "Return" then
        spawn(self.text)
        self:destroy()
    else
        Entry.on_key(self, e)
    end
end

seat:bind({
    ["Super+Space"] = function()
        local d = desk()
        local l = d:add(Launcher({
            x = d.width // 2 - 300,
            y = 200,
        }))
        seat.focus = l
    end,
})
