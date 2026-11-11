-- data/lua/gnoblin/shell/xr.lua: the stock shell's side of XR mode (docs/xr.md, design/xr-extension.md).
-- The gnoblin-xr extension is an out-of-process program on the owner socket. It reads the View nodes under the node named
-- `room`, in tree order, and shows each one in the headset; it never decides where a window sits. This file decides:
-- it builds `room` when the seat named `xr` arrives, gives every window a View, places the Views on a ring round the
-- viewer, and tears the room down when the seat goes. Units: `room` is in millimetres (the head camera's `metres` is
-- 0.001), and angles are radians, as the node schema says.
local XR_SEAT, UNIT = "xr", 0.001

-- The ring. A pure function of the Views' pixel sizes and the focus, so it is tested without a core.
local XrRing = {
    radius = 1600, -- the distance from the viewer to the centre of a panel, in millimetres
    eye = 1400, -- the height of a panel's centre above the room's floor
    mm_per_px = 0.75, -- one window pixel is this many millimetres: 800 px is 0.6 m
    focus_scale = 1.15, -- the focused panel is this much larger ...
    focus_pull = 100, -- ... and this much closer to the viewer
    gap = math.rad(3), -- the angle between neighbouring panels
    half_arc = math.rad(150), -- the most the panels reach to either side of the front: panels shrink to fit
    max_panels = 12, -- panels on the ring; the rest stack behind it
    stack_step = 250, -- each tier of stacked panels is this much further back
}

