-- carousel3d: windows on a ring, cover-flow style. The focused window stands at the
-- front, nearest the eye and facing it. The others sit around the ring, each turned
-- to follow the ring's tangent, and fade with distance from the front.
--
-- Geometry is 3D: every placement uses a transform with a position, a rotation about
-- the Y axis and unit scale. Depth needs the compositor's depth pass (D18), and the
-- camera is a perspective camera with its eye at the centre of the area, looking down
-- -z. The ring's radius is set by the number of windows, so cards never overlap in
-- the ring's plane.
--
-- arrange: positions all windows; every window is visible. Windows back on the ring
--          fade with opacity.
-- on_request: declines every geometry request.
-- state (ctx.state): shown = id of the front window, focus_seen = last focus seen.
--          When ctx.focus changes the front moves to it. A rotate command moves the
--          front and the compositor should focus the window it returns.
-- options:
--   fov_degrees  vertical field of view of the camera, strictly between 0 and 180
--   card_width   card width as a fraction of the area width, 0..1
--   card_height  card height as a fraction of the area height, 0..1
-- commands:
--   rotate(step): move the front by step cards (default 1, negative goes back).
--                 Returns the id of the new front window, or nil with no windows.
--
-- Usage: select it by name, "carousel3d", in the layout settings.

local M = {}
M.kind = "tiling"
M.extent = "unbounded"

M.options = {
    fov_degrees = 40,
    card_width = 0.36,
    card_height = 0.6,
}

local TAU = 2 * math.pi

local function index_of(ctx, id)
    if id == nil then
        return nil
    end
    for i, window in ipairs(ctx.windows) do
        if window.id == id then
            return i
        end
    end
    return nil
end

-- Index of the front window, following focus when it changes.
local function front_index(ctx)
    local state = ctx.state
    if ctx.focus ~= nil and ctx.focus ~= state.focus_seen then
        local index = index_of(ctx, ctx.focus)
        if index ~= nil then
            state.focus_seen = ctx.focus
            state.shown = ctx.focus
        end
    end
    local index = index_of(ctx, state.shown)
    if index == nil then
        index = 1
        state.shown = ctx.windows[1] and ctx.windows[1].id or nil
    end
    return index
end

-- Signed slot offset of window i from the front, in (-n/2, n/2].
local function slot_offset(i, front, n)
    local d = (i - front) % n
    if 2 * d > n then
        d = d - n
    end
    return d
end

function M.arrange(ctx)
    local n = #ctx.windows
    local placements = {}
    if n == 0 then
        return { placements = placements }
    end

    local area = ctx.area
    local fov = ctx.params.fov_degrees
    local half_fov = math.rad(fov) / 2
    local cx = area.x + area.width / 2
    local cy = area.y + area.height / 2

    local W = math.max(1, area.width * ctx.params.card_width)
    local H = math.max(1, area.height * ctx.params.card_height)

    -- Ring radius: large enough that n cards do not overlap along the ring.
    local R = math.max(W * 1.25, n * W * 1.25 / TAU)
    -- Distance from the eye to the front card, so that the front card's height
    -- fills the card_height fraction of the screen.
    local F = math.max(1, (area.height / 2) / math.tan(half_fov))
    local zc = -(F + R)

    local front = front_index(ctx)
    for i, window in ipairs(ctx.windows) do
        local a = slot_offset(i, front, n) * TAU / n
        local cos, sin = math.cos(a), math.sin(a)
        -- Card centre on the ring.
        local centre_x = cx + R * sin
        local centre_z = zc + R * cos
        -- The transform pivots at the card's top-left corner; move it so the card
        -- turns about its own centre.
        placements[#placements + 1] = {
            window = window.id,
            width = W,
            height = H,
            transform = {
                position = {
                    x = centre_x - (W / 2) * cos,
                    y = cy - H / 2,
                    z = centre_z + (W / 2) * sin,
                },
                rotation = { x = 0, y = math.deg(a), z = 0 },
                scale = { x = 1, y = 1, z = 1 },
            },
            visible = true,
            opacity = 0.4 + 0.6 * (1 + cos) / 2,
        }
    end

    local near = 1
    local far = 1000000
    return {
        placements = placements,
        camera = {
            kind = "perspective",
            fov_degrees = fov,
            near = near,
            far = far,
            pose = { position = { x = cx, y = cy, z = 0 } },
        },
    }
end

function M.on_request(_ctx)
    return nil
end

M.command = {
    rotate = function(ctx, args)
        args = args or {}
        local n = #ctx.windows
        if n == 0 then
            return nil
        end
        local step = args.step or 1
        local front = front_index(ctx)
        local nxt = (front - 1 + step) % n + 1
        ctx.state.shown = ctx.windows[nxt].id
        ctx.state.focus_seen = ctx.state.shown
        return ctx.state.shown
    end,
}

return M
