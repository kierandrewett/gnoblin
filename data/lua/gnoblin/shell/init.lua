-- data/lua/gnoblin/shell/init.lua: the stock shell. Copy it and edit it.
-- Loaded as require("gnoblin.shell"). The modules it depends on are required here, before the code that uses them.
local Focus = require("gnoblin.shell.focus")
local Snap = require("gnoblin.shell.snap")
local Rules = require("gnoblin.shell.rules")
local Drag = require("gnoblin.shell.drag")
require("gnoblin.shell.ui")
require("gnoblin.shell.floating")
require("gnoblin.shell.strip")
seat, lock = root:find(Seat), root:find(Lock)
spawn, date = sys.spawn, sys.date
function desk_of(screen) -- the Desk whose screen it is
    for _, d in ipairs(root:all(Desk)) do
        if d.screen == screen then
            return d
        end
    end
end
function desk() -- the Desk under the pointer
    return desk_of(seat.screen)
end

-- Key and gesture bindings: Gnoblin claims each chord, so no client sees it. `keys` maps a chord to its fn and
-- `claims` maps the claim id the compositor reports back to its chord. A bound fn runs on press.
function Seat:bind(map)
    local keys, claims = self.data.keys or {}, self.data.claims or {}
    local chords = {}
    for chord in pairs(map) do
        chords[#chords + 1] = chord
    end
    table.sort(chords)
    for _, chord in ipairs(chords) do
        keys[chord] = map[chord] or nil
        claims[self:claim(chord, map[chord] ~= false)] = chord
    end
    self.data.keys, self.data.claims = keys, claims
end
function Seat:unbind(id)
    local chord = self.data.claims[id]
    if chord then
        self.data.keys[chord] = nil
        self.data.claims[id] = nil
    end
    self:unclaim(id)
end
function Seat:on_chord(e)
    local chord = self.data.claims[e.claim]
    local fn = chord and self.data.keys[chord]
    if fn and e.state == "press" then
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
    for _, w in ipairs(self:all(Window)) do -- the work area may have moved under a maximised or snapped window
        w:refit(self:work_area())
    end
end
-- The rectangle the Desk's windows fill: the space under the bars.
function Desk:work_area()
    return { x = 0, y = self.space.y, w = self.width, h = self.space.height }
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
    root:add(Desk({ screen = self }))
end
function Screen:on_destroyed() -- unplugged: its windows move to another desk
    local gone = desk_of(self)
    if not gone then
        return
    end
    local to
    for _, d in ipairs(root:all(Desk)) do
        if d ~= gone then
            to = d
            break
        end
    end
    for _, w in ipairs(gone:all("Workspace > Window")) do
        if to then
            to.active:add(w)
        end
    end
    gone:destroy()
end
-- Focus and raise policy: seat.focus_options holds the window_management focus keys (nil = documented defaults).
function focus_settings()
    return Focus.settings(seat.focus_options or {})
end
-- Window rules: seat.data.window_rules is the user's list, compiled once per table (a bad rule fails at load).
local rules_src, rules_compiled, rules_seen
function window_rules()
    local src = seat and seat.data.window_rules or nil
    if not rules_seen or rules_src ~= src then
        rules_seen, rules_src = true, src
        rules_compiled = Rules.compile(src or {})
    end
    return rules_compiled
end

function Window:on_ready()
    local p = self.parent_window
    if p then
        p:add(self) -- a dialog moves with its parent
        self.x, self.y = (p.width - self.width) // 2, (p.height - self.height) // 2
    else
        desk().active:add(self)
    end
    if Focus.opens_focused(focus_settings(), self.from_launcher) then
        seat.focus = self
    end
    self:apply_rules()
end
-- What the rules last did to each window, kept off the node: the key of the rules that matched, and the
-- fields the rules wrote, so a field no rule sets any more goes back to its default.
local rule_state = setmetatable({}, { __mode = "k" })
-- The window as the rules read it: plain values, so rules.lua stays pure.
local function rule_view(w)
    local ws = w:closest(Workspace)
    return {
        app_id = w.app_id,
        title = w.title,
        focused = seat ~= nil and seat.focus == w,
        workspace_id = ws and ws.name or nil,
    }
end
-- The fields the rules own, with the value each takes when no rule sets it.
local RULE_DEFAULTS = { secure = false, fullscreen = false, maximised = false, opacity = 1.0 }
-- The rules are a binding: arrange runs again when a rule's inputs (title, app id, focus, workspace) change.
function Window:arrange()
    self:apply_rules()
end
function Window:apply_rules()
    local rules = window_rules()
    local view = rule_view(self)
    local key = Rules.key(rules, view)
    local state = rule_state[self] or { wrote = {} }
    if key == state.key then
        return
    end
    local fx = Rules.effects(rules, view)
    local d = self:closest(Desk)
    if fx.workspace and not d then
        return -- not on a desk yet: on_ready applies the rules once the window is placed
    end
    state.key = key
    rule_state[self] = state
    for field, default in pairs(RULE_DEFAULTS) do
        if fx[field] ~= nil then
            self[field] = fx[field]
            state.wrote[field] = true
        elseif state.wrote[field] then
            self[field] = default
            state.wrote[field] = nil
        end
    end
    if fx.workspace and d and not self.parent_window then -- a dialog follows its parent, whatever a rule says
        for _, ws in ipairs(d.space.children) do
            if ws.name == tostring(fx.workspace.id) and self:closest(Workspace) ~= ws then
                ws:add(self)
            end
        end
    end
end
-- The window whose button is held, so focus and rest wait for the release.
local pressed
-- Per window: a counter a leave or new rest bumps, so a stale rest timer does nothing.
local rest_gen = setmetatable({}, { __mode = "k" })
-- Per window: the auto-raise token. A leave clears it, so a stale auto-raise does nothing.
local auto_token = setmetatable({}, { __mode = "k" })

-- After the pointer rests over `w` for the rest time, focus it, unless the window was left or pressed meanwhile.
local function rest_focus_later(w, opts)
    local gen = (rest_gen[w] or 0) + 1
    rest_gen[w] = gen
    go(function()
        await(Focus.rest_seconds())
        if rest_gen[w] == gen and pressed ~= w then
            seat.focus = w
        end
    end)
end

function Window:on_enter() -- focus follows the pointer under the hover modes; auto-raise after a rest
    local opts = focus_settings()
    self:apply_rules()
    if pressed then
        return
    end
    if Focus.rest_focus(opts) then
        rest_focus_later(self, opts)
    elseif Focus.hover_focus(opts, self, true) then
        seat.focus = self
    end
    if opts.auto_raise then
        local token = {}
        auto_token[self] = token
        go(function()
            await(Focus.auto_raise_seconds(opts))
            if auto_token[self] == token and seat.focus == self then
                self:raise()
            end
        end)
    end
end
function Window:on_motion()
    if pressed then
        return
    end
    local opts = focus_settings()
    if Focus.rest_focus(opts) then
        rest_focus_later(self, opts)
    elseif Focus.hover_focus(opts, self, false) then
        seat.focus = self
    end
end
function Window:on_leave()
    rest_gen[self] = (rest_gen[self] or 0) + 1
    auto_token[self] = nil
    if pressed ~= self and seat.focus == self and Focus.leave_clears(focus_settings()) then
        seat.focus = nil
    end
end
function Window:on_press(e)
    seat.focus = self
    pressed = self
    if focus_settings().raise_on_click then
        self:raise()
    end
    e:stop()
end
function Window:on_release()
    pressed = nil
end
function Window:on_activate() -- an activation request raises and focuses a window on the active workspace
    if self:closest(Workspace) == desk().active then
        seat.focus = self
        self:raise()
    end
end
-- A raised window goes last among its siblings, which draws on top. A fullscreen sibling keeps the top: it is put back
-- over the raised window, so a window that raises or is activated lands under the fullscreen ones.
function Window:raise()
    local parent, above = self.parent, {}
    for _, c in ipairs(parent.children) do
        if c ~= self and c.fullscreen and not self.fullscreen then
            above[#above + 1] = c
        end
    end
    parent:add(self)
    for _, c in ipairs(above) do
        parent:add(c)
    end
end
function Window:on_move(e)
    drag(e.seat, self)
end
function Window:on_closed()
    if pressed == self then
        pressed = nil
    end
    if seat.focus == self then
        seat.focus = nil
    end
    rest_gen[self] = (rest_gen[self] or 0) + 1
    auto_token[self] = nil
    self:vanish()
end
function Window:vanish()
    self:destroy()
end -- a method: replace it to animate

-- Pointer distance from a screen edge that tiles a dropped window (no documented value: a small fixed zone).
local EDGE_MARGIN = 4
function drag(s, w) -- follow the pointer until release, then tile it if dropped on an edge
    local plan = Drag.start(w)
    if plan == "refuse" then
        return
    elseif plan == "float" then
        w:unmaximise()
    end
    local mode = Drag.mode(s.drag_constraint)
    local dx, dy = w.x - s.x, w.y - s.y
    local function place()
        return Drag.constrain({ x = s.x + dx, y = s.y + dy, w = w.width, h = w.height }, work_area(), mode)
    end
    w.x = function()
        return place().x
    end
    w.y = function()
        return place().y
    end
    s:grab(w)
    await(w.release)
    s:grab(nil)
    w.x, w.y = w.x, w.y -- a plain write ends the bindings
    local zone = Snap.edge_zone(s.x, s.y, work_area(), EDGE_MARGIN)
    if zone then
        w:snap(zone)
    end
end

-- The area a snapped or maximised window fills: the Desk's work area, inside the bars.
function work_area()
    return desk():work_area()
end
-- A window's floating rectangle, kept while it is snapped or maximised so that leaving either state restores it.
local function keep_floating(w)
    if not w.data.floating then
        w.data.floating = { x = w.x, y = w.y, w = w.width, h = w.height }
    end
end
local function restore_floating(w)
    local r = w.data.floating
    if r then
        w.data.floating = nil
        w.x, w.y, w.width, w.height = r.x, r.y, r.w, r.h
    end
end
-- Tile the window to a snap zone. Refused (false) when edge tiling is off or the zone does not tile. A maximised
-- window floats first, so unsnapping returns it to the rectangle it had before.
function Window:snap(zone)
    local rect = Snap.enabled({ edge_tiling = seat.edge_tiling }) and Snap.tile_rect(zone, work_area())
    if not rect then
        return false
    end
    if self.maximised then
        self:unmaximise()
    end
    keep_floating(self)
    self.data.snapped = zone
    self.x, self.y, self.width, self.height = rect.x, rect.y, rect.w, rect.h
    return true
end
function Window:unsnap()
    if not self.data.snapped then
        return false
    end
    self.data.snapped = nil
    restore_floating(self)
    return true
end
function Window:maximise()
    if self.maximised or self.fullscreen then
        return false
    end
    if self.data.snapped then
        self:unsnap()
    end
    keep_floating(self)
    self.maximised = true
    local a = work_area()
    self.x, self.y, self.width, self.height = a.x, a.y, a.w, a.h
    return true
end
function Window:unmaximise()
    if not self.maximised then
        return false
    end
    self.maximised = false
    restore_floating(self)
    return true
end
-- Re-fits a maximised or snapped window to a work area that has changed (a bar was added, removed or resized).
function Window:refit(a)
    if self.maximised then
        self.x, self.y, self.width, self.height = a.x, a.y, a.w, a.h
    elseif self.data.snapped then
        local rect = Snap.tile_rect(self.data.snapped, a)
        self.x, self.y, self.width, self.height = rect.x, rect.y, rect.w, rect.h
    end
end
-- The client asks for a state change; the shell decides and the window's properties echo the answer.
function Window:on_request(e)
    if e.request == "maximize" then
        self:maximise()
    elseif e.request == "unmaximize" then
        self:unmaximise()
    elseif e.request == "activate" then
        self:on_activate()
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

-- The modules that extend the classes defined above. decorations.lua chains onto Window:on_ready, so it loads last.
require("gnoblin.shell.panel")
require("gnoblin.shell.lock")
require("gnoblin.shell.overview")
require("gnoblin.shell.launcher")
require("gnoblin.shell.workspaces")
require("gnoblin.shell.decorations")
