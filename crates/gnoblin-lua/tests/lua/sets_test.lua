-- tests/lua/sets_test.lua: workspace sets of data/lua/gnoblin/shell/sets.lua. Pure: a set is { name, entries = { {
-- workspace, output? } }. Ports the workspace-set cases of crates/gnoblin-wm/tests/failure_modes.rs. A refused list is
-- never partly applied: `define` returns nil and the previous list stays with the caller.
local H = require("harness")

local function sets()
    return dofile(GNOBLIN_LUA_DIR .. "/shell/sets.lua")
end

local function set(name, entries)
    return { name = name, entries = entries }
end

-- A started shell with one Screen for each output name (the first is primary), holding the user's `sets`. Returns the
-- host and the screens.
local function shell(sets, outputs)
    local core, host = require("fake_core")()
    H.boot(core, host)
    root:add(Seat({}))
    root:add(Lock({}))
    local screens = {}
    for i, name in ipairs(outputs or { "DP-1", "HDMI-1" }) do
        screens[i] = root:add(Screen({ width = 800, height = 600, name = name, primary = i == 1 }))
    end
    root:find(Seat).data.workspace_sets = sets
    H.load_shell()
    H.publish_seat(host, { screen = host.ids("Screen")[1] })
    H.start_shell(host)
    return host, screens
end

local function shown(screen)
    return desk_of(screen).active.name
end

-- Workspaces 1 to 3 exist; the tests that need others say so.
local DECLARED = { [1] = true, [2] = true, [3] = true }

return {
    {
        "activating an unknown set is refused and changes nothing",
        function()
            local s = sets()
            local list = assert(s.define({}, { set("work", { { workspace = 2 } }) }, DECLARED))
            local plan, err = s.activate(list, "nope", { declared = DECLARED, outputs = {}, primary = "DP-1" })
            H.check(plan == nil, "an unknown set has no plan")
            H.check(err.kind == "unknown_set" and err.name == "nope", "refused as an unknown set")
        end,
    },
    {
        "a set with no entries is refused and the previous sets stay",
        function()
            local s = sets()
            local prev = assert(s.define({}, { set("work", { { workspace = 2 } }) }, DECLARED))
            local list, err = s.define(prev, { set("empty", {}) }, DECLARED)
            H.check(list == nil, "the empty set is refused")
            H.check(err.kind == "empty_set" and err.name == "empty", "refused as an empty set")
            H.check(#prev == 1 and prev[1].name == "work", "the previous sets are untouched")
        end,
    },
    {
        "a set naming an undeclared workspace is refused as a whole",
        function()
            local s = sets()
            local prev = assert(s.define({}, { set("work", { { workspace = 2 } }) }, DECLARED))
            local list, err =
                s.define(prev, { set("good", { { workspace = 1 } }), set("bad", { { workspace = 99 } }) }, DECLARED)
            H.check(list == nil, "the whole list is refused")
            H.check(err.kind == "undeclared_workspace" and err.set == "bad" and err.workspace == 99, "names the set")
            H.check(prev[1].name == "work" and #prev == 1, "a refused list is never partly applied")
        end,
    },
    {
        "a set whose workspace is removed refuses to activate",
        function()
            local s = sets()
            local list = assert(s.define({}, { set("work", { { workspace = 3 } }) }, DECLARED))
            local without3 = { [1] = true, [2] = true }
            local plan, err = s.activate(list, "work", { declared = without3, outputs = {}, primary = "DP-1" })
            H.check(plan == nil, "no plan for a removed workspace")
            H.check(err.kind == "undeclared_workspace" and err.set == "work" and err.workspace == 3, "names it")
        end,
    },
    {
        "duplicate set names are refused",
        function()
            local s = sets()
            local list, err = s.define(
                {},
                { set("work", { { workspace = 1 } }), set("work", { { workspace = 2 } }) },
                DECLARED
            )
            H.check(list == nil, "refused")
            H.check(err.kind == "duplicate_name" and err.name == "work", "refused as a duplicate name")
        end,
    },
    {
        "an empty set name is refused",
        function()
            local s = sets()
            local list, err = s.define({}, { set("", { { workspace = 1 } }) }, DECLARED)
            H.check(list == nil, "refused")
            H.check(err.kind == "empty_name", "refused as an empty name")
        end,
    },
    {
        "two entries for one output in a set are refused",
        function()
            local s = sets()
            local list, err = s.define({}, {
                set("clash", { { workspace = 1, output = "HDMI-1" }, { workspace = 2, output = "HDMI-1" } }),
            }, DECLARED)
            H.check(list == nil, "refused")
            H.check(
                err.kind == "duplicate_output" and err.set == "clash" and err.output == "HDMI-1",
                "names the output"
            )
        end,
    },
    {
        "an unknown output falls back to the primary with a warning",
        function()
            local s = sets()
            local list = assert(s.define({}, { set("work", { { workspace = 3, output = "GONE-1" } }) }, DECLARED))
            local plan = assert(s.activate(list, "work", {
                declared = DECLARED,
                outputs = { "DP-1", "HDMI-1" },
                primary = "DP-1",
            }))
            H.check(plan.placements[1].workspace == 3 and plan.placements[1].output == "DP-1", "placed on the primary")
            H.check(#plan.warnings == 1, "one warning")
            H.check(
                plan.warnings[1].kind == "output_fallback"
                    and plan.warnings[1].set == "work"
                    and plan.warnings[1].output == "GONE-1",
                "warns of the missing output"
            )
        end,
    },
    {
        "a set shows each named workspace on its own output",
        function()
            local s = sets()
            local list = assert(
                s.define(
                    {},
                    { set("split", { { workspace = 1, output = "DP-1" }, { workspace = 2, output = "HDMI-1" } }) },
                    DECLARED
                )
            )
            local plan = assert(s.activate(list, "split", {
                declared = DECLARED,
                outputs = { "DP-1", "HDMI-1" },
                primary = "DP-1",
            }))
            H.check(plan.placements[1].workspace == 1 and plan.placements[1].output == "DP-1", "workspace 1 on DP-1")
            H.check(
                plan.placements[2].workspace == 2 and plan.placements[2].output == "HDMI-1",
                "workspace 2 on HDMI-1"
            )
            H.check(#plan.warnings == 0, "no fallback warning")
        end,
    },
    {
        "activating the live set again changes nothing",
        function()
            local s = sets()
            local list = assert(s.define({}, { set("work", { { workspace = 2, output = "HDMI-1" } }) }, DECLARED))
            local opts = { declared = DECLARED, outputs = { "DP-1", "HDMI-1" }, primary = "DP-1" }
            local first = assert(s.activate(list, "work", opts))
            H.check(first.changed, "the first activation changes the live set")
            opts.active = "work"
            local second = assert(s.activate(list, "work", opts))
            H.check(not second.changed, "the second activation changes nothing")
        end,
    },
    -- The user's sets as the stock shell reads them (`seat.data.workspace_sets`), activated with `activate_set`.
    {
        "the user's sets show each named workspace on its own output",
        function()
            local host, screens = shell({
                set("split", { { workspace = 3, output = "DP-1" }, { workspace = "2", output = "HDMI-1" } }),
            })
            H.check(#host.reports == 0, "a valid list reports nothing: " .. table.concat(host.reports, "; "))
            H.check(activate_set("split") == true, "the set activates")
            H.check(shown(screens[1]) == "3", "workspace 3 on DP-1")
            H.check(shown(screens[2]) == "2", "workspace 2 on HDMI-1")
            H.check(#host.reports == 0, "no report")
        end,
    },
    {
        "activating an unknown set is refused, reported and changes nothing in the stock shell",
        function()
            local host, screens = shell({ set("work", { { workspace = 2, output = "DP-1" } }) })
            H.check(activate_set("nope") == false, "an unknown set is refused")
            H.check(#host.reports == 1 and host.reports[1]:find("\"nope\"", 1, true), "reported by name")
            H.check(shown(screens[1]) == "1", "nothing changed")
        end,
    },
    {
        "a list naming a workspace no desk has is refused whole at start and nothing activates",
        function()
            local host = shell({
                set("good", { { workspace = 1 } }),
                set("bad", { { workspace = 99 } }),
            })
            H.check(#host.reports == 1, "one report expected, got " .. #host.reports)
            H.check(
                host.reports[1]:find("\"bad\"", 1, true) and host.reports[1]:find("\"99\"", 1, true),
                "names the set"
            )
            H.check(activate_set("good") == false, "a refused list is never partly applied")
        end,
    },
    {
        "a refused new list is reported once and the sets in force stay",
        function()
            local host, screens = shell({ set("work", { { workspace = 2, output = "DP-1" } }) })
            root:find(Seat).data.workspace_sets = { set("empty", {}) }
            H.check(activate_set("work") == true, "the old set still activates")
            H.check(shown(screens[1]) == "2", "the old set is in force")
            H.check(activate_set("work") == true and #host.reports == 1, "the refusal is reported once")
            H.check(host.reports[1]:find("\"empty\"", 1, true), "names the empty set: " .. host.reports[1])
        end,
    },
    {
        "an output that is gone shows its workspace on the primary output, with a report",
        function()
            local host, screens = shell({ set("work", { { workspace = 3, output = "GONE-1" } }) })
            H.check(activate_set("work") == true, "the set still activates")
            H.check(shown(screens[1]) == "3", "the workspace shows on the primary output")
            H.check(shown(screens[2]) == "1", "the other output is untouched")
            H.check(#host.reports == 1 and host.reports[1]:find("\"GONE-1\"", 1, true), "the output is named")
        end,
    },
    {
        "activating the live set again in the stock shell changes nothing",
        function()
            local _, screens = shell({ set("work", { { workspace = 3, output = "DP-1" } }) })
            H.check(activate_set("work") == true, "first activation")
            local d = desk_of(screens[1])
            go(function()
                d:show(d.space.children[1])
            end)
            H.check(activate_set("work") == true, "second activation")
            H.check(shown(screens[1]) == "1", "the live set is not applied again")
        end,
    },
}
