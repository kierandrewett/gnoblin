-- Regression check: a rule on focused = false must dim a window when it loses focus.
gnoblin.window_rule {
    match = {type = "window", title = "^dim%-", focused = false},
    opacity = 0.4,
}
