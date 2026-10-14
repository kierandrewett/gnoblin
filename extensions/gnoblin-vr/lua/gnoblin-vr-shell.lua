-- Window arrangement policy for the Gnoblin VR extension, written entirely in Lua.
--
-- The compositor only stores anchors, surfaces and poses (gnoblin.spatial). Everything that decides
-- where a window goes, what a controller grab does and how the viewport pill moves lives here, so a
-- configuration can replace any of it.
--
--   local shell = require("gnoblin-vr-shell").new()
--   shell:dock("window:1", {width = 1.2, height = 0.7}, {x = -0.7, y = 0, z = 0})
--   shell:float("window:2", {x = 0.5, y = 1.4, z = -2}, {width = 1.6, height = 0.9})
--   local grab = shell:grab(ray)          -- ray = {origin = {...}, direction = {...}}
--   shell:drag(grab, ray)                 -- call as the controller moves
--   shell:release(grab)
local M = {}

local function vec(x, y, z)
    return { x = x, y = y, z = z }
end

local function normalise(v)
    local length = math.sqrt(v.x * v.x + v.y * v.y + v.z * v.z)
    if length < 1e-9 then
        return vec(0, 0, -1)
    end
    return vec(v.x / length, v.y / length, v.z / length)
end

--- Rotation about the vertical axis. Positive yaw turns a surface that faces +Z towards +X.
function M.yaw_rotation(yaw)
    return { x = 0, y = math.sin(yaw / 2), z = 0, w = math.cos(yaw / 2) }
end

--- Yaw that turns a surface at position so that its front faces point.
function M.yaw_towards(position, point)
    return math.atan(point.x - position.x, point.z - position.z)
end

--- Point at distance along a ray.
function M.point_on_ray(ray, distance)
    local direction = normalise(ray.direction)
    return vec(
        ray.origin.x + direction.x * distance,
        ray.origin.y + direction.y * distance,
        ray.origin.z + direction.z * distance
    )
end

-- Mutating calls return an Operation in the supervised runtime and the value itself from
-- gnoblinctl lua; this lets one policy run in both.
local function settle(result, callback)
    if type(result) == "table" and type(result.on_complete) == "function" then
        result:on_complete(function(value, err)
            callback(value, err)
        end)
    else
        callback(result, nil)
    end
end

local Shell = {}
Shell.__index = Shell

--- Create the policy. Options: world_anchor (default "world"), viewport_id (default "viewport"),
--- pill (initial pose position, default 0.0, 1.3, -1.5) and minimum_size (default 0.2 metres).
function M.new(options)
    options = options or {}
    local self = setmetatable({}, Shell)
    self.world_anchor = options.world_anchor or "world"
    self.viewport_id = options.viewport_id or "viewport"
    self.minimum_size = options.minimum_size or 0.2
    self.pill = options.pill or vec(0, 1.3, -1.5)
    self.pill_yaw = 0
    self.sizes = {}
    local identity = { position = vec(0, 0, 0), rotation = { x = 0, y = 0, z = 0, w = 1 } }
    gnoblin.spatial.anchor({ id = self.world_anchor, pose = identity })
    self:move_pill(self.pill, 0)
    return self
end

--- Move the viewport pill. Windows docked to the viewport move with it.
function Shell:move_pill(position, yaw)
    self.pill = position
    self.pill_yaw = yaw or self.pill_yaw
    return gnoblin.spatial.anchor({
        id = self.viewport_id,
        pose = { position = position, rotation = M.yaw_rotation(self.pill_yaw) },
    })
end

--- Drag the pill to where a controller ray points, keeping its distance and facing the user.
function Shell:drag_pill(ray, distance)
    distance = distance or 1.5
    local target = M.point_on_ray(ray, distance)
    return self:move_pill(target, M.yaw_towards(target, ray.origin))
end

--- Dock a window to the viewport at an offset from the pill, in the pill's own frame.
function Shell:dock(surface_id, size, offset)
    self.sizes[surface_id] = size
    return gnoblin.spatial.attach({
        surface_id = surface_id,
        viewport_id = self.viewport_id,
        pose = {
            position = offset or vec(0, 0, 0),
            rotation = { x = 0, y = 0, z = 0, w = 1 },
        },
        size = size,
    })
end

--- Place a window in free space, turned to face the user's current viewport.
function Shell:float(surface_id, position, size)
    self.sizes[surface_id] = size
    local yaw = M.yaw_towards(position, self.pill)
    return gnoblin.spatial.place({
        surface_id = surface_id,
        anchor_id = self.world_anchor,
        pose = { position = position, rotation = M.yaw_rotation(yaw) },
        size = size,
    })
end

--- Start grabbing the window under a controller ray. Returns nil when the ray hits nothing.
function Shell:grab(ray)
    local grabbed
    settle(gnoblin.spatial.hit({ origin = ray.origin, direction = ray.direction }), function(hit)
        if hit and hit.hit then
            grabbed = {
                surface_id = hit.surface_id,
                distance = hit.distance,
                u = hit.u,
                v = hit.v,
            }
        end
    end)
    if grabbed then
        gnoblin.spatial.raise({ surface_id = grabbed.surface_id })
    end
    return grabbed
end

--- Move a grabbed window with the controller. A docked window leaves the viewport and floats.
function Shell:drag(grab, ray)
    if not grab then
        return
    end
    local position = M.point_on_ray(ray, grab.distance)
    local size = self.sizes[grab.surface_id] or { width = 1, height = 1 }
    return self:float(grab.surface_id, position, size)
end

--- Resize a grabbed window by dragging a corner: the corner follows the ray in the window plane.
--- The window keeps its centre; width and height are clamped to minimum_size.
function Shell:resize(grab, size)
    if not grab then
        return
    end
    size = {
        width = math.max(self.minimum_size, size.width),
        height = math.max(self.minimum_size, size.height),
    }
    self.sizes[grab.surface_id] = size
    local current
    settle(gnoblin.spatial.surface({ surface_id = grab.surface_id }), function(value)
        current = value
    end)
    if not current then
        return
    end
    return gnoblin.spatial.place({
        surface_id = grab.surface_id,
        anchor_id = current.anchor_id or self.world_anchor,
        pose = current.pose,
        size = size,
    })
end

--- Where a controller's visible ray ends. Returns {origin, end_point, hit, surface_id, u, v, distance}:
--- end_point is the hit point when the ray meets a surface, otherwise max_length metres out
--- (default 5). The client draws the line and puts the cursor billboard at end_point.
function Shell:ray_segment(ray, max_length)
    max_length = max_length or 5
    local result = { origin = ray.origin, hit = false, distance = max_length }
    settle(gnoblin.spatial.hit({ origin = ray.origin, direction = ray.direction }), function(hit)
        if hit and hit.hit and hit.distance <= max_length then
            result.hit = true
            result.surface_id = hit.surface_id
            result.u, result.v = hit.u, hit.v
            result.distance = hit.distance
        end
    end)
    result.end_point = M.point_on_ray(ray, result.distance)
    return result
end

function Shell:release(grab)
    return nil
end

M.Shell = Shell
return M
