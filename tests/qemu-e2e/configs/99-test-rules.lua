-- Regression check: a window rule must place a matching window on workspace 2.
gnoblin.window_rule {
    match = {type = "window", title = "^rule%-test$"},
    workspace = {number = 2},
}
