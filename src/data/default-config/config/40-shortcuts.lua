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
        },
    },
}
