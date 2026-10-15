-- Pure policy tests: lua extensions/gnoblin-vr/tests/vr-desktop.test.lua
local handlers, calls, windows, now = {}, {}, {}, 0
local function record(name)
    return function(args)
        calls[#calls + 1] = { name = name, args = args }
        return args
    end
end
local function method_record(name)
    return function(_, args)
        calls[#calls + 1] = { name = name, args = args }
        return args
    end
end
gnoblin = {
    spatial = {
        anchor = record("anchor"),
        place = record("place"),
        attach = record("attach"),
        move = record("move"),
        raise = record("raise"),
        update_pose = record("update_pose"),
        surface = record("surface"),
        snapshot = record("snapshot"),
        hit = record("hit"),
        head = record("head"),
        point = record("point"),
        remove = record("remove"),
        environment = record("environment"),
    },
    windows = {
        list = function()
            return windows
        end,
    },
    outputs = {
        list = function()
            return { { kind = "xr", state = "running" } }
        end,
    },
    events = {
        on = function(name, callback)
            handlers[name] = callback
            return {}
        end,
    },
}
package.path = "extensions/gnoblin-vr/lua/?.lua;" .. package.path
local desktop = require("gnoblin-vr-desktop")
local function expect(condition, message)
    if not condition then
        error("FAIL: " .. message, 2)
    end
    print("PASS: " .. message)
end
local function near(a, b, epsilon)
    return math.abs(a - b) < (epsilon or 1e-6)
end
local function calls_named(name)
    local result = {}
    for _, call in ipairs(calls) do
        if call.name == name then
            result[#result + 1] = call
        end
    end
    return result
end
local function clear_calls()
    calls = {}
end

windows = {
    {
        id = "window:1",
        frame = { width = 1200, height = 800 },
        focused = true,
        resizable = true,
        resize = method_record("window_resize"),
    },
    {
        id = "window:2",
        frame = { width = 800, height = 600 },
        focused = false,
        resize = method_record("window_resize"),
    },
}
local policy = desktop.auto({
    now = function()
        return now
    end,
    resize_debounce = 0.2,
})
local placed = calls_named("place")
expect(#placed == 2, "auto mirrors every existing window when the headset is running")
expect(
    near(placed[1].args.size.width, 1.2) and near(placed[1].args.size.height, 0.8),
    "window pixel dimensions become metres at the default pixel density"
)
local arc_left = desktop.arc_pose(0)
local arc_right = desktop.arc_pose(1)
expect(
    near(math.sqrt(placed[1].args.pose.position.x ^ 2 + placed[1].args.pose.position.z ^ 2), 2),
    "arc placement keeps windows two metres ahead"
)
expect(
    near(arc_left.position.y, 1.45) and arc_right.position.x > arc_left.position.x,
    "arc poses use eye height and spread along the forward arc"
)
expect(#calls_named("raise") > 0, "focused window is raised in the spatial scene")

clear_calls()
local created = { id = "window:3", frame = { width = 1000, height = 500 }, resize = method_record("window_resize") }
handlers["gnoblin.window.created"]({ window = created })
expect(#calls_named("place") == 1 and policy.slots["window:3"] == 2, "new windows use the next free arc slot")
expect(calls_named("place")[1].args.pose.position.x < 0, "the next slot fans the arc to the opposite side")
handlers["gnoblin.window.closed"]({ window_id = "window:2" })
expect(#calls_named("remove") == 1, "closed windows are removed from the spatial scene")

clear_calls()
local head = { position = { x = 0, y = 1.5, z = 0 } }
local hand = {
    active = true,
    position = { x = 1, y = 1.5, z = 0 },
    rotation = { x = 0, y = 0, z = 0, w = 1 },
    grab = true,
    hit = { surface_id = "window:1", u = 0.5, v = 0.5, distance = 2 },
}
policy:input({ head = head, hands = { right = hand } })
hand.position = { x = 1.5, y = 1.6, z = -0.2 }
policy:input({ head = head, hands = { right = hand } })
local updates = calls_named("update_pose")
expect(
    #updates == 1 and near(updates[1].args.pose.position.x - policy.poses["window:1"].position.x, 0),
    "window grab follows the controller while maintaining its captured offset"
)
local drag_pose = policy.poses["window:1"]
local facing_yaw = desktop.yaw_towards(drag_pose.position, head.position)
expect(near(drag_pose.rotation.y, math.sin(facing_yaw / 2)), "dragged window continuously faces the head")
hand.grab = false
policy:input({ head = head, hands = { right = hand } })

clear_calls()
local resized = {
    active = true,
    position = { x = 1, y = 1.5, z = 0 },
    rotation = { x = 0, y = 0, z = 0, w = 1 },
    grab = true,
    hit = { surface_id = "window:1", u = 0.95, v = 0.95, distance = 2 },
}
policy:input({ head = head, hands = { right = resized } })
resized.position = { x = 1.8, y = 1.9, z = 0 }
policy:input({ head = head, hands = { right = resized } })
expect(
    #calls_named("place") > 0 and policy.sizes["window:1"].width > 1.2,
    "corner grab expands the spatial quad while preserving the opposite corner"
)
local test_size, test_pose = desktop.resize_corner(
    { width = 2, height = 1 },
    { position = { x = 0, y = 0, z = 0 }, rotation = { x = 0, y = 0, z = 0, w = 1 } },
    0.95,
    0.95,
    { x = 2, y = -1, z = 0 }
)
expect(
    near(test_pose.position.x - test_size.width / 2, -1) and near(test_pose.position.y + test_size.height / 2, 0.5),
    "resize math keeps the opposite corner fixed"
)
resized.grab = false
policy:input({ head = head, hands = { right = resized } })
expect(#calls_named("window_resize") == 1, "releasing a corner grab resizes the real window")

clear_calls()
local workspace_hand =
    { active = true, position = { x = 0, y = 1, z = 0 }, rotation = { x = 0, y = 0, z = 0, w = 1 }, grab = true }
policy:input({ head = head, hands = { left = workspace_hand } })
workspace_hand.position = { x = 0.4, y = 1.2, z = -0.3 }
policy:input({ head = head, hands = { left = workspace_hand } })
local anchors = calls_named("anchor")
expect(near(anchors[#anchors].args.pose.position.x, 0.4), "empty-ray grip translates the workspace anchor")
workspace_hand.rotation = { x = 0, y = math.sin(math.pi / 8), z = 0, w = math.cos(math.pi / 8) }
policy:input({ head = head, hands = { left = workspace_hand } })
anchors = calls_named("anchor")
expect(math.abs(anchors[#anchors].args.pose.rotation.y) > 0.1, "turning a workspace grip rotates the anchor")
workspace_hand.grab = false
policy:input({ head = head, hands = { left = workspace_hand } })

clear_calls()
local left = { active = true, grab = true, position = { x = -0.5, y = 1, z = 0 } }
local right = { active = true, grab = true, position = { x = 0.5, y = 1, z = 0 } }
policy:input({ head = head, hands = { left = left, right = right } })
right.position = { x = 0, y = 1.2, z = -1 }
policy:input({ head = head, hands = { left = left, right = right } })
anchors = calls_named("anchor")
expect(
    #anchors >= 2 and math.abs(anchors[#anchors].args.pose.rotation.y) > 0.1,
    "two-hand gesture rotates the shared workspace anchor"
)
expect(#calls_named("place") >= 1, "two-hand gesture scales window quads through Lua placements")

clear_calls()
local menu =
    { active = true, position = { x = 0, y = 1, z = 0 }, rotation = { x = 0, y = 0, z = 0, w = 1 }, menu = true }
policy:input({ head = head, hands = { left = menu } })
expect(calls_named("environment")[1].args.mode == "passthrough", "left menu toggles passthrough on")
menu.menu = false
policy:input({ head = head, hands = { left = menu } })
menu.menu = true
policy:input({ head = head, hands = { left = menu } })
expect(calls_named("environment")[2].args.mode == "opaque", "left menu toggles passthrough off")

clear_calls()
local pointer = {
    active = true,
    position = { x = 0, y = 1, z = 0 },
    rotation = { x = 0, y = 0, z = 0, w = 1 },
    select = true,
    scroll = { x = 0, y = -1 },
}
policy:input({ head = head, hands = { left = pointer } })
local points = calls_named("point")
expect(#points >= 2 and points[1].args.scroll.y == -1, "pointer hover and thumbstick scroll reach spatial.point")
expect(
    points[#points].args.button == "left" and points[#points].args.state == "press",
    "trigger press edge reaches the pointer"
)
pointer.select = false
policy:input({ head = head, hands = { left = pointer } })
points = calls_named("point")
expect(
    points[#points].args.button == "left" and points[#points].args.state == "release",
    "trigger release edge reaches the pointer"
)
local right_menu = {
    active = true,
    position = { x = 0, y = 1, z = 0 },
    rotation = { x = 0, y = 0, z = 0, w = 1 },
    menu = true,
}
policy:input({ head = head, hands = { right = right_menu } })
points = calls_named("point")
expect(
    points[#points].args.button == "right" and points[#points].args.state == "press",
    "right-hand menu press edge sends a right click"
)

print("ok")
