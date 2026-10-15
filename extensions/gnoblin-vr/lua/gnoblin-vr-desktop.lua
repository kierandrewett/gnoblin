-- Default Lua policy for mirroring and interacting with desktop windows in VR.
-- All placement and gesture decisions live here; Gnoblin's compositor stores the scene.
local M = {}

local IDENTITY = { x = 0, y = 0, z = 0, w = 1 }
local function vec(x, y, z)
    return { x = x, y = y, z = z }
end
local function copy(v)
    return { x = v.x, y = v.y, z = v.z }
end
local function length(v)
    return math.sqrt(v.x * v.x + v.y * v.y + v.z * v.z)
end
local function unit(v)
    local n = length(v)
    if n < 1e-9 then
        return vec(0, 0, -1)
    end
    return vec(v.x / n, v.y / n, v.z / n)
end
local function yaw_rotation(yaw)
    return { x = 0, y = math.sin(yaw / 2), z = 0, w = math.cos(yaw / 2) }
end
local function yaw_towards(position, point)
    return math.atan(point.x - position.x, point.z - position.z)
end
local function qrotate(q, v)
    local tx = 2 * (q.y * v.z - q.z * v.y)
    local ty = 2 * (q.z * v.x - q.x * v.z)
    local tz = 2 * (q.x * v.y - q.y * v.x)
    return vec(
        v.x + q.w * tx + q.y * tz - q.z * ty,
        v.y + q.w * ty + q.z * tx - q.x * tz,
        v.z + q.w * tz + q.x * ty - q.y * tx
    )
end
local function qmul(a, b)
    return {
        x = a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
        y = a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
        z = a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
        w = a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
    }
end
local function qinverse(q)
    return { x = -q.x, y = -q.y, z = -q.z, w = q.w }
end
local function local_to_world(anchor, position)
    local rotated = qrotate(anchor.rotation or IDENTITY, position)
    return vec(anchor.position.x + rotated.x, anchor.position.y + rotated.y, anchor.position.z + rotated.z)
end
local function world_to_local(anchor, position)
    return qrotate(
        qinverse(anchor.rotation or IDENTITY),
        vec(position.x - anchor.position.x, position.y - anchor.position.y, position.z - anchor.position.z)
    )
end
local function yaw_of(q)
    q = q or IDENTITY
    return math.atan(2 * (q.w * q.y + q.x * q.z), 1 - 2 * (q.y * q.y + q.x * q.x))
end
local function ray_for(hand)
    return { origin = hand.position, direction = qrotate(hand.rotation or IDENTITY, vec(0, 0, -1)) }
end
local function ray_point(ray, distance)
    local d = unit(ray.direction)
    return vec(ray.origin.x + d.x * distance, ray.origin.y + d.y * distance, ray.origin.z + d.z * distance)
end
---Pure geometry helpers, also useful to replace or test the default policy.
M.yaw_towards = yaw_towards
M.ray_for = ray_for
M.ray_point = ray_point

---Return an arc placement for a zero-based slot.
function M.arc_pose(slot, options)
    options = options or {}
    local radius = options.radius or 2
    local step = options.arc_step or math.rad(18)
    local arc_slot = slot == 0 and 0 or (slot % 2 == 1 and (slot + 1) / 2 or -slot / 2)
    local angle = (arc_slot - (options.arc_center or 0)) * step
    local position = vec(math.sin(angle) * radius, options.eye_height or 1.45, -math.cos(angle) * radius)
    return { position = position, rotation = yaw_rotation(yaw_towards(position, vec(0, position.y, 0))) }
end

