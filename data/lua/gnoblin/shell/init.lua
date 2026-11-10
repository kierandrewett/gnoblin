-- data/lua/gnoblin/shell/init.lua: the stock shell. Copy it and edit it.
seat, lock = root:find(Seat), root:find(Lock)
spawn, date = sys.spawn, sys.date
function desk()
    return seat.screen.source
end -- the Desk under the pointer

-- Key and gesture bindings: Gnoblin claims each chord, so no client sees it.
function Seat:bind(map)
    local keys = self.data.keys or {}
    local chords = {}
    for chord in pairs(map) do
        chords[#chords + 1] = chord
    end
    table.sort(chords)
    for _, chord in ipairs(chords) do
        keys[self:claim(chord, map[chord] ~= false)] = map[chord] or nil
    end
    self.data.keys = keys
end
function Seat:on_chord(e)
    local fn = self.data.keys[e.chord]
    if fn then
        fn(e)
    end
end

-- One Desk per Screen: bars on its edges, workspaces in the space between.
Node:extend("Desk")({
    screen = Screen,
    active = Workspace,
    space = Node,
    width = function(d)
        return d.screen.width
    end,
    height = function(d)
        return d.screen.height
    end,
})
function Desk:workspace(name)
    return Floating({ name = name })
end -- override to choose a layout
function Desk:build()
    self.space = Node({ clip = true })
    for i = 1, 9 do
        self.space:add(self:workspace(tostring(i)))
    end
    for i, ws in ipairs(self.space.children) do
        ws.visible, ws.active = i == 1, i == 1
    end
    self.active = self.space.children[1]
    return { self.space }
end
function Desk:arrange()
    local top, bottom = 0, 0
    for _, bar in ipairs(self:all("> Bar")) do
        bar.width = self.width
        if bar.edge == "top" then
            bar.y, top = top, top + bar.height
        else
            bottom = bottom + bar.height
            bar.y = self.height - bottom
        end
    end
    local s = self.space
    s.y, s.width, s.height = top, self.width, self.height - top - bottom
    for _, ws in ipairs(s.children) do
        ws.width, ws.height = s.width, s.height
    end
end
function Desk:show(ws)
    local from = self.active
    if not ws or ws == from then
        return
    end
    local dir, W = ws.index > from.index and 1 or -1, self.width
    ws.x, ws.visible, ws.active, from.active, self.active = dir * W, true, true, false, ws
    from:to(0.25, { x = -dir * W }, ease.out_cubic)
    await(ws:to(0.25, { x = 0 }, ease.out_cubic))
    if self.active ~= from then
        from.visible, from.x = false, 0
    end
end
function Desk:step(n)
    self:show(self.space.children[self.active.index + n])
end
function Workspace:on_activate()
    self:closest(Desk):show(self)
end
function Workspace:walk(step) -- focus the next window in tree order
    local wins, f = self:all(Window), seat.focus
    for i, w in ipairs(wins) do
        if w == f then
            seat.focus = wins[i + step] or f
            return
        end
    end
end
function Screen:on_ready()
    self.source = root:add(Desk({ screen = self }))
end
function Screen:on_destroyed() -- unplugged: its windows move to another desk
    local to = root:find(Desk)
    if to == self.source then
        to = root:all(Desk)[2]
    end
    for _, w in ipairs(self.source:all("Workspace > Window")) do
        if to then
            to.active:add(w)
        end
    end
    self.source:destroy()
end
-- Focus and raise policy: seat.focus_options holds the window_management focus keys (nil = documented defaults).
function focus_settings()
    return Focus.settings(seat.focus_options or {})
end
-- Window rules: seat.window_rules is the user's list, compiled once per table (a bad rule fails at load).
local rules_src, rules_compiled, rules_seen
function window_rules()
    if not rules_seen or rules_src ~= seat.window_rules then
        rules_seen, rules_src = true, seat.window_rules
        rules_compiled = Rules.compile(seat.window_rules or {})
    end
    return rules_compiled
end

function Window:on_ready()
    local p = self.parent_window
    if p then
        p:add(self) -- a dialog moves with its parent
        self.x, self.y = (p.width - self.committed_width) // 2, (p.height - self.committed_height) // 2
    else
        desk().active:add(self)
    end
    self.activated = function(w)
        return seat.focus == w
    end
    if Focus.opens_focused(focus_settings(), self.from_launcher) then
        seat.focus = self
    end
    self:apply_rules()
end
-- The rule key each window last applied, kept off the node: the core does not declare it.
local rule_keys = setmetatable({}, { __mode = "k" })
-- The window as the rules read it: plain values, so rules.lua stays pure.
local function rule_view(w)
    local ws = w:closest(Workspace)
    return {
        app_id = w.app_id,
        title = w.title,
        focused = seat.focus == w,
        workspace_id = ws and ws.name or nil,
    }
end
function Window:apply_rules()
    local rules = window_rules()
    local view = rule_view(self)
    local key = Rules.key(rules, view)
    if key == rule_keys[self] then
        return
    end
    rule_keys[self] = key
    local fx = Rules.effects(rules, view)
    if fx.fullscreen ~= nil then
        self.fullscreen = fx.fullscreen
    end
    if fx.secure ~= nil then
        self.secure = fx.secure
    end
    if fx.workspace then
        for _, ws in ipairs(self:closest(Desk).space.children) do
            if ws.name == tostring(fx.workspace.id) and self:closest(Workspace) ~= ws then
                ws:add(self)
            end
        end
    end
end
function Window:on_enter() -- focus follows the pointer under the hover modes; auto-raise after a rest
    local opts = focus_settings()
    if Focus.hover_focus(opts, self, true) then
        seat.focus = self
    end
    self:apply_rules()
    if opts.auto_raise then
        go(function()
            await(Focus.auto_raise_seconds(opts))
            if seat.focus == self then
                self:raise()
            end
        end)
    end
end
function Window:on_press(e)
    seat.focus = self
    if focus_settings().raise_on_click then
        self:raise()
    end
    e:stop()
end
function Window:on_move(e)
    drag(e.seat, self)
end
function Window:on_closed()
    self:vanish()
end
function Window:vanish()
    self:destroy()
end -- a method: replace it to animate

-- Pointer distance from a screen edge that tiles a dropped window (no documented value: a small fixed zone).
local EDGE_MARGIN = 4
function drag(s, w) -- follow the pointer until release, then tile it if dropped on an edge
    local dx, dy = w.x - s.x, w.y - s.y
    w.x = function()
        return s.x + dx
    end
    w.y = function()
        return s.y + dy
    end
    s:grab(w)
    await(w.release)
    s:grab(nil)
    w.x, w.y = w.x, w.y -- a plain write ends the bindings
    if not Snap.enabled({ edge_tiling = seat.edge_tiling }) then
        return
    end
    local d = desk()
    local area = { x = 0, y = d.space.y, w = d.width, h = d.space.height }
    local rect = Snap.tile_rect(Snap.edge_zone(s.x, s.y, area, EDGE_MARGIN), area)
    if rect then
        w.x, w.y, w.width, w.height = rect.x, rect.y, rect.w, rect.h
    end
end
-- Super+Left and Super+Right move focus between the columns of a Strip; on any other layout they do nothing.
-- A shell loaded without a seat (the Lua tests) has no keys to bind.
if seat then
    seat:bind({
        ["Super+Left"] = function()
            if desk().active:is(Strip) then
                desk().active:focus_left()
            end
        end,
        ["Super+Right"] = function()
            if desk().active:is(Strip) then
                desk().active:focus_right()
            end
        end,
    })
end
