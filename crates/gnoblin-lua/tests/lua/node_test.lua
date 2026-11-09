-- tests/lua/node_test.lua: the class system of data/lua/gnoblin/node.lua over the fake native core.
-- Each case gets its own runtime from H.fresh(), so no case sees another's classes or nodes.
-- Returns a list of { name, fn }; a case fails by raising.
local H = require("harness")

return {
    -- Classes and properties.
    {
        "a subclass inherits a default and a constructor field overrides it",
        function()
            H.fresh()
            local Panel = Canvas:extend("Panel")({ width = 3 })
            local a, b = Panel({}), Panel({ width = 9 })
            H.check(a.width == 3, "inherited default, got " .. tostring(a.width))
            H.check(b.width == 9, "constructor field, got " .. tostring(b.width))
        end,
    },

    {
        "a Lua property rejects a value of the wrong type",
        function()
            H.fresh()
            local Panel = Canvas:extend("Panel")({ label = "" })
            local p = Panel({})
            local ok, err = pcall(function()
                p.label = 42
            end)
            H.check(
                not ok and tostring(err):find("takes string", 1, true),
                "wrong type must raise, got: " .. tostring(err)
            )
        end,
    },

    {
        "a property that does not exist raises",
        function()
            H.fresh()
            local p = Canvas({})
            local ok, err = pcall(function()
                p.nope = 1
            end)
            H.check(not ok and tostring(err):find("has no property", 1, true), "unknown property must raise")
        end,
    },

    {
        "a native property is written to the mirror and reads back",
        function()
            local _, host = H.fresh()
            local p = Canvas({ x = 4 })
            H.check(p.x == 4, "read back before the frame")
            host.frame(0, {})
            H.check(host.dump():find("x=4", 1, true), "mirror holds x=4")
        end,
    },

    {
        "a node reference reads back as the same Lua node",
        function()
            H.fresh()
            local seat = Seat({})
            local w = Window({})
            seat.focus = w
            H.check(rawequal(seat.focus, w), "focus is the same Lua node")
        end,
    },

    -- Reactivity.
    {
        "a binding re-runs when a property it reads changes",
        function()
            local _, host = H.fresh()
            local outer = Canvas({ width = 100 })
            local inner = Canvas({})
            outer:add(inner)
            inner.width = function(n)
                return n.parent.width / 2
            end
            H.check(inner.width == 50, "initial binding value, got " .. tostring(inner.width))
            outer.width = 200
            H.check(inner.width == 100, "binding follows the parent, got " .. tostring(inner.width))
            host.frame(0, {})
            H.check(#host.reports == 0, "no reports")
        end,
    },

    {
        "a binding does not re-run for its own write",
        function()
            H.fresh()
            local runs = 0
            local c = Canvas({})
            c.width = function(n)
                runs = runs + 1
                return n.width
            end
            H.check(runs == 1, "a self-reading binding runs once, got " .. runs)
        end,
    },

    -- Bug b: two bindings that feed each other stop, report once, and leave no queue behind.
    {
        "two bindings that feed each other report once, and the next frame does not report again",
        function()
            local _, host = H.fresh()
            local c = Canvas({})
            c.x = function(n)
                return n.y + 1
            end
            c.y = function(n)
                return n.x + 1
            end
            host.frame(0, {})
            H.check(#host.reports == 1, "one report expected, got " .. #host.reports)
            host.frame(1, {})
            H.check(#host.reports == 1, "the stuck queue must not report again, got " .. #host.reports)
        end,
    },

    -- Handlers and signals.
    {
        "a class handler runs for each instance that receives the event",
        function()
            local _, host = H.fresh()
            local seen = 0
            local Panel = Canvas:extend("Panel")({})
            function Panel:on_press()
                seen = seen + 1
            end
            local a, b = Panel({}), Panel({})
            root:add(a)
            root:add(b)
            local ids = host.ids("Canvas")
            H.check(#ids == 2, "two Canvas nodes, got " .. #ids)
            host.frame(0, { { type = "press", bubbles = true, target = ids[1] } })
            host.frame(1, { { type = "press", bubbles = true, target = ids[2] } })
            H.check(seen == 2, "two presses, got " .. seen)
        end,
    },

    {
        "an input event bubbles from the target, and stop() ends the walk",
        function()
            local _, host = H.fresh()
            local outer = Canvas({})
            local inner = Canvas({})
            root:add(outer)
            outer:add(inner)
            local hits = {}
            function outer:on_press()
                hits[#hits + 1] = "outer"
            end
            function inner:on_press(e)
                hits[#hits + 1] = "inner"
                e:stop()
            end
            local ids = host.ids("Canvas")
            host.frame(0, { { type = "press", bubbles = true, target = ids[2] } })
            H.same(hits, { "inner" })
        end,
    },

    {
        "a press on a child reaches its parent when no handler stops it",
        function()
            local _, host = H.fresh()
            local outer = Canvas({})
            local inner = Canvas({})
            root:add(outer)
            outer:add(inner)
            local hits = {}
            function outer:on_press()
                hits[#hits + 1] = "outer"
            end
            local ids = host.ids("Canvas")
            host.frame(0, { { type = "press", bubbles = true, target = ids[2] } })
            H.same(hits, { "outer" })
        end,
    },

    {
        "await resumes after a timer and not before",
        function()
            local _, host = H.fresh()
            local done
            go(function()
                await(0.5)
                done = "fired"
            end)
            H.check(done == nil, "not yet")
            host.frame(0.4, {})
            H.check(done == nil, "still not at 0.4")
            host.frame(0.5, {})
            H.check(done == "fired", "fired at 0.5")
        end,
    },

    {
        "a ready handler that awaits is resumed by the frame that reaches its timer",
        function()
            local _, host = H.fresh()
            local at
            local c = Canvas({})
            function c:on_ready()
                await(1)
                at = "waited"
            end
            root:add(c)
            H.check(at == nil, "waiting")
            host.frame(1, {})
            H.check(at == "waited", "resumed at 1")
        end,
    },

    -- Motion.
    {
        "to() returns a signal that fires when the motion ends",
        function()
            local _, host = H.fresh()
            local c = Canvas({ x = 0 })
            local done = c:to(0.2, { x = 10 })
            local finished = false
            go(function()
                await(done)
                finished = true
            end)
            host.frame(0.1, {})
            H.check(not finished, "not finished at 0.1")
            host.frame(0.2, {})
            H.check(finished, "finished at 0.2")
        end,
    },

    -- Selectors.
    {
        "find, all and closest match a class and a name",
        function()
            H.fresh()
            local root_box = Canvas({ name = "root" })
            local Panel = Canvas:extend("Panel")({})
            local p1 = Panel({ name = "one" })
            local p2 = Panel({ name = "two" })
            root_box:add(p1)
            p1:add(p2)
            H.check(root_box:find("Panel#two") == p2, "find by name")
            H.check(#root_box:all("Panel") == 2, "all by class")
            H.check(p2:closest("Panel") == p2, "closest finds itself")
            H.check(p2:closest("Panel#one") == p1, "closest walks up")
        end,
    },

    {
        "a child selector matches direct children only",
        function()
            H.fresh()
            local outer = Canvas({})
            local mid = Canvas({})
            local leaf = Canvas({})
            outer:add(mid)
            mid:add(leaf)
            H.check(#outer:all("> Canvas") == 1, "one direct child")
            H.check(#outer:all("Canvas") == 2, "two descendants")
        end,
    },

    -- Destruction.
    {
        "destroy removes a subtree, and a handler of a destroyed node never resumes",
        function()
            local _, host = H.fresh()
            local ran = false
            local c = Canvas({})
            local child = Canvas({})
            function child:on_ready()
                await(1)
                ran = true
            end
            root:add(c)
            c:add(child)
            c:destroy()
            H.check(#root.children == 0, "subtree left the root")
            host.frame(1, {})
            H.check(not ran, "a destroyed node's coroutine must not resume")
            H.check(#host.reports == 0, "no reports")
        end,
    },

    -- Bug a: a handler started by a binding runs outside the binding, so it may await.
    {
        "a handler started by a binding can await",
        function()
            local _, host = H.fresh()
            local resumed = false
            local holder = Canvas({})
            root:add(holder)
            local Late = Canvas:extend("Late")({})
            function Late:on_ready()
                await(0.1)
                resumed = true
            end
            holder.width = function(n)
                n:add(Late({}))
                return 1
            end
            H.check(#host.reports == 0, "await inside a binding must not report: " .. (host.reports[1] or ""))
            host.frame(0.1, {})
            H.check(resumed, "handler resumed after the timer")
        end,
    },

    -- Bug c: a timer scheduled while a frame runs is placed in time order.
    {
        "a timer scheduled during a frame is in the next wake time and fires on time",
        function()
            local _, host = H.fresh()
            local c_done = false
            go(function()
                await(5)
                await(1)
                c_done = true
            end)
            go(function()
                await(20)
            end)
            local wake = host.frame(5, {})
            H.check(wake == 6, "wake at 6, got " .. tostring(wake))
            host.frame(6, {})
            H.check(c_done, "the timer at 6 fired at 6")
        end,
    },

    -- Bug d: wrapping a node that Gnoblin made does not build or arrange it.
    {
        "wrapping a node Gnoblin made does not run its build",
        function()
            local core, host = H.fresh()
            local built = 0
            function Canvas:build()
                built = built + 1
                return {}
            end
            local id = core.new("Canvas")
            core.insert(core.root(), id)
            host.frame(0, {})
            local kids = root.children
            H.check(#kids == 1, "one child, got " .. #kids)
            H.check(built == 0, "build ran " .. built .. " times on a node Gnoblin made")
        end,
    },

    -- Bug e: a change to a node destroyed in the same frame is dropped, and nothing is reported.
    {
        "a change to a node already destroyed is dropped without a report",
        function()
            local _, host = H.fresh()
            local seat = Seat({})
            local w = Window({})
            root:add(seat)
            seat.focus = w
            host.frame(0, {})
            host.close(host.ids("Window")[1])
            host.close(host.ids("Seat")[1])
            host.frame(1, {})
            host.frame(2, {})
            H.check(#host.reports == 0, "reports: " .. table.concat(host.reports, "; "))
        end,
    },

    -- Bug g: a native read-only property (a parent, a child list, an index) has no default to write at
    -- construction, so `make` must skip it; writing it would raise in the host.
    {
        "a class over a native read-only property constructs without writing it",
        function()
            local _, host = H.fresh()
            local ws = Workspace({ name = "1" })
            root:add(ws)
            H.check(type(ws.index) == "number", "index must read back as a number, got " .. tostring(ws.index))
            H.check(#host.reports == 0, "reports: " .. table.concat(host.reports, "; "))
        end,
    },

    -- Bug f: extend checks its base when it is called, not when the class is declared.
    {
        "extend rejects a base that is not a class",
        function()
            H.fresh()
            local ok, err = pcall(function()
                Node.extend("nope", "Bad")
            end)
            H.check(
                not ok and tostring(err):find("base", 1, true),
                "extend with a string base must raise, got " .. tostring(err)
            )
        end,
    },
}