---Compute a corner resize, anchored at the opposite corner; u/v range from 0 to 1.
function M.resize_corner(size, pose, u, v, hand_point, minimum)
    minimum = minimum or 0.2
    local right = (u < 0.5) and -1 or 1
    local down = (v < 0.5) and 1 or -1
    local corner_x = right * size.width / 2
    local corner_y = down * size.height / 2
    local opposite_x, opposite_y = -corner_x, -corner_y
    local q = pose.rotation or IDENTITY
    local anchor = qrotate(q, vec(opposite_x, opposite_y, 0))
    local origin = vec(pose.position.x + anchor.x, pose.position.y + anchor.y, pose.position.z + anchor.z)
    local local_delta = qrotate(
        { x = -q.x, y = -q.y, z = -q.z, w = q.w },
        vec(hand_point.x - origin.x, hand_point.y - origin.y, hand_point.z - origin.z)
    )
    local width = math.max(minimum, math.abs(local_delta.x))
    local height = math.max(minimum, math.abs(local_delta.y))
    local center_local = vec(opposite_x + right * width / 2, opposite_y + down * height / 2, 0)
    local center = qrotate(q, center_local)
    return { width = width, height = height }, {
        position = vec(pose.position.x + center.x, pose.position.y + center.y, pose.position.z + center.z),
        rotation = q,
    }
end

local Policy = {}
Policy.__index = Policy

function M.new(options)
    local self = setmetatable({}, Policy)
    self.opts = options or {}
    local function positive(name, value)
        assert(type(value) == "number" and value > 0, name .. " must be a positive number")
    end
    self.anchor_id = self.opts.world_anchor or "world"
    self.ppm = self.opts.pixels_per_metre or 1000
    positive("pixels_per_metre", self.ppm)
    positive("radius", self.opts.radius or 2)
    positive("arc_step", self.opts.arc_step or math.rad(18))
    positive("minimum_size", self.opts.minimum_size or 0.2)
    local minimum_scale = self.opts.minimum_workspace_scale or 0.25
    local maximum_scale = self.opts.maximum_workspace_scale or 4
    positive("minimum_workspace_scale", minimum_scale)
    assert(
        type(maximum_scale) == "number" and maximum_scale >= minimum_scale,
        "maximum_workspace_scale must be at least minimum_workspace_scale"
    )
    assert(type(self.opts.eye_height or 1.45) == "number", "eye_height must be a number")
    assert(type(self.opts.arc_center or 0) == "number", "arc_center must be a number")
    assert(
        type(self.opts.resize_debounce or 0.25) == "number" and (self.opts.resize_debounce or 0.25) >= 0,
        "resize_debounce must be a non-negative number"
    )
    self.windows, self.poses, self.sizes, self.slots = {}, {}, {}, {}
    self.slot_count = 0
    self.buttons, self.grips = {}, {}
    self.environment_mode = self.opts.environment or "opaque"
    assert(
        self.environment_mode == "opaque" or self.environment_mode == "passthrough",
        "environment must be \"opaque\" or \"passthrough\""
    )
    self.anchor_pose = { position = vec(0, 0, 0), rotation = IDENTITY }
    self.workspace_grab = nil
    self.two_hand_grab = nil
    self.resize_pending = {}
    self.resize_delay = self.opts.resize_debounce or 0.25
    self:_anchor(self.anchor_pose)
    return self
end

function Policy:_anchor(pose)
    self.anchor_pose = pose
    return gnoblin.spatial.anchor({ id = self.anchor_id, pose = pose })
end

function Policy:_slot(id)
    if self.slots[id] then
        return self.slots[id]
    end
    local used = {}
    for _, slot in pairs(self.slots) do
        used[slot] = true
    end
    local slot = 0
    while used[slot] do
        slot = slot + 1
    end
    self.slots[id] = slot
    self.slot_count = math.max(self.slot_count, slot + 1)
    return slot
end

function Policy:place(window)
    if not window or not window.id or window.minimized then
        return
    end
    self.windows[window.id] = window
    local frame = window.frame or { width = 900, height = 600 }
    local size = { width = frame.width / self.ppm, height = frame.height / self.ppm }
    local pose = M.arc_pose(self:_slot(window.id), self.opts)
    self.sizes[window.id], self.poses[window.id] = size, pose
    return gnoblin.spatial.place({ surface_id = window.id, anchor_id = self.anchor_id, pose = pose, size = size })
end

function Policy:remove(id)
    self.windows[id], self.poses[id], self.sizes[id], self.slots[id] = nil, nil, nil, nil
    self.resize_pending[id] = nil
    return gnoblin.spatial.remove({ surface_id = id })
