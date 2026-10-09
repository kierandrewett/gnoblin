-- Regression check: a blur rule must smooth what shows through a translucent window.
gnoblin.window_rule {
    match = {type = "window", title = "^blur%-test$"},
    blur = 40,
}
