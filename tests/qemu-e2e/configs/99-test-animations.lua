-- Experiment: slow fade-ins for normal windows and for dialogs, so a screenshot can tell them apart.
gnoblin.animation {
    name = "slow-open",
    event = "open",
    duration = 3000,
    ease = "linear",
    from = {opacity = 0},
    to = {opacity = 1},
}

gnoblin.animation {
    name = "slow-dialog-open",
    event = "dialog-open",
    duration = 3000,
    ease = "linear",
    from = {opacity = 0},
    to = {opacity = 1},
}