end

function Policy:reconcile()
    local list = gnoblin.windows.list()
    local live = {}
    for _, window in ipairs(list or {}) do
        live[window.id] = true
        if not self.windows[window.id] then
            self:place(window)
        else
            self.windows[window.id] = window
        end
    end
    for id in pairs(self.windows) do
        if not live[id] then
            self:remove(id)
        end
    end
    for _, window in ipairs(list or {}) do
        if window.focused then
            gnoblin.spatial.raise({ surface_id = window.id })
        end
    end
end

function Policy:_point(hand_name, hand)
    if not hand or not hand.active then
        return
    end
    local ray = ray_for(hand)
    local previous = self.buttons[hand_name] or { left = false, right = false }
    local select_now = hand.select == true
    local menu_now = hand_name == "right" and hand.menu == true
    local function call(button, now, before)
        if now ~= before then
            gnoblin.spatial.point({
                origin = ray.origin,
                direction = ray.direction,
                button = button,
                state = now and "press" or "release",
            })
        end
    end
    gnoblin.spatial.point({
        origin = ray.origin,
        direction = ray.direction,
        scroll = hand.scroll and (hand.scroll.x ~= 0 or hand.scroll.y ~= 0) and hand.scroll or nil,
    })
    call("left", select_now, previous.left)
    call("right", menu_now, previous.right)
    previous.left, previous.right = select_now, menu_now
    self.buttons[hand_name] = previous
end

local function corner_hit(hit)
    if not hit or hit.u == nil or hit.v == nil then
        return nil
    end
    local horizontal = hit.u < 0.5 and "left" or "right"
    local vertical = hit.v < 0.5 and "top" or "bottom"
    if math.min(hit.u, 1 - hit.u) <= 0.12 and math.min(hit.v, 1 - hit.v) <= 0.12 then
        return horizontal .. "_" .. vertical
    end
end

function Policy:_start_grip(name, hand)
    local hit = hand.hit
    if hit and hit.surface_id and self.poses[hit.surface_id] then
        gnoblin.spatial.raise({ surface_id = hit.surface_id })
        local corner = corner_hit(hit)
        local pose, size = self.poses[hit.surface_id], self.sizes[hit.surface_id]
        if corner and self.windows[hit.surface_id].resizable ~= false then
            self.grips[name] = {
                kind = "resize",
                id = hit.surface_id,
                corner = corner,
                hit = hit,
                start_pose = pose,
                start_size = size,
            }
        else
            local world_position = local_to_world(self.anchor_pose, pose.position)
            self.grips[name] = {
                kind = "window",
                id = hit.surface_id,
                offset = vec(
                    world_position.x - hand.position.x,
                    world_position.y - hand.position.y,
                    world_position.z - hand.position.z
                ),
                distance = hit.distance or 1.5,
            }
        end
        return
    end
    self.grips[name] = {
        kind = "workspace",
        hand = copy(hand.position),
        hand_yaw = yaw_of(hand.rotation),
        anchor = self.anchor_pose,
    }
    self.workspace_grab = self.grips[name]
end

