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
    seat.focus = self
end
function Window:on_press(e)
    seat.focus = self
    self:raise()
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

function drag(s, w) -- follow the pointer until release
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
