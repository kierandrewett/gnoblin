-- Fixture for tests/test-background-effect.py with GNOBLIN_BLUR_TEST_RULE=1.
gnoblin.configure({
    protocols = {
        wlr_layer_shell = true,
        wlr_screencopy = true,
        ext_background_effect_v1 = true,
    },
})

gnoblin.window_rule({
    match = { type = "layer" },
    blur = 24,
})