function Policy:_drag(name, hand, head)
    local grab = self.grips[name]
    if not grab then
        return
    end
    if grab.kind == "window" then
        local world_position =
            vec(hand.position.x + grab.offset.x, hand.position.y + grab.offset.y, hand.position.z + grab.offset.z)
        local position = world_to_local(self.anchor_pose, world_position)
        local world_rotation = yaw_rotation(yaw_towards(world_position, head.position))
        local local_rotation = qmul(qinverse(self.anchor_pose.rotation or IDENTITY), world_rotation)
        local pose = { position = position, rotation = local_rotation }
        self.poses[grab.id] = pose
        gnoblin.spatial.update_pose({ surface_id = grab.id, pose = pose })
    elseif grab.kind == "resize" then
        local hit = grab.hit
        local local_hand = world_to_local(self.anchor_pose, hand.position)
        local size, pose =
            M.resize_corner(grab.start_size, grab.start_pose, hit.u, hit.v, local_hand, self.opts.minimum_size or 0.2)
        self.sizes[grab.id], self.poses[grab.id] = size, pose
        gnoblin.spatial.update_pose({ surface_id = grab.id, pose = pose })
        gnoblin.spatial.place({ surface_id = grab.id, anchor_id = self.anchor_id, pose = pose, size = size })
        self.resize_pending[grab.id] = { size = size, since = (self.opts.now and self.opts.now() or os.clock()) }
    elseif grab.kind == "workspace" then
        local start = grab.hand
        local delta = vec(hand.position.x - start.x, hand.position.y - start.y, hand.position.z - start.z)
        local yaw = yaw_of(hand.rotation) - grab.hand_yaw
        local turn = yaw_rotation(yaw)
        local base = grab.anchor.rotation or IDENTITY
        local rotation = qmul(turn, base)
        self:_anchor({
            position = vec(
                grab.anchor.position.x + delta.x,
                grab.anchor.position.y + delta.y,
                grab.anchor.position.z + delta.z
            ),
            rotation = rotation,
        })
    end
end

function Policy:_two_hand(left, right)
    if not (left and right and left.active and right.active and left.grab and right.grab) then
        self.two_hand_grab = nil
        return
    end
    local center = vec(
        (left.position.x + right.position.x) / 2,
        (left.position.y + right.position.y) / 2,
        (left.position.z + right.position.z) / 2
    )
    local delta = vec(right.position.x - left.position.x, 0, right.position.z - left.position.z)
    local distance = math.max(0.01, length(delta))
    local angle = math.atan(delta.x, -delta.z)
    if not self.two_hand_grab then
        local sizes, poses = {}, {}
        for id, size in pairs(self.sizes) do
            sizes[id] = { width = size.width, height = size.height }
        end
        for id, pose in pairs(self.poses) do
            poses[id] = { position = copy(pose.position), rotation = pose.rotation }
        end
        self.two_hand_grab = {
            center = center,
            distance = distance,
            angle = angle,
            anchor = self.anchor_pose,
            sizes = sizes,
            poses = poses,
        }
        return
    end
    local initial = self.two_hand_grab
    local scale = math.max(
        self.opts.minimum_workspace_scale or 0.25,
        math.min(self.opts.maximum_workspace_scale or 4, distance / initial.distance)
    )
    local yaw = angle - initial.angle
    local q = yaw_rotation(yaw)
    local initial_center = initial.center
    local translation = vec(center.x - initial_center.x, center.y - initial_center.y, center.z - initial_center.z)
    local anchor_position = vec(
        initial.anchor.position.x + translation.x,
        initial.anchor.position.y + translation.y,
        initial.anchor.position.z + translation.z
    )
    local base_rotation = initial.anchor.rotation or IDENTITY
    local combined = {
        x = q.w * base_rotation.x + q.y * base_rotation.z,
        y = q.w * base_rotation.y + q.y * base_rotation.w,
        z = q.w * base_rotation.z - q.y * base_rotation.x,
        w = q.w * base_rotation.w - q.y * base_rotation.y,
    }
    self:_anchor({ position = anchor_position, rotation = combined })
    for id, initial_size in pairs(initial.sizes) do
        local size = {
            width = math.max(self.opts.minimum_size or 0.2, initial_size.width * scale),
            height = math.max(self.opts.minimum_size or 0.2, initial_size.height * scale),
        }
        local original = initial.poses[id]
        local pose = {
            position = vec(original.position.x * scale, original.position.y * scale, original.position.z * scale),
            rotation = original.rotation,
        }
        self.sizes[id], self.poses[id] = size, pose
        gnoblin.spatial.place({ surface_id = id, anchor_id = self.anchor_id, pose = pose, size = size })
    end
end

