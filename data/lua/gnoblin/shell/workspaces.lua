-- data/lua/gnoblin/shell/workspaces.lua: Super+1 to Super+9 slide the desk to that workspace. The slide itself is
-- Desk:show in init.lua. Design section 6.7.
for i = 1, 9 do
    seat:bind({
        ["Super+" .. i] = function()
            go(function()
                desk():show(desk().space.children[i])
            end)
        end,
    })
end
