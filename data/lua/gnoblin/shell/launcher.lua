-- data/lua/gnoblin/shell/launcher.lua: Super+Space opens a command entry on the desk. Return spawns the command and
-- closes the entry; Escape closes it. Design section 6.5.
Entry:extend("Launcher")({
    width = 600,
    height = 48,
    radius = 12,
    fill = "#313244",
    size = 18,
})
-- Key presses are a signal: the handler chains after Entry's, so Entry's typing still runs first. Escape and
-- Return are the launcher's own keys.
function Launcher:on_key(e)
    if e.key == "Escape" then
        self:destroy()
    elseif e.key == "Return" then
        spawn(self.text)
        self:destroy()
    end
end

-- The Seat exists only in a running shell, so the chord is bound only when one does.
if seat then
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
end