function Policy:input(event)
    local hands = event.hands or {}
    self:_point("left", hands.left)
    self:_point("right", hands.right)
    for _, name in ipairs({ "left", "right" }) do
        local hand = hands[name]
        if hand and hand.active then
            local held = self.grips[name]
            if hand.grab and not held then
                self:_start_grip(name, hand)
            elseif hand.grab and held then
                self:_drag(name, hand, event.head)
            elseif not hand.grab and held then
                if held.kind == "resize" then
                    self:_commit_resize(held.id)
                end
                self.grips[name] = nil
                if held.kind == "workspace" then
                    self.workspace_grab = nil
                end
            end
        elseif self.grips[name] then
            self.grips[name] = nil
        end
    end
    self:_two_hand(hands.left, hands.right)
    if hands.left and hands.left.active and hands.left.menu and not self.left_menu then
        self.environment_mode = self.environment_mode == "opaque" and "passthrough" or "opaque"
        gnoblin.spatial.environment({ mode = self.environment_mode })
    end
    self.left_menu = hands.left and hands.left.menu or false
    self:_flush_resizes()
end

function Policy:_commit_resize(id)
    local pending, window = self.resize_pending[id], self.windows[id]
    if not pending or not window then
        return
    end
    local size = {
        width = math.max(1, math.floor(pending.size.width * self.ppm + 0.5)),
        height = math.max(1, math.floor(pending.size.height * self.ppm + 0.5)),
    }
    window:resize(size)
    self.resize_pending[id] = nil
end

function Policy:_flush_resizes()
    local now = self.opts.now and self.opts.now() or os.clock()
    for id, pending in pairs(self.resize_pending) do
        if now - pending.since >= self.resize_delay then
            self:_commit_resize(id)
        end
    end
end

---Enable the default desktop policy. Every option may be overridden by passing a replacement
---function for the named policy method, or by constructing M.new(opts) and managing events yourself.
function M.auto(options)
    options = options or {}
    local policy = M.new(options)
    for _, method in ipairs({ "place", "remove", "reconcile", "input", "_point", "_start_grip", "_drag" }) do
        if options[method] ~= nil then
            assert(type(options[method]) == "function", method .. " override must be a function")
            policy[method] = options[method]
        end
    end
    local function on(name, callback)
        return gnoblin.events.on(name, callback)
    end
    on("gnoblin.vr.headset-changed", function(event)
        local running = false
        for _, headset in ipairs(event.headsets or {}) do
            if headset.state == "running" then
                running = true
            end
        end
        policy.running = running
        if running then
            gnoblin.spatial.environment({ mode = policy.environment_mode })
            policy:reconcile()
        else
            for id in pairs(policy.windows) do
                policy:remove(id)
            end
        end
    end)
    on("gnoblin.window.created", function(event)
        if policy.running then
            policy:place(event.window)
        end
    end)
    on("gnoblin.window.closed", function(event)
        policy:remove(event.window_id)
    end)
    on("gnoblin.window.focused", function(event)
        if policy.running and event.window_id then
            gnoblin.spatial.raise({ surface_id = event.window_id })
        end
    end)
    on("gnoblin.window.changed", function(event)
        local window = event.window
        if not window then
            return
        end
        if window.minimized then
            if policy.windows[window.id] then
                policy:remove(window.id)
            end
        elseif policy.running and not policy.windows[window.id] then
            policy:place(window)
        elseif policy.windows[window.id] then
            policy.windows[window.id] = window
            local frame = window.frame
            if frame then
                local size = { width = frame.width / policy.ppm, height = frame.height / policy.ppm }
                if size.width ~= policy.sizes[window.id].width or size.height ~= policy.sizes[window.id].height then
                    policy.sizes[window.id] = size
                    gnoblin.spatial.place({
                        surface_id = window.id,
                        anchor_id = policy.anchor_id,
                        pose = policy.poses[window.id],
                        size = size,
                    })
                end
            end
        end
    end)
    on("gnoblin.vr.input", function(event)
        if policy.running then
            policy:input(event)
        end
    end)
    policy.running = false
    for _, output in ipairs(gnoblin.outputs.list()) do
        if output.kind == "xr" then
            if output.state == "running" then
                policy.running = true
            end
        end
    end
    if policy.running then
        gnoblin.spatial.environment({ mode = policy.environment_mode })
        policy:reconcile()
    end
    return policy
end

return M
