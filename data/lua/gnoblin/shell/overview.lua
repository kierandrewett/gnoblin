-- data/lua/gnoblin/shell/overview.lua: the stock overview. While it is on, every workspace of the Desk is shown at
-- full strength; while it is off, all but the active workspace are dimmed.
local overviewing = false
function Desk:toggle_overview()
    overviewing = not overviewing
    for _, ws in ipairs(self.space.children) do
        ws.opacity = (overviewing or ws == self.active) and 1 or 0.4
    end
end
