-- tests/lua/gestures_test.lua: gesture claims on the Seat and the stock gestures (data/lua/gnoblin/shell/gestures.lua)
-- over the fake core. The compositor's side of a claim is tested in Rust; here an event is what it would send.
local H = require("harness")

-- A started shell with one Screen (1000 wide) and one Seat, and a Window built from `props` when given. Returns the
-- host, the window and the stock gestures module.
local function setup(props)
    local core, host = require("fake_core")()
    H.boot(core, host)
    root:add(Seat({}))
    root:add(Screen({ width = 1000, height = 800 }))
    local win = props and root:add(Window(props))
    H.load_shell()
    H.publish_seat(host, { screen = host.ids("Screen")[1] })
    H.start_shell(host)
    return host, win, require("gnoblin.shell.gestures")
end

-- The id of the last `claim_gesture` call for `kind` and `fingers`, or nil.
local function claim_id(host, kind, fingers)
    for i = #host.calls, 1, -1 do
        local c = host.calls[i]
        if c.method == "claim_gesture" and c.text == kind then
            local id, n = string.unpack("<i8i8", c.args[1])
            if n == fingers then
                return id
            end
        end
    end
end

-- The event the compositor sends for one phase of a claimed gesture.
local function gesture(id, phase, dx, dy, extra)
    local g = {
        claim = id,
        kind = "swipe",
        fingers = 3,
        phase = phase,
        dx = dx or 0,
        dy = dy or 0,
        scale = 1,
        rotation = 0,
    }
    for k, v in pairs(extra or {}) do
        g[k] = v
    end
    return { target = 0, type = "gesture", bubbles = false, gesture = g }
end

-- Runs a whole swipe through the shell at frame time `t`: begin, one update to (`dx`, `dy`), then `last` (end or
-- cancel).
local function swipe(host, id, t, dx, dy, last)
    H.frame(host, t, { gesture(id, "begin") })
    H.frame(host, t + 0.01, { gesture(id, "update", dx, dy) })
    H.frame(host, t + 0.02, { gesture(id, last or "end", dx, dy) })
end

-- Lets any slide the shell started run to its end.
local function settle(host, from)
    for i = 1, 4 do
        H.frame(host, from + i, {})
    end
end