-- Where each of `views` goes. `views` is a list of { id = <anything comparable>, width = <px>, height = <px> } in
-- creation order; `focus` is the `id` of the focused view, or nil. Returns two lists:
--   poses: one per view, in the order of `views`: { x, y, z, rotation = { 0, yaw, 0 }, scale, focused, behind }
--     x, y, z are in millimetres in the room's frame (the viewer is at x = 0, z = 0, looking down -z), `scale` is
--     millimetres per pixel for the View, `focused` is true for the front-centre panel and `behind` is 0 on the ring
--     and the tier (1, 2, ...) of a stacked panel.
--   order: the view indices from the farthest to the nearest, which is the order the room must list the Views in,
--     because the extension draws them in tree order and nothing depth-tests.
-- The ring is a carousel: the focused panel is at the front, and its neighbours in creation order are on either
-- side (the right side holds the extra one), wrapping round. Without a focus the first view is at the front.
function XrRing.arrange(views, focus)
    local n = #views
    local poses, order = {}, {}
    if n == 0 then
        return poses, order
    end
    local R = XrRing
    local focus_index
    if focus ~= nil then
        for i, v in ipairs(views) do
            if v.id == focus then
                focus_index = i
                break
            end
        end
    end
    local m = math.min(n, R.max_panels)
    local seq = {} -- the views, with a focused one past the ring swapped into the ring's last place
    for i = 1, n do
        seq[i] = i
    end
    if focus_index and focus_index > m then
        seq[m], seq[focus_index] = seq[focus_index], seq[m]
    end
    local c = 1 -- the ring position at the front
    if focus_index then
        for k = 1, m do
            if seq[k] == focus_index then
                c = k
            end
        end
    end
    local function at(k) -- ring positions wrap
        return (k - 1) % m + 1
    end
    local nr = m // 2
    local right, left = {}, {}
    for j = 1, nr do
        right[j] = seq[at(c + j)]
    end
    for j = 1, m - 1 - nr do
        left[j] = seq[at(c - j)]
    end
    local centre = seq[c]
    local function span(i, focused) -- the angle a panel covers, as arc length over the radius
        local w = math.max(views[i].width or 0, 0)
        return (focused and R.focus_scale or 1) * w * R.mm_per_px / R.radius
    end
    local a_centre = span(centre, focus_index ~= nil)
    -- Both sides must fit in half_arc, gaps included; the panels shrink together until they do.
    local shrink = 1
    for _, side in ipairs({ right, left }) do
        local need = a_centre / 2
        for _, i in ipairs(side) do
            need = need + span(i, false)
        end
        local allowed = R.half_arc - #side * R.gap
        if need > allowed then
            shrink = math.min(shrink, allowed / need)
        end
    end
    local phi = { [centre] = 0 }
    for _, s in ipairs({ { 1, right }, { -1, left } }) do
        local sign, side = s[1], s[2]
        local edge = shrink * a_centre / 2
        for _, i in ipairs(side) do
            local half = shrink * span(i, false) / 2
            local at_angle = edge + R.gap + half
            phi[i] = sign * at_angle
            edge = at_angle + half
        end
    end
    local function put(i, angle, radius, focused, tier)
        poses[i] = {
            x = radius * math.sin(angle),
            y = R.eye,
            z = -radius * math.cos(angle),
            rotation = { 0, 0 - angle, 0 }, -- a panel faces the viewer: yaw -angle turns +z back to the origin
            scale = shrink * (focused and R.focus_scale or 1) * R.mm_per_px,
            focused = focused,
            behind = tier,
        }
    end
    for k = 1, m do
        local i = seq[k]
        local focused = focus_index ~= nil and i == centre
        put(i, phi[i], focused and R.radius - R.focus_pull or R.radius, focused, 0)
    end
    -- Extras sit behind the ring panel at the same angle, a tier further back for every full turn of the ring.
    local tiers = {}
    for j = 1, n - m do
        local tier = (j - 1) // m + 1
        local i = seq[m + j]
        put(i, phi[seq[at(j)]], R.radius + R.stack_step * tier, false, tier)
        tiers[tier] = tiers[tier] or {}
        table.insert(tiers[tier], i)
    end
    for tier = #tiers, 1, -1 do
        for _, i in ipairs(tiers[tier]) do
            order[#order + 1] = i
        end
    end
    local ring = {}
    for k = 1, m do
        if not (focus_index and seq[k] == centre) then
            ring[#ring + 1] = seq[k]
        end
    end
    table.sort(ring)
    for _, i in ipairs(ring) do
        order[#order + 1] = i
    end
    if focus_index then
        order[#order + 1] = centre
    end
    return poses, order
end

-- The room: the node the extension reads. It lists the head camera first, then one View per window, farthest first.
-- It is its own arrangement: the binding runs again when a window changes size, is minimised or destroyed, when a View
-- comes or goes, or when the focus moves.
Node:extend("XrRoom")({})

-- The live room, or nil: { room, seat, views = { [window] = View }, list = { window, ... } }. `list` is the windows in
-- the order they gained a View, which is the ring's order: the room's own child order is back to front instead. A room
-- that is already in the tree when the seat arrives is the user's, and is left alone.
local state

-- Whether a window gets a View: dialogs are drawn inside their parent's View, and a window rule can say `xr = false`.
local function shown(w)
    return not w.parent_window and w.data.xr ~= false
end
local function add_view(w)
    if not state or state.views[w] or not shown(w) then
        return
    end
    state.views[w] = state.room:add(View({
        source = w,
        width = w.width,
        height = w.height,
        visible = not w.minimised,
    }))
    state.list[#state.list + 1] = w
end
local function forget(w) -- the View is gone or going: take its window off the list
    for i, x in ipairs(state.list) do
        if x == w then
            table.remove(state.list, i)
            break
        end
    end
end
local function drop_view(w)
    local v = state and state.views[w]
    if v then
        state.views[w] = nil
        forget(w)
        v:destroy()
    end
end
local function sync(w)
    if shown(w) then
        add_view(w)
    else
        drop_view(w)
    end
end

-- What was last written to each View, so a run that changes nothing writes nothing.
local written = setmetatable({}, { __mode = "k" })
local function put(v, key, value)
    local last = written[v]
    if not last then
        last = {}
        written[v] = last
    end
    local same = last[key] == value
    if type(value) == "table" then
        same = last[key] ~= nil and last[key][1] == value[1] and last[key][2] == value[2] and last[key][3] == value[3]
    end
    if not same then
        last[key] = value
        v[key] = value
    end
end

function XrRoom:arrange()
    local _ = self.children -- read first: a View coming or going reruns this, even before the room is live
    if not state or state.room ~= self then
        return
    end
    local list, views, hidden = state.list, {}, {}
    for i = #list, 1, -1 do
        local w = list[i]
        local v = state.views[w]
        if v.source == nil then -- its window is gone
            state.views[w] = nil
            table.remove(list, i)
            v:destroy()
        end
    end
    local children = self.children -- after the sweep: a destroyed View is not in it
    for _, w in ipairs(list) do
        local v = state.views[w]
        put(v, "width", w.width)
        put(v, "height", w.height)
        put(v, "visible", not w.minimised)
        if w.minimised then
            hidden[#hidden + 1] = v
        else
            views[#views + 1] = { id = w, width = w.width, height = w.height, view = v }
        end
    end
    local poses, order = XrRing.arrange(views, seat and seat.focus or nil)
    for i, pose in ipairs(poses) do
        local v = views[i].view
        put(v, "x", pose.x)
        put(v, "y", pose.y)
        put(v, "z", pose.z)
        put(v, "rotation", pose.rotation)
        put(v, "scale", { pose.scale, pose.scale, pose.scale })
    end
    -- Back to front: the extension submits the Views in tree order, so the nearest one is last. Minimised Views draw
    -- nothing and go first.
    local want = hidden
    for _, i in ipairs(order) do
        want[#want + 1] = views[i].view
    end
    local mine = {}
    for _, v in ipairs(want) do
        mine[v] = true
    end
    local k, same = 0, true
    for _, c in ipairs(children) do
        if mine[c] then
            k = k + 1
            same = same and want[k] == c
        end
    end
    if not same then
        for _, v in ipairs(want) do
            self:add(v)
        end
    end
end

-- Builds the room for the seat named `xr`: the head camera, then a View for every window already in the tree.
local function build(s)
    if state or root:find("#room") then
        return
    end
    state = { seat = s, views = {}, list = {} }
    state.room = XrRoom({ name = "room", y = -XrRing.eye })
    state.room:add(Camera({ name = "head", projection = "stereo", metres = UNIT }))
    root:add(state.room)
    for _, w in ipairs(root:all(Window)) do
        add_view(w)
    end
end
local function teardown()
    local room = state and state.room
    state = nil
    if room then
        room:destroy()
    end
end

function Seat:on_ready()
    if self.name == XR_SEAT then
        build(self)
    end
end
function Seat:on_destroyed()
    if state and state.seat == self then
        teardown()
    end
end
function XrRoom:on_destroyed() -- something else destroyed the room: the seat stays, and the next xr seat builds a new one
    if state and state.room == self then
        state = nil
    end
end
function Window:on_ready()
    sync(self)
end
function Window:on_destroyed()
    drop_view(self)
end
-- A rule that says `xr = false` (or no longer says it) adds or removes the View when the rules are applied.
local apply_rules = Window.apply_rules
function Window:apply_rules()
    apply_rules(self)
    sync(self)
end

return XrRing
