-- Keyboard shortcuts use Lua callbacks over the shared input policy.
-- Install wpctl and playerctl for the corresponding media keys.
local function command(binding, argv)
    return {
        binding = binding,
        callback = function()
            gnoblin.commands.run(argv)
        end,
    }
end

-- Select the next or previous configured input source. With fewer than two sources there is nothing to switch.
local function switch_input_source(step)
    return function()
        local sources = gnoblin.input.sources()
        if #sources < 2 then
            return
        end
        local index = 1
        for position, source in ipairs(sources) do
            if source.current then
                index = position
            end
        end
        local target = sources[(index - 1 + step) % #sources + 1]
        gnoblin.input.select_source {type = target.type, id = target.id}
    end
end

gnoblin.configure {
    keybindings = {
        keyboard = {
            console = {
                binding = "<Alt>F2",
                callback = function()
                    gnoblin.console.toggle()
                end,
            },
            volume_up = command("XF86AudioRaiseVolume", {"wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"}),
            volume_down = command("XF86AudioLowerVolume", {"wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"}),
            volume_mute = command("XF86AudioMute", {"wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"}),
            microphone_mute = command("XF86AudioMicMute", {"wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"}),
            media_play_pause = command("XF86AudioPlay", {"playerctl", "play-pause"}),
            media_next = command("XF86AudioNext", {"playerctl", "next"}),
            media_previous = command("XF86AudioPrev", {"playerctl", "previous"}),
            files = command("<Super>e", {"sh", "-c", 'gio open "$HOME"'}),
            switch_input_source = {binding = "<Super>space", callback = switch_input_source(1)},
            switch_input_source_backward = {binding = "<Shift><Super>space", callback = switch_input_source(-1)},
        },
    },
}