-- Every write to a node's `x`, in order: its value and its duration (nil for an instant write).
local function x_writes(host, id)
    local out = {}
    for _, batch in ipairs(host.batches) do
        for _, o in ipairs(batch) do
            if o.op == "set" and o.id == id and o.key == "x" then
                out[#out + 1] = { value = o.value, seconds = o.seconds }
            end
        end
    end
    return out
end

return {
    {
        "the stock shell claims three-finger swipes for the workspace slide",
        function()
            local host = setup()
            H.check(claim_id(host, "swipe", 3) ~= nil, "no claim_gesture call for a three-finger swipe")
        end,
    },
    {
        "Seat:bind claims a gesture by its spelling and shares the claim ids with chords",
        function()
            local host = setup()
            local before = #host.calls
            seat:bind({ Swipe4 = function() end, Pinch2 = false, ["<Super>q"] = function() end })
            local made = {}
            for i = before + 1, #host.calls do
                local c = host.calls[i]
                local id, extra = string.unpack("<i8i8", c.args[1])
                made[#made + 1] = { method = c.method, text = c.text, id = id, extra = extra }
            end
            H.check(#made == 3, "three claims, got " .. #made)
            -- Sorted by key: "<Super>q", "Pinch2", "Swipe4".
            H.check(made[1].method == "claim" and made[1].text == "<Super>q", "the chord claims as it always did")
            H.check(made[2].method == "claim_gesture" and made[2].text == "pinch" and made[2].extra == 2, "Pinch2")
            H.check(made[3].method == "claim_gesture" and made[3].text == "swipe" and made[3].extra == 4, "Swipe4")
            H.check(
                made[2].id == made[1].id + 1 and made[3].id == made[2].id + 1,
                "one id space for chords and gestures"
            )
        end,
    },
    {
        "a gesture event runs the fn bound under its claim for every phase, and unbind stops it",
        function()
            local host = setup()
            local phases = {}
            seat:bind({
                Swipe4 = function(e)
                    phases[#phases + 1] = ("%s:%d:%d"):format(e.phase, e.dx, e.dy)
                end,
            })
            local id = claim_id(host, "swipe", 4)
            H.check(id ~= nil, "bind sent no claim for Swipe4")
            local function event(phase, dx, dy)
                return gesture(id, phase, dx, dy, { fingers = 4 })
            end
            H.frame(host, 1, { event("begin", 0, 0) })
            H.frame(host, 2, { event("update", 10, -4) })
            H.frame(host, 3, { event("end", 10, -4) })
            H.check(table.concat(phases, " ") == "begin:0:0 update:10:-4 end:10:-4", table.concat(phases, " "))
            seat:unbind(id)
            local last = host.calls[#host.calls]
            H.check(last.method == "unclaim" and string.unpack("<i8", last.args[1]) == id, "unbind sent no unclaim")
            H.frame(host, 4, { event("begin", 0, 0) })
            H.check(#phases == 3, "the fn ran after unbind: " .. #phases)
        end,
    },
    {
        "a gesture claimed with a fn of false claims it and runs nothing",
        function()
            local host = setup()
            seat:bind({ Swipe5 = false })
            local id = claim_id(host, "swipe", 5)
            H.check(id ~= nil, "the gesture is claimed all the same")
            H.frame(host, 1, { gesture(id, "begin", 0, 0, { fingers = 5 }) })
        end,
    },
    {
        "a gesture event for a claim nobody holds is ignored",
        function()
            local host = setup()
            H.frame(host, 1, { gesture(9999, "begin") })
            H.check(#host.reports == 0, "an unknown claim must not be reported")
        end,
    },
    {
        "a three-finger swipe follows the fingers: the next workspace comes in as they move left",
        function()
            local host = setup()
            local d = desk()
            local first, second = d.space.children[1], d.space.children[2]
            local id = claim_id(host, "swipe", 3)
            H.frame(host, 1, { gesture(id, "begin") })
            H.frame(host, 1.1, { gesture(id, "update", -200, 0) }) -- half of the 400 units of travel
            H.check(first.x == -500, "the active workspace must follow the fingers, x = " .. first.x)
            H.check(second.visible and second.x == 500, "the next workspace must come in, x = " .. second.x)
            H.frame(host, 1.2, { gesture(id, "update", -400, 0) })
            H.check(first.x == -1000 and second.x == 0, "a whole travel brings it all the way in")
            H.frame(host, 1.3, { gesture(id, "update", -900, 0) })
            H.check(first.x == -1000 and second.x == 0, "movement past the travel does not overshoot")
        end,
    },
    {
        "fingers moving right bring the previous workspace in from the left",
        function()
            local host = setup()
            local d = desk()
            go(function()
                d:show(d.space.children[3])
            end)
            settle(host, 0)
            local id = claim_id(host, "swipe", 3)
            H.frame(host, 10, { gesture(id, "begin") })
            H.frame(host, 10.1, { gesture(id, "update", 100, 0) })
            local previous = d.space.children[2]
            H.check(previous.visible and previous.x == -750, "the previous workspace must come in, x = " .. previous.x)
            H.check(d.space.children[3].x == 250, "the active one follows the fingers to the right")
        end,
    },
    {
        "lifting the fingers past the commit point finishes the slide from where they left it",
        function()
            local host = setup()
            local d = desk()
            local first, second = d.space.children[1], d.space.children[2]
            local first_id, second_id = host.ids("Workspace")[1], host.ids("Workspace")[2]
            local id = claim_id(host, "swipe", 3)
            swipe(host, id, 1, -200, 0)
            settle(host, 1)
            H.check(d.active == second and second.active and not first.active, "the next workspace must be active")
            H.check(second.x == 0 and second.visible, "it must end in place")
            H.check(not first.visible and first.x == 0, "the old one must be hidden and put back")
            local writes = x_writes(host, second_id)
            H.check(
                writes[#writes - 1].value == 500 and writes[#writes - 1].seconds == nil,
                "it starts where the finger left it"
            )
            local tween = writes[#writes]
            H.check(
                tween.value == 0 and tween.seconds == 0.125,
                "it takes the rest of the slide: " .. tostring(tween.seconds)
            )
            local out = x_writes(host, first_id)
            -- The last write puts the hidden workspace back at 0; the one before it is the slide out.
            H.check(out[#out].value == 0 and out[#out].seconds == nil, "the old one is put back once it is hidden")
            H.check(
                out[#out - 1].value == -1000 and out[#out - 1].seconds == 0.125,
                "the old one slides out for as long"
            )
        end,
    },
    {
        "lifting the fingers before the commit point slides back and leaves the workspace as it was",
        function()
            local host = setup()
            local d = desk()
            local first, second = d.space.children[1], d.space.children[2]
            local id = claim_id(host, "swipe", 3)
            swipe(host, id, 1, -40, 0) -- a tenth of the travel
            settle(host, 1)
            H.check(d.active == first and first.active, "the active workspace must not change")
            H.check(first.x == 0, "it must be back in place, x = " .. first.x)
            H.check(not second.visible and second.x == 0, "the next workspace must be hidden again")
        end,
    },
    {
        "a cancelled swipe slides back even when it went far",
        function()
            local host = setup()
            local d = desk()
            local first, second = d.space.children[1], d.space.children[2]
            local id = claim_id(host, "swipe", 3)
            swipe(host, id, 1, -300, 0, "cancel")
            settle(host, 1)
            H.check(d.active == first and first.x == 0, "a cancel must not switch workspace")
            H.check(not second.visible, "the next workspace must be hidden again")
        end,
    },
    {
        "a swipe that is more vertical than horizontal moves nothing",
        function()
            local host = setup()
            local d = desk()
            local first, second = d.space.children[1], d.space.children[2]
            local id = claim_id(host, "swipe", 3)
            swipe(host, id, 1, -30, -300)
            settle(host, 1)
            H.check(d.active == first and first.x == 0, "the workspace must not move")
            H.check(not second.visible, "no workspace must come in")
        end,
    },
    {
        "a swipe toward no workspace moves nothing",
        function()
            local host = setup()
            local d = desk()
            local id = claim_id(host, "swipe", 3)
            swipe(host, id, 1, 300, 0) -- right of the first workspace: there is none
            settle(host, 1)
            H.check(d.active == d.space.children[1] and d.space.children[1].x == 0, "the first workspace stays put")
        end,
    },
    {
        "a swipe that turns back hides the workspace it had brought in and brings the other one",
        function()
            local host = setup()
            local d = desk()
            go(function()
                d:show(d.space.children[2])
            end)
            settle(host, 0)
            local previous, active, next_ws = d.space.children[1], d.space.children[2], d.space.children[3]
            local id = claim_id(host, "swipe", 3)
            H.frame(host, 10, { gesture(id, "begin") })
            H.frame(host, 10.1, { gesture(id, "update", -200, 0) })
            H.check(next_ws.visible, "the next workspace must come in first")
            H.frame(host, 10.2, { gesture(id, "update", 200, 0) })
            H.check(not next_ws.visible and next_ws.x == 0, "it must go away when the fingers turn back")
            H.check(previous.visible and previous.x == -500 and active.x == 500, "the previous one takes its place")
        end,
    },
    {
        "with reduced motion on, a committed swipe writes no tween and still ends in place",
        function()
            local host = setup()
            H.publish_seat(host, { reduced_motion = true })
            H.frame(host, 0.5, {})
            local d = desk()
            local first, second = d.space.children[1], d.space.children[2]
            local id = claim_id(host, "swipe", 3)
            swipe(host, id, 1, -300, 0)
            settle(host, 1)
            H.check(#host.timed_sets() == 0, "no write may carry a duration, got " .. #host.timed_sets())
            H.check(d.active == second and second.x == 0, "the next workspace must be in place")
            H.check(not first.visible, "the old workspace must be hidden")
        end,
    },
    {
        "workspace_next and workspace_previous step the desk",
        function()
            local host, _, Gestures = setup()
            local d = desk()
            Gestures.actions.workspace_next()
            settle(host, 0)
            H.check(d.active == d.space.children[2], "workspace_next must show the second workspace")
            Gestures.actions.workspace_previous()
            settle(host, 10)
            H.check(d.active == d.space.children[1], "workspace_previous must show the first workspace")
        end,
    },
    {
        "window_close closes the focused window and does nothing without one",
        function()
            local host, win, Gestures = setup({ app_id = "foot" })
            seat.focus = nil
            local before = #host.calls
            Gestures.actions.window_close()
            H.check(#host.calls == before, "no focused window: nothing to close")
            seat.focus = win
            Gestures.actions.window_close()
            H.check(host.calls[#host.calls].method == "close", "the focused window must be asked to close")
        end,
    },
    {
        "window_minimise hides the focused window",
        function()
            local _, win, Gestures = setup({ app_id = "foot" })
            seat.focus = win
            Gestures.actions.window_minimise()
            H.check(win.minimised, "the focused window must be minimised")
        end,
    },
    {
        "window_toggle_maximise maximises the focused window and then restores it",
        function()
            local _, win, Gestures = setup({ app_id = "foot" })
            seat.focus = win
            Gestures.actions.window_toggle_maximise()
            H.check(win.maximised, "the first toggle must maximise")
            Gestures.actions.window_toggle_maximise()
            H.check(not win.maximised, "the second toggle must restore")
        end,
    },
    {
        "on_swipe runs the fn for the direction a swipe ended in, past the threshold only",
        function()
            local _, _, Gestures = setup()
            local ran = {}
            local handler = Gestures.on_swipe({
                up = function()
                    ran[#ran + 1] = "up"
                end,
                right = function()
                    ran[#ran + 1] = "right"
                end,
            })
            handler({ phase = "end", dx = 10, dy = -120 })
            handler({ phase = "end", dx = 90, dy = 30 })
            handler({ phase = "end", dx = 20, dy = -20 }) -- under the 48 units
            handler({ phase = "end", dx = -200, dy = 0 }) -- left: not mapped
            handler({ phase = "update", dx = 0, dy = -300 }) -- only the end acts
            handler({ phase = "cancel", dx = 0, dy = -300 })
            H.check(table.concat(ran, ",") == "up,right", "ran " .. table.concat(ran, ","))
        end,
    },
    {
        "on_pinch runs out for a spread and in for a gather, past the threshold only",
        function()
            local _, _, Gestures = setup()
            local ran = {}
            local handler = Gestures.on_pinch({
                out = function()
                    ran[#ran + 1] = "out"
                end,
                ["in"] = function()
                    ran[#ran + 1] = "in"
                end,
            })
            handler({ phase = "end", scale = 1.5 })
            handler({ phase = "end", scale = 0.6 })
            handler({ phase = "end", scale = 1.05 }) -- inside the 0.12 threshold
            handler({ phase = "cancel", scale = 2 })
            H.check(table.concat(ran, ",") == "out,in", "ran " .. table.concat(ran, ","))
        end,
    },
    {
        "a user binds a swipe to an action: four fingers up toggles maximise through the whole chain",
        function()
            local host, win, Gestures = setup({ app_id = "foot" })
            seat.focus = win
            seat:bind({ Swipe4 = Gestures.on_swipe({ up = Gestures.actions.window_toggle_maximise }) })
            local id = claim_id(host, "swipe", 4)
            local function event(phase, dy)
                return gesture(id, phase, 0, dy, { fingers = 4 })
            end
            H.frame(host, 1, { event("begin", 0) })
            H.frame(host, 2, { event("update", -80) })
            H.check(not win.maximised, "nothing runs before the swipe ends")
            H.frame(host, 3, { event("end", -120) })
            H.check(win.maximised, "the swipe up must maximise the focused window")
        end,
    },
}
