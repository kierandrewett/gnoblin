-- tests/lua/rules_test.lua: window rules of data/lua/gnoblin/shell/rules.lua. Pure: rules over plain window tables.
-- Patterns are Lua string patterns matched unanchored with string.find (window_rule.md); last matching rule wins
-- per field; a rule that matches nothing changes nothing.
local H = require("harness")

local function rules()
    return dofile(GNOBLIN_LUA_DIR .. "/shell/rules.lua")
end

local function win(t)
    return {
        app_id = t.app_id or "",
        class = t.class or "",
        title = t.title or "",
        focused = t.focused or false,
        workspace_id = t.workspace_id or "1",
    }
end

return {
    {
        "a pattern matches anywhere in the app_id unless anchored",
        function()
            local r = rules()
            local c = r.compile({ { match = { app_id = "fire" }, fullscreen = true } })
            H.check(r.effects(c, win({ app_id = "org.mozilla.firefox" })).fullscreen == true, "unanchored matches")
            local a = r.compile({ { match = { app_id = "^fire$" }, fullscreen = true } })
            H.check(r.effects(a, win({ app_id = "firefox" })).fullscreen == nil, "anchored does not match a longer id")
        end,
    },

    {
        "a malformed pattern is a compile error, not a silent non-match",
        function()
            local ok = pcall(rules().compile, { { match = { title = "[unclosed" }, secure = true } })
            H.check(not ok, "a malformed pattern must raise at compile time")
        end,
    },

    {
        "an unknown rule field raises, so a typo is not silently ignored",
        function()
            local ok = pcall(rules().compile, { { match = { app_id = "x" }, fullscren = true } })
            H.check(not ok, "an unknown field must raise")
        end,
    },

    {
        "when the app_id is empty the WM class stands in for it",
        function()
            local r = rules()
            local c = r.compile({ { match = { app_id = "^Gimp$" }, secure = true } })
            H.check(r.effects(c, win({ app_id = "", class = "Gimp" })).secure == true, "class falls back to app_id")
        end,
    },

    {
        "the last matching rule wins for a field",
        function()
            local r = rules()
            local c = r.compile({
                { match = { app_id = "foo" }, fullscreen = true },
                { match = { app_id = "foo" }, fullscreen = false },
            })
            H.check(r.effects(c, win({ app_id = "foo" })).fullscreen == false, "the later rule wins")
        end,
    },

    {
        "a later rule that does not set a field leaves the earlier value",
        function()
            local r = rules()
            local c = r.compile({
                { match = { app_id = "foo" }, secure = true },
                { match = { app_id = "foo" }, fullscreen = true },
            })
            local e = r.effects(c, win({ app_id = "foo" }))
            H.check(e.secure == true and e.fullscreen == true, "both fields apply")
        end,
    },

    {
        "workspace and corner rules carry their values",
        function()
            local r = rules()
            local c = r.compile({
                { match = { app_id = "term" }, workspace = { id = "2" }, corners = { radius = 8 } },
            })
            local e = r.effects(c, win({ app_id = "term" }))
            H.check(e.workspace.id == "2", "workspace id")
            H.check(e.corners.radius == 8, "corner radius")
        end,
    },

    {
        "the focused condition follows the window's focus",
        function()
            local r = rules()
            local c = r.compile({ { match = { focused = true }, secure = true } })
            H.check(r.effects(c, win({ focused = true })).secure == true, "focused matches")
            H.check(r.effects(c, win({ focused = false })).secure == nil, "unfocused does not")
        end,
    },

    {
        "the match key changes only when the set of matching rules changes",
        function()
            local r = rules()
            local c = r.compile({
                { match = { app_id = "foo" }, secure = true },
                { match = { focused = true }, fullscreen = true },
            })
            local a = r.key(c, win({ app_id = "foo" }))
            H.check(a == r.key(c, win({ app_id = "foo", title = "other title" })), "an unrelated change keeps the key")
            H.check(a ~= r.key(c, win({ app_id = "foo", focused = true })), "gaining focus changes the key")
        end,
    },

    {
        "a rule that stops matching clears secure (the old one-way set kept it)",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            root:add(Lock({}))
            root:add(Screen({ width = 800, height = 600 }))
            local w = root:add(Window({ app_id = "bank" }))
            root:find(Seat).data.window_rules = { { match = { app_id = "^bank$" }, secure = true } }
            H.load_shell()
            H.start_shell(host)
            H.check(w.secure == true, "the rule sets secure")
            host.external(host.ids("Window")[1], "app_id", "notes")
            H.frame(host, 1)
            H.check(w.secure == false, "secure must clear once no rule matches")
        end,
    },

    {
        "a title change re-evaluates the rules: secure follows the new title",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            root:add(Lock({}))
            root:add(Screen({ width = 800, height = 600 }))
            local w = root:add(Window({ title = "Inbox" }))
            root:find(Seat).data.window_rules = { { match = { title = "^Secret" }, secure = true } }
            H.load_shell()
            H.start_shell(host)
            H.check(w.secure == false, "no rule matches the first title")
            host.external(host.ids("Window")[1], "title", "Secret plans")
            H.frame(host, 1)
            H.check(w.secure == true, "the new title must match the rule")
        end,
    },

    {
        "re-applying unchanged rules writes nothing to the core",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            root:add(Lock({}))
            root:add(Screen({ width = 800, height = 600 }))
            root:add(Window({ app_id = "foo" }))
            root:find(Seat).data.window_rules = { { match = { app_id = "foo" }, fullscreen = true } }
            H.load_shell()
            H.start_shell(host)
            local writes = 0
            local set = core.set
            core.set = function(...)
                writes = writes + 1
                return set(...)
            end
            H.frame(host, 1)
            H.frame(host, 2)
            H.check(writes == 0, ("%d core writes on frames with no change"):format(writes))
        end,
    },

    {
        "rules merge in list order: a later rule overrides a field, an earlier field another rule does not set stays",
        function()
            local core, host = require("fake_core")()
            H.boot(core, host)
            root:add(Seat({}))
            root:add(Lock({}))
            root:add(Screen({ width = 800, height = 600 }))
            local w = root:add(Window({ app_id = "foo" }))
            root:find(Seat).data.window_rules = {
                { match = { app_id = "foo" }, secure = true },
                { match = { app_id = "foo" }, fullscreen = true },
                { match = { app_id = "foo" }, fullscreen = false },
            }
            H.load_shell()
            H.start_shell(host)
            H.check(w.secure == true and w.fullscreen == false, "secure kept, the last fullscreen wins")
        end,
    },

    {
        "a rule's opacity must be a number in [0, 1]",
        function()
            local r = rules()
            H.check(pcall(r.compile, { { match = { app_id = "x" }, opacity = 0.5 } }), "0.5 is valid")
            H.check(not pcall(r.compile, { { match = { app_id = "x" }, opacity = 1.5 } }), "above 1 is refused")
            H.check(not pcall(r.compile, { { match = { app_id = "x" }, opacity = "half" } }), "a string is refused")
        end,
    },

    {
        "corners is refused at load: the stock shell cannot round corners (effects are blur and shadow only)",
        function()
            local ok, err = pcall(rules().compile, { { match = { app_id = "x" }, corners = { radius = 8 } } })
            H.check(not ok and tostring(err):find("corners", 1, true), "corners must raise naming the field")
        end,
    },

    {
        "maximised is a boolean rule field applied to the window",
        function()
            local r = rules()
            local c = r.compile({ { match = { app_id = "foo" }, maximised = true } })
            H.check(r.effects(c, win({ app_id = "foo" })).maximised == true, "maximised applies")
            H.check(not pcall(r.compile, { { match = { app_id = "x" }, maximised = "yes" } }), "non-boolean refused")
        end,
    },
}
