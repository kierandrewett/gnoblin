-- floating: no tiling. Every tiled window is floated and keeps the rectangle it has.
--
-- arrange: returns no placements and one float effect per window.
-- on_request: declines every geometry request, so windows keep their rects.
-- options: none.
-- commands: none.
--
-- Usage: select it by name, "floating", in the layout settings.

local M = {}
M.kind = "floating"

M.options = {}

function M.arrange(ctx)
    local effects = {}
    for index, window in ipairs(ctx.windows) do
        effects[index] = { kind = "float", window = window.id }
    end
    return { placements = {}, effects = effects }
end

function M.on_request(_ctx)
    return nil
end

return M
