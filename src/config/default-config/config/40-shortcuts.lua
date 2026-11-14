-- Keyboard shortcuts that run a command, by name. Each command runs directly with no shell.
-- Install wpctl and playerctl for the corresponding media keys.
-- Removed from the starter: the Alt+F2 console toggle and the Super+E files shortcut. The
-- first has no declarative form (it needs a Lua callback, which keyboard bindings do not
-- carry yet); the second needs $HOME, which a direct argv cannot expand without a shell.
gnoblin.configure({
    shortcuts = {
        ["volume-up"] = {
            binding = "XF86AudioRaiseVolume",
            command = { "wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+" },
        },
        ["volume-down"] = {
            binding = "XF86AudioLowerVolume",
            command = { "wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-" },
        },
        ["volume-mute"] = {
            binding = "XF86AudioMute",
            command = { "wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle" },
        },
        ["microphone-mute"] = {
            binding = "XF86AudioMicMute",
            command = { "wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle" },
        },
        ["media-play-pause"] = { binding = "XF86AudioPlay", command = { "playerctl", "play-pause" } },
        ["media-next"] = { binding = "XF86AudioNext", command = { "playerctl", "next" } },
        ["media-previous"] = { binding = "XF86AudioPrev", command = { "playerctl", "previous" } },
    },
})
