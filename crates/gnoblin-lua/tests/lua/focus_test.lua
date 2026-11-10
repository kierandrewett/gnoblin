-- tests/lua/focus_test.lua: the focus policy of data/lua/gnoblin/shell/focus.lua. The module is pure: options in,
-- decisions out. The shell wiring (press raises, hover focus, auto-raise timer) is exercised in shell_test.lua.
local H = require("harness")

local function focus()
    return dofile(GNOBLIN_LUA_DIR .. "/shell/focus.lua")
end

return {
    {
        "with no options, the documented defaults apply",
        function()
            local o = focus().settings({})
            H.check(o.focus_mode == "click", "focus_mode defaults to click")
            H.check(o.focus_new_windows == "prevent", "focus_new_windows defaults to prevent")
            H.check(o.raise_on_click == true, "raise_on_click defaults to true")
            H.check(o.auto_raise == false, "auto_raise defaults to false")
            H.check(o.auto_raise_delay == 500, "auto_raise_delay defaults to 500 ms")
            H.check(o.focus_change_on_pointer_rest == false, "focus_change_on_pointer_rest defaults to false")
        end,
    },

    {
        "a user option overrides only the key it names",
        function()
            local o = focus().settings({ focus_mode = "hover" })
            H.check(o.focus_mode == "hover", "the named key takes the user value")
            H.check(o.raise_on_click == true, "the other keys keep their defaults")
        end,
    },

    {
        "an unknown focus_mode raises instead of falling back",
        function()
            local ok = pcall(focus().settings, { focus_mode = "sloppy" })
            H.check(not ok, "an unknown focus_mode must raise")
        end,
    },

    {
        "focus_new_windows accepts only allow or prevent",
        function()
            local ok = pcall(focus().settings, { focus_new_windows = "always" })
            H.check(not ok, "an unknown focus_new_windows must raise")
        end,
    },

    {
        "auto_raise_delay must be a non-negative number of milliseconds",
        function()
            H.check(not pcall(focus().settings, { auto_raise_delay = -1 }), "a negative delay must raise")
            H.check(not pcall(focus().settings, { auto_raise_delay = "soon" }), "a string delay must raise")
        end,
    },

    {
        "an unknown option name raises, so typos are not silently ignored",
        function()
            local ok = pcall(focus().settings, { focus_mod = "hover" })
            H.check(not ok, "an unknown key must raise")
        end,
    },

    {
        "prevent keeps a window started from a launcher behind; allow focuses it",
        function()
            local f = focus()
            local prevent, allow = f.settings({}), f.settings({ focus_new_windows = "allow" })
            H.check(not f.opens_focused(prevent, true), "prevent must not focus a launcher window")
            H.check(f.opens_focused(allow, true), "allow must focus a launcher window")
        end,
    },

    {
        "a window not started from a launcher takes focus under either policy",
        function()
            local f = focus()
            H.check(f.opens_focused(f.settings({}), false), "prevent still focuses an ordinary window")
            H.check(f.opens_focused(f.settings({ focus_new_windows = "allow" }), false), "allow focuses it too")
        end,
    },

    {
        "click mode never moves focus on hover",
        function()
            local f = focus()
            local o = f.settings({})
            H.check(not f.hover_focus(o, { id = 1 }, true), "click mode ignores pointer entry")
        end,
    },

    {
        "hover mode focuses the window under the pointer, on entry or movement",
        function()
            local f = focus()
            local o = f.settings({ focus_mode = "hover" })
            H.check(f.hover_focus(o, { id = 1 }, true), "hover focuses on entry")
            H.check(f.hover_focus(o, { id = 1 }, false), "hover focuses on movement within a window")
            H.check(not f.hover_focus(o, nil, false), "hover over the desktop focuses nothing")
        end,
    },

    {
        "hover-strict focuses only when the pointer enters the window",
        function()
            local f = focus()
            local o = f.settings({ focus_mode = "hover-strict" })
            H.check(f.hover_focus(o, { id = 1 }, true), "hover-strict focuses on entry")
            H.check(not f.hover_focus(o, { id = 1 }, false), "hover-strict ignores movement inside a window")
        end,
    },

    {
        "auto_raise_delay converts to seconds for the frame clock",
        function()
            local f = focus()
            H.check(f.auto_raise_seconds(f.settings({})) == 0.5, "500 ms is 0.5 s")
            H.check(f.auto_raise_seconds(f.settings({ auto_raise_delay = 1500 })) == 1.5, "1500 ms is 1.5 s")
        end,
    },
}
