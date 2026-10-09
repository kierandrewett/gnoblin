-- Pointer bindings and behavior are Lua policy. Mutter performs the grab.
gnoblin.configure {
    keybindings = {
        pointer = {
            move = {
                binding = "<Super>Button1",
                callback = function(event)
                    if not event.window or not event.focus_context or not event.window.movable then
                        return "forward"
                    end
                    event.window:begin_move(event.focus_context)
                    return "consume"
                end,
            },
            resize = {
                binding = "<Super>Button3",
                callback = function(event)
                    local window = event.window
                    if not window or not event.focus_context or not window.resizable then
                        return "forward"
                    end
                    local frame = window.frame
                    local vertical = event.pointer.y < frame.y + frame.height / 2
                        and "north" or "south"
                    local horizontal = event.pointer.x < frame.x + frame.width / 2
                        and "west" or "east"
                    window:begin_resize(vertical .. "_" .. horizontal, event.focus_context)
                    return "consume"
                end,
            },
        },
    },
}
