-- data/lua/gnoblin/shell/gestures.lua: the touchpad gestures of the stock shell, and the five actions the configuration
-- used to name (`workspace.next`, `workspace.previous`, `window.close`, `window.minimize`, `window.toggle-maximize`).
--
-- A gesture is bound like a chord: `Seat:bind` takes a key spelled `Swipe3` or `Pinch4` (the gesture and its finger
-- count), Gnoblin claims every such gesture so no client sees it, and the fn runs for each phase of it. The fn gets
-- `e.phase` ("begin", "update", "end" or "cancel"), `e.dx` and `e.dy` (the centroid's movement since the begin, in
-- touchpad units: x to the right, y downward), `e.scale` (the contacts' spread relative to the begin; 1 for a swipe) and
-- `e.rotation` (radians, clockwise positive). A session lock that comes up mid-gesture cancels it.
--
--   seat:bind({ Swipe4 = Gestures.on_swipe({ up = Gestures.actions.window_toggle_maximise }) })
--
-- The stock shell binds one gesture: a three-finger swipe slides to the next or previous workspace, and follows the
-- fingers while they move. A claimed gesture shadows a `touchpad-gestures` command binding of the same kind and finger
-- count, so a configured three-finger swipe command never runs under the stock shell; use four fingers for it.
-- Copy this file and edit it to change any of that.
local Gestures = {
    -- How far fingers move for a slide of one whole workspace, in touchpad units.
    travel = 400,
    -- The share of a slide at which lifting the fingers finishes it; a shorter slide goes back.
    commit = 0.25,
    -- The shortest swipe `Gestures.on_swipe` acts on, in touchpad units.
    swipe_threshold = 48,
    -- How far from 1 a pinch's scale must be for `Gestures.on_pinch` to act on it.
    pinch_threshold = 0.12,
}

-- The window the seat has focused, or nil.
local function focused_window()
    local f = seat.focus
    if f and f:is(Window) then
        return f
    end
end

-- The five actions. Each takes nothing and acts on the Desk under the pointer or the focused window.
Gestures.actions = {
    workspace_next = function()
        go(function()
            desk():step(1)
        end)
    end,
    workspace_previous = function()
        go(function()
            desk():step(-1)
        end)
    end,
    window_close = function()
        local w = focused_window()
        if w then
            w:close()
        end
    end,
    window_minimise = function()
        local w = focused_window()
        if w then
            w:minimise()
        end
    end,
    window_toggle_maximise = function()
        local w = focused_window()
        if w then
            if w.maximised then
                w:unmaximise()
            else
                w:maximise()
            end
        end
    end,
}

-- The way a gesture has gone: "left", "right", "up" or "down" by its longer axis, or nil while it has moved less than
-- `threshold` (default `Gestures.swipe_threshold`).
function Gestures.direction(e, threshold)
    local dx, dy = e.dx, e.dy
    if math.max(math.abs(dx), math.abs(dy)) < (threshold or Gestures.swipe_threshold) then
        return nil
    end
    if math.abs(dx) >= math.abs(dy) then
        return dx > 0 and "right" or "left"
    end
    return dy > 0 and "down" or "up"
end

-- A fn for `Seat:bind` that runs `map[direction]` when a swipe ends, if the swipe went that way (see
-- `Gestures.direction`). A cancelled swipe runs nothing.
function Gestures.on_swipe(map, threshold)
    return function(e)
        if e.phase == "end" then
            local direction = Gestures.direction(e, threshold)
            local fn = direction and map[direction]
            if fn then
                fn(e)
            end
        end
    end
end

-- A fn for `Seat:bind` that runs `map["in"]` or `map.out` when a pinch ends: "out" when the contacts spread to at least
-- 1 + `threshold` of their start (default `Gestures.pinch_threshold`), "in" when they gathered to at most 1 - threshold.
function Gestures.on_pinch(map, threshold)
    local t = threshold or Gestures.pinch_threshold
    return function(e)
        if e.phase == "end" then
            local fn = (e.scale >= 1 + t and map.out) or (e.scale <= 1 - t and map["in"])
            if fn then
                fn(e)
            end
        end
    end
end

-- A fn for `Seat:bind` that slides the Desk's workspaces under the fingers. Fingers moving right bring the previous
-- workspace in from the left, fingers moving left the next one from the right. Lifting after `Gestures.commit` of a
-- `Gestures.travel` finishes the slide from where the fingers left it; lifting sooner, or a cancel, slides back. A
-- gesture that is more vertical than horizontal, or that has no workspace to reveal, moves nothing.
function Gestures.workspace_slide()
    local slide -- { desk, from, sign, to, progress } for the gesture in flight
    -- Puts the workspaces where a slide of `progress` (0 to 1) in direction `sign` has them.
    local function place(s, progress)
        local W = s.desk.width
        s.progress = progress
        s.from.x = s.sign * progress * W
        s.to.x = s.sign * (progress - 1) * W
    end
    -- Hides the workspace a slide had brought in and puts the one it moved back.
    local function reset(s)
        if s.to then
            s.to.visible, s.to.x = false, 0
            s.from.x = 0
            s.to, s.sign = nil, 0
        end
    end
    local function settle_back(s)
        local W = s.desk.width
        s.from:to(motion(0.2), { x = 0 }, ease.out_cubic)
        await(s.to:to(motion(0.2), { x = -s.sign * W }, ease.out_cubic))
        if s.desk.active ~= s.to then
            s.to.visible, s.to.x = false, 0
        end
    end
    return function(e)
        if e.phase == "begin" then
            local d = desk()
            slide = d and { desk = d, from = d.active, sign = 0 } or nil
            return
        end
        local s = slide
        if not s then
            return
        end
        if e.phase == "update" then
            if s.desk.active ~= s.from then -- something else switched workspace under the fingers
                if s.desk.active ~= s.to then -- the new workspace is showing for good if it is the one switched to
                    reset(s)
                end
                slide = nil
                return
            end
            if e.dx == 0 then
                return -- no direction yet
            end
            local sign = e.dx > 0 and 1 or -1
            if not s.to and math.abs(e.dy) > math.abs(e.dx) then
                return -- vertical so far: not a slide
            end
            if sign ~= s.sign then
                reset(s)
                local target = s.desk.space.children[s.desk:position(s.from) - sign]
                if target then
                    s.sign, s.to = sign, target
                    target.visible = true
                end
            end
            if s.to then
                place(s, math.min(math.abs(e.dx) / Gestures.travel, 1))
            end
            return
        end
        slide = nil
        if not s.to or s.desk.active ~= s.from then
            return
        end
        if e.phase == "end" and s.progress >= Gestures.commit and math.abs(e.dx) >= math.abs(e.dy) then
            go(function()
                s.desk:show(s.to, s.progress)
            end)
        else
            go(function()
                settle_back(s)
            end)
        end
    end
end

if seat then
    seat:bind({ Swipe3 = Gestures.workspace_slide() })
end

return Gestures
