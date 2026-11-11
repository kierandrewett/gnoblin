-- tests/lua/xr_test.lua: the stock shell's XR side (data/lua/gnoblin/shell/xr.lua) over the fake core. The room, its head
-- camera and its Views are the contract with the gnoblin-xr extension (extensions/gnoblin-xr: xr-scene reads them,
-- xr-tree maps the node properties). The ring is a pure function, tested with golden numbers; the rest runs the shell.
local H = require("harness")

local function near(got, want, what)
    H.check(
        math.abs(got - want) <= 1e-9 * math.max(1, math.abs(want)),
        ("%s: got %.12g, want %.12g"):format(what, got, want)
    )
end

-- A pose against its golden numbers: millimetres in the room's frame, yaw in radians, scale in mm per pixel.
local function pose_is(p, want, what)
    near(p.x, want[1], what .. " x")
    near(p.y, want[2], what .. " y")
    near(p.z, want[3], what .. " z")
    near(p.rotation[1], 0, what .. " pitch")
    near(p.rotation[2], want[4], what .. " yaw")
    near(p.rotation[3], 0, what .. " roll")
    near(p.scale, want[5], what .. " scale")
    H.check(p.focused == want[6], what .. " focused")
    H.check(p.behind == want[7], what .. " behind")
end

local function same_list(got, want, what)
    H.check(#got == #want, ("%s: got %d entries, want %d"):format(what, #got, #want))
    for i = 1, #want do
        H.check(got[i] == want[i], ("%s: entry %d is %s, want %s"):format(what, i, tostring(got[i]), tostring(want[i])))
    end
end

-- `n` views of `w` by `h` pixels, ids 1..n.
local function panels(n, w, h)
    local out = {}
    for i = 1, n do
        out[i] = { id = i, width = w, height = h }
    end
    return out
end

-- The stock shell over a 1000 by 800 screen with a seat and `n` windows that map before it starts (800 by 600 each).
-- With `xr_boot`, the seat named `xr` is in the boot snapshot too. Returns host and the windows.
local function shell(n, xr_boot)
    local core, host = require("fake_core")()
    H.boot(core, host)
    root:add(Seat({}))
    root:add(Lock({}))
    root:add(Screen({ width = 1000, height = 800 }))
    local wins = {}
    for i = 1, n do
        wins[i] = root:add(Window({ app_id = "w" .. i, width = 800, height = 600 }))
    end
    if xr_boot then
        root:add(Seat({}))
        host.external(host.ids("Seat")[2], "name", "xr")
    end
    H.load_shell()
    H.publish_seat(host, { screen = host.ids("Screen")[1] })
    H.start_shell(host)
    H.check(#host.reports == 0, "the shell reported an error: " .. table.concat(host.reports, " | "))
    return host, wins
end

-- The headset's seat arrives: the extension creates a seat and names it. Returns its node id.
local function xr_seat_arrives(host, name)
    local id = host.create("Seat")
    host.external(id, "name", name or "xr")
    host.frame(1, {})
    return id
end

-- The pure ring, from a shell loaded over a fresh fake core (the module needs the node classes to load).
local function ring()
    H.fresh()
    root:add(Seat({}))
    H.load_shell()
    return H.xr
end

local function room()
    return root:find("#room")
end
local function views_of(r)
    return r:all("> View")
end
local function view_of(w)
    for _, v in ipairs(views_of(room())) do
        if v.source == w then
            return v
        end
    end
end
-- The windows the room's Views show, in tree order: the order the extension submits them.
local function shown_order(r)
    local out = {}
    for _, v in ipairs(views_of(r)) do
        out[#out + 1] = v.source
    end
    return out
end

return {
    {
        "the ring puts one panel straight ahead at eye height, and a focused one is larger and closer",
        function()
            local poses, order = ring().arrange({ { id = "a", width = 800, height = 600 } }, nil)
            pose_is(poses[1], { 0, 1400, -1600, 0, 0.75, false, 0 }, "unfocused")
            same_list(order, { 1 }, "order")
            poses, order = ring().arrange({ { id = "a", width = 800, height = 600 } }, "a")
            pose_is(poses[1], { 0, 1400, -1500, 0, 0.8624999999999999, true, 0 }, "focused")
            same_list(order, { 1 }, "order")
        end,
    },
    {
        "the ring has no panels for no views",
        function()
            local poses, order = ring().arrange({}, nil)
            H.check(#poses == 0 and #order == 0, "nothing to place")
        end,
    },
    {
        "three panels form a flat arc with the focused one at the front and its neighbours turned in",
        function()
            local poses, order = ring().arrange(panels(3, 800, 600), 2)
            pose_is(
                poses[1],
                { -703.8364939904852, 1400, -1436.8765394866678, 0.45548487755982986, 0.75, false, 0 },
                "left"
            )
            pose_is(poses[2], { 0, 1400, -1500, 0, 0.8624999999999999, true, 0 }, "front")
            pose_is(
                poses[3],
                { 703.8364939904852, 1400, -1436.8765394866678, -0.45548487755982986, 0.75, false, 0 },
                "right"
            )
            same_list(order, { 1, 3, 2 }, "the focused panel draws last")
        end,
    },
    {
        "the focus is a carousel: the focused panel's neighbours in creation order wrap round it",
        function()
            local poses, order = ring().arrange(panels(3, 800, 600), 1)
            pose_is(poses[1], { 0, 1400, -1500, 0, 0.8624999999999999, true, 0 }, "front")
            pose_is(
                poses[2],
                { 703.8364939904852, 1400, -1436.8765394866678, -0.45548487755982986, 0.75, false, 0 },
                "right"
            )
            pose_is(
                poses[3],
                { -703.8364939904852, 1400, -1436.8765394866678, 0.45548487755982986, 0.75, false, 0 },
                "left"
            )
            same_list(order, { 2, 3, 1 }, "order")
        end,
    },
    {
        "a focus that names no view puts the first view at the front, unenlarged",
        function()
            local poses = ring().arrange(panels(3, 800, 600), "nobody")
            pose_is(poses[1], { 0, 1400, -1600, 0, 0.75, false, 0 }, "front")
        end,
    },
    {
        "panel spacing follows each panel's width",
        function()
            local poses = ring().arrange(
                { { id = "a", width = 1920, height = 1080 }, { id = "b", width = 800, height = 600 } },
                "b"
            )
            pose_is(
                poses[1],
                { 1052.5893657824845, 1400, -1205.0127082490155, -0.7179848775598299, 0.75, false, 0 },
                "wide"
            )
            pose_is(poses[2], { 0, 1400, -1500, 0, 0.8624999999999999, true, 0 }, "front")
        end,
    },
    {
        "twelve wide panels shrink together to fit 150 degrees to either side",
        function()
            local poses, order = ring().arrange(panels(12, 1920, 1080), 1)
            local scale = 0.291994247481941
            pose_is(poses[1], { 0, 1400, -1500, 0, 0.3357933846042322, true, 0 }, "front")
            pose_is(
                poses[2],
                { 665.5858301153864, 1400, -1454.98986345253, -0.4290324568115338, scale, false, 0 },
                "first right"
            )
            pose_is(
                poses[7],
                { 1029.27333286744, 1400, 1224.9883290251998, -2.442797329502329, scale, false, 0 },
                "last right"
            )
            pose_is(
                poses[8],
                { -1427.0537501096674, 1400, 723.5451570551317, 2.0400443549641705, scale, false, 0 },
                "last left"
            )
            pose_is(
                poses[12],
                { -665.5858301153864, 1400, -1454.98986345253, 0.4290324568115338, scale, false, 0 },
                "first left"
            )
            same_list(order, { 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 1 }, "order")
            for i = 2, 12 do -- every panel stays on the ring and clear of its neighbours' edges
                near(math.sqrt(poses[i].x ^ 2 + poses[i].z ^ 2), 1600, "radius of panel " .. i)
                near(math.atan(poses[i].x, -poses[i].z), -poses[i].rotation[2], "panel " .. i .. " faces the viewer")
                H.check(
                    math.abs(poses[i].rotation[2]) <= math.rad(150) + 1e-9,
                    "panel " .. i .. " is within 150 degrees"
                )
            end
        end,
    },
    {
        "panels past the twelfth stack behind the ring, and a focused one past it takes the ring's last place",
        function()
            local poses, order = ring().arrange(panels(14, 800, 600), 14)
            local scale = 0.7007861939566585
            pose_is(
                poses[14],
                { 0, 1400, -1500, 0, 0.8059041230501574, true, 0 },
                "the focused extra moves onto the ring"
            )
            pose_is(
                poses[1],
                { 665.5858301153864, 1400, -1454.98986345253, -0.4290324568115338, scale, false, 0 },
                "ring"
            )
            pose_is(
                poses[13],
                { 769.5836160709155, 1400, -1682.332029616988, -0.4290324568115338, scale, false, 1 },
                "behind view 1"
            )
            pose_is(
                poses[12],
                { 1367.400006471674, 1400, -1246.08074469565, -0.831785431349693, scale, false, 1 },
                "behind view 2"
            )
            same_list(order, { 13, 12, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 14 }, "stacked panels draw first")
        end,
    },
    {
        "a second tier of stacked panels sits further back and draws before the first",
        function()
            local poses, order = ring().arrange(panels(25, 800, 600), nil)
            near(math.sqrt(poses[13].x ^ 2 + poses[13].z ^ 2), 1850, "tier one is 250 mm behind the ring")
            near(math.sqrt(poses[25].x ^ 2 + poses[25].z ^ 2), 2100, "tier two is 500 mm behind the ring")
            H.check(poses[25].behind == 2 and poses[13].behind == 1 and poses[1].behind == 0, "tiers")
            H.check(order[1] == 25 and order[2] == 13, "the farthest tier draws first")
        end,
    },
    {
        "the stock rules accept xr as a boolean and refuse anything else",
        function()
            ring()
            local rules = require("gnoblin.shell.rules")
            local compiled = rules.compile({ { match = { app_id = "^a$" }, xr = false } })
            H.check(rules.effects(compiled, { app_id = "a" }).xr == false, "the rule says xr = false")
            H.check(not pcall(rules.compile, { { match = { app_id = "a" }, xr = "no" } }), "a string is refused")
        end,
    },
    {
        "no room exists until the seat named xr arrives, and another seat does not build one",
        function()
            local host = shell(2)
            H.check(room() == nil, "no room at start")
            xr_seat_arrives(host, "seat1")
            H.check(room() == nil, "a seat with another name builds no room")
        end,
    },
    {
        "the seat named xr builds the room under root: a stereo head camera in millimetres, and a View per window",
        function()
            local host, wins = shell(2)
            xr_seat_arrives(host)
            local r = room()
            H.check(r ~= nil and r.parent == root, "the room is a child of root")
            H.check(r.name == "room", "named room")
            H.check(#host.reports == 0, "reported: " .. table.concat(host.reports, " | "))
            near(r.y, -1400, "the room's floor is an eye height below the viewer")
            local head = r:find("> Camera")
            H.check(head ~= nil and head.name == "head", "a camera named head")
            H.check(head.projection == "stereo", "the head camera is stereo")
            near(head.metres, 0.001, "the room is in millimetres")
            H.check(r.children[1] == head, "the head camera comes first")
            H.check(#views_of(r) == 2, "one View per window")
            for i, w in ipairs(wins) do
                local v = view_of(w)
                H.check(v ~= nil, "window " .. i .. " has a View")
                near(v.width, 800, "view width")
                near(v.height, 600, "view height")
                H.check(v.visible and v.input, "the View is visible and takes input")
                near(v.opacity, 1, "view opacity")
            end
        end,
    },
    {
        "a seat named xr that is in the boot snapshot builds the room when the shell starts",
        function()
            local _, wins = shell(2, true)
            H.check(room() ~= nil, "the room exists after the first frame")
            H.check(#views_of(room()) == 2 and view_of(wins[1]) ~= nil and view_of(wins[2]) ~= nil, "a View per window")
        end,
    },
    {
        "the room is not built twice, and a room that is already in the tree is left alone",
        function()
            local host = shell(1)
            local mine = root:add(Node({ name = "room" }))
            xr_seat_arrives(host)
            H.check(#root:all("#room") == 1 and room() == mine, "the existing room stays")
            H.check(#mine.children == 0, "and gets no head camera or Views")
        end,
    },
    {
        "destroying the seat tears the room down, and the windows stay on their workspaces",
        function()
            local host, wins = shell(2)
            local id = xr_seat_arrives(host)
            H.check(room() ~= nil, "built")
            host.close(id)
            host.frame(2, {})
            H.check(room() == nil, "the room is gone")
            H.check(#host.ids("View") == 0 and #host.ids("Camera") == 0, "no View or camera is left")
            for i, w in ipairs(wins) do
                H.check(w.parent:is(Workspace) and w.visible, "window " .. i .. " is still on its workspace")
            end
            H.check(#host.reports == 0, "reported: " .. table.concat(host.reports, " | "))
            xr_seat_arrives(host) -- a new xr seat builds a new room
            H.check(room() ~= nil and #views_of(room()) == 2, "the next xr seat builds the room again")
        end,
    },
    {
        "a window that opens while the room exists gets a View, and one that opens before the room does not",
        function()
            local host = shell(1)
            local late = root:add(Window({ app_id = "late", width = 640, height = 480 }))
            host.frame(1, {})
            xr_seat_arrives(host)
            H.check(#views_of(room()) == 2, "both windows have a View after the room is built")
            local newer = root:add(Window({ app_id = "newer", width = 320, height = 240 }))
            host.frame(2, {})
            local v = view_of(newer)
            H.check(v ~= nil and v.source == newer, "the new window has a View whose source is the window")
            near(v.width, 320, "width")
            near(v.height, 240, "height")
            H.check(shown_order(room())[1] ~= nil, "the room lists its Views")
            H.check(#views_of(room()) == 3, "three Views")
            H.check(late.parent:is(Workspace), "the window itself stays on its workspace")
        end,
    },
    {
        "a View follows its window's size",
        function()
            local host, wins = shell(2)
            xr_seat_arrives(host)
            host.external(host.ids("Window")[1], "width", 1200)
            host.external(host.ids("Window")[1], "height", 700)
            host.frame(2, {})
            local v = view_of(wins[1])
            near(v.width, 1200, "width follows")
            near(v.height, 700, "height follows")
            near(view_of(wins[2]).width, 800, "the other View is unchanged")
        end,
    },
    {
        "a destroyed window's View is removed, whether the compositor or the shell ended it",
        function()
            local host, wins = shell(3)
            xr_seat_arrives(host)
            H.check(#views_of(room()) == 3, "three Views")
            host.close(host.ids("Window")[1]) -- the compositor ends the window
            host.frame(2, {})
            H.check(#views_of(room()) == 2 and view_of(wins[1]) == nil, "the compositor's destroy removes the View")
            wins[2]:destroy() -- the shell ends it, as Window:vanish does
            host.frame(3, {})
            H.check(#views_of(room()) == 1 and view_of(wins[3]) ~= nil, "a shell destroy removes the View")
            H.check(#host.reports == 0, "reported: " .. table.concat(host.reports, " | "))
        end,
    },
    {
        "a minimised window's View is hidden and leaves the ring until the window is shown again",
        function()
            local host, wins = shell(3)
            xr_seat_arrives(host)
            root:find(Seat).focus = wins[1]
            host.frame(2, {})
            local before = view_of(wins[3]).x
            host.external(host.ids("Window")[2], "minimised", true)
            host.frame(3, {})
            local hidden = view_of(wins[2])
            H.check(hidden ~= nil and hidden.visible == false, "the View stays and is hidden")
            H.check(views_of(room())[1] == hidden, "a hidden View lists first: it draws nothing")
            H.check(view_of(wins[3]).x ~= before, "the other panels close the gap")
            host.external(host.ids("Window")[2], "minimised", false)
            host.frame(4, {})
            H.check(hidden.visible == true, "the View is visible again")
            near(view_of(wins[3]).x, before, "and the ring is as it was")
        end,
    },
    {
        "a window rule with xr = false keeps the window out of the room, and dropping the rule brings it in",
        function()
            local host, wins = shell(0)
            root:find(Seat).data.window_rules = { { match = { title = "^private" }, xr = false } }
            local a = root:add(Window({ app_id = "a", title = "public", width = 800, height = 600 }))
            local b = root:add(Window({ app_id = "b", title = "private notes", width = 800, height = 600 }))
            host.frame(1, {})
            xr_seat_arrives(host)
            H.check(view_of(a) ~= nil, "an unmatched window has a View")
            H.check(view_of(b) == nil, "a matched window has none")
            H.check(b.data.xr == false, "the rule's value is on the window")
            local c = root:add(Window({ app_id = "c", title = "private mail", width = 800, height = 600 }))
            host.frame(2, {})
            H.check(view_of(c) == nil, "a window that opens later is held out too")
            host.external(host.ids("Window")[2], "title", "notes") -- the title no longer matches
            b:apply_rules()
            host.frame(3, {})
            H.check(view_of(b) ~= nil, "the window comes in when the rule stops matching")
            host.external(host.ids("Window")[1], "title", "private again")
            a:apply_rules()
            host.frame(4, {})
            H.check(view_of(a) == nil, "the window goes out when the rule starts matching")
            H.check(#host.reports == 0, "reported: " .. table.concat(host.reports, " | "))
        end,
    },
    {
        "a dialog has no View of its own: its parent's View shows it",
        function()
            local host, wins = shell(1)
            local dialog = root:add(Window({ app_id = "dialog", width = 300, height = 200, parent_window = wins[1] }))
            host.frame(1, {})
            xr_seat_arrives(host)
            H.check(#views_of(room()) == 1 and view_of(wins[1]) ~= nil, "only the parent has a View")
            H.check(view_of(dialog) == nil, "the dialog has none")
        end,
    },
    {
        "the focused window moves to the front and last in tree order, and the ring turns with the focus",
        function()
            local host, wins = shell(3)
            xr_seat_arrives(host)
            local seat = root:find(Seat)
            seat.focus = wins[2]
            host.frame(2, {})
            local front = view_of(wins[2])
            near(front.x, 0, "x")
            near(front.y, 1400, "y")
            near(front.z, -1500, "z")
            near(front.rotation[2], 0, "yaw")
            near(front.scale[1], 0.8625, "a focused panel is larger")
            H.check(shown_order(room())[3] == wins[2], "the focused View is last, so it draws on top")
            near(view_of(wins[1]).x, -703.8364939904852, "the previous window is on the left")
            near(view_of(wins[3]).x, 703.8364939904852, "the next window is on the right")
            seat.focus = wins[3]
            host.frame(3, {})
            near(view_of(wins[3]).x, 0, "the new focus is at the front")
            near(view_of(wins[3]).scale[1], 0.8625, "and larger")
            near(view_of(wins[2]).scale[1], 0.75, "the old focus is back to normal size")
            near(view_of(wins[2]).x, -703.8364939904852, "and on the left")
            H.check(shown_order(room())[3] == wins[3], "the new focus is last")
        end,
    },
    {
        "a View's tree order is back to front: panels stacked behind the ring draw first",
        function()
            local host, wins = shell(0)
            xr_seat_arrives(host)
            for i = 1, 14 do
                wins[i] = root:add(Window({ app_id = "w" .. i, width = 800, height = 600 }))
            end
            root:find(Seat).focus = nil
            host.frame(2, {})
            H.check(#views_of(room()) == 14, "14 Views")
            local order = shown_order(room())
            H.check(order[1] == wins[13] and order[2] == wins[14], "the stacked panels come first")
            local behind = view_of(wins[13])
            near(math.sqrt(behind.x ^ 2 + behind.z ^ 2), 1850, "and sit 250 mm behind the ring")
        end,
    },
    {
        "a frame in which no window changes writes no pose to any View",
        function()
            local host, wins = shell(3)
            xr_seat_arrives(host)
            host.external(host.ids("Window")[1], "width", 900)
            host.frame(2, {})
            local writes = 0
            for _, op in ipairs(host.batches[#host.batches]) do
                if op.op == "set" and (op.key == "y" or op.key == "z" or op.key == "rotation") then
                    writes = writes + 1
                end
            end
            H.check(writes > 0, "the resize moved the neighbours")
            host.external(host.ids("Window")[1], "title", "new title")
            host.frame(3, {})
            writes = 0
            for _, op in ipairs(host.batches[#host.batches]) do
                if
                    op.op == "set"
                    and (op.key == "x" or op.key == "y" or op.key == "z" or op.key == "rotation" or op.key == "scale")
                then
                    writes = writes + 1
                end
            end
            H.check(writes == 0, "a title change writes no pose")
        end,
    },
}
