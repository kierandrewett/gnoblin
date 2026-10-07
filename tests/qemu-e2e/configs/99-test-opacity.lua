-- Regression check: the opacity rule field must make a matching window translucent.
gnoblin.window_rule {
    match = {type = "window", title = "^opacity%-test$"},
    opacity = 0.4,
}
