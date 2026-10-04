gnoblin.configure({
    window_management = {
        focus_mode = "click",
        -- This session tests per-window input, so activation uses the session's
        -- smart policy from startup. The main DevKit test covers strict policy.
        focus_new_windows = "smart",
    },
    shortcuts = {
        shell_input_capture = {
            binding = "Super",
            trigger = "release",
            capture_input = true,
        },
    },
    input_sources = {
        sources = {
            { type = "xkb", id = "us" },
            { type = "xkb", id = "gb" },
            { type = "ibus", id = "xkb:us::eng" },
        },
        per_window = true,
    },
})
