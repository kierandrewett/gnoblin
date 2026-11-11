-- data/lua/gnoblin/shell/init.lua: the stock shell. Copy it and edit it.
-- Loaded as require("gnoblin.shell"). The modules it depends on are required here, before the code that uses them.
local Focus = require("gnoblin.shell.focus")
local Snap = require("gnoblin.shell.snap")
local Rules = require("gnoblin.shell.rules")
local Sets = require("gnoblin.shell.sets")
local Drag = require("gnoblin.shell.drag")
require("gnoblin.shell.ui")
local Placement = require("gnoblin.shell.floating")
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

-- The duration the shell gives an animation: `seconds`, or 0 while the seat asks for reduced motion
-- (`appearance.reduced-motion`). A 0 makes `node:to` an instant write. The compositor lands a timed write at once
-- under reduced motion too, so a shell that forgets this still obeys the preference.
function motion(seconds)
    return seat.reduced_motion and 0 or seconds
end

-- Key and gesture bindings: Gnoblin claims each chord or gesture, so no client sees it. `keys` maps a chord (or a
-- gesture's spelling) to its fn and `claims` maps the claim id the compositor reports back to it. A bound chord fn runs
-- on press. A key spelled `Swipe3` or `Pinch4` (the gesture and its finger count) claims that gesture: its fn runs for
-- every phase, with `e.phase` ("begin", "update", "end" or "cancel"), `e.dx` and `e.dy` (the movement since the
-- begin), `e.scale` (1 for a swipe; above 1 for a spread) and `e.rotation` (radians, clockwise positive). A fn of
-- `false` claims the chord or gesture and does nothing with it.
--
-- `device = "Macro Pad"` in the map scopes every binding in it to the devices that report that name (see
-- `seat.device`, one "id:name:caps" item per device): a chord claimed for the pad is withheld from clients only when
-- the pad sends it, and the same chord on the desk keyboard still types. A chord bound for a device is a different
-- binding from the same chord bound for every device, and wins on its device. `keys` and `claims` hold the chord
-- behind the device name and a line feed when it is scoped, so the two bindings do not share a slot.
local GESTURES = { Swipe = "swipe", Pinch = "pinch" }
local function claim_key(seat, key, grab, device)
    local name, fingers = key:match("^(%a+)(%d+)$")
    if GESTURES[name] then
        return seat:claim_gesture(GESTURES[name], tonumber(fingers), device)
    end
    return seat:claim(key, grab, device)
end
function Seat:bind(map)
    local keys, claims = self.data.keys or {}, self.data.claims or {}
    local device = type(map.device) == "string" and map.device or nil
    local chords = {}
    for chord in pairs(map) do
        if not (chord == "device" and device) then
            chords[#chords + 1] = chord
        end
    end
    table.sort(chords)
    for _, chord in ipairs(chords) do
        local slot = device and (device .. "\n" .. chord) or chord
        keys[slot] = map[chord] or nil
        claims[claim_key(self, chord, map[chord] ~= false, device)] = slot
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
function Seat:on_gesture(e)
    local key = self.data.claims[e.claim]
    local fn = key and self.data.keys[key]
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
        return d.screen and d.screen.width or 0
    end,
    height = function(d)
        return d.screen and d.screen.height or 0
    end,
})
function Desk:workspace(name)
    return Floating({ name = name })
end -- override to choose a layout
-- The workspaces a Desk starts with: the configuration's declared list (`seat.workspaces`, one "id:name" item each)
-- as { id, title } pairs, or the nine workspaces "1" to "9" when none is declared. A workspace is named by its id.
local function declared_workspace_list()
    local out = {}
    for _, item in ipairs(seat and seat.workspaces or {}) do
        local id, title = item:match("^([^:]*):(.*)$")
        out[#out + 1] = { id = id or item, title = title or item }
    end
    if #out == 0 then
        for i = 1, 9 do
            out[i] = { id = tostring(i), title = tostring(i) }
        end
    end
    return out
end
function Desk:build()
    self.space = Node({ clip = true })
    for _, w in ipairs(declared_workspace_list()) do
        local ws = self.space:add(self:workspace(w.id))
        ws.data.title = w.title
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
-- A workspace's place among the Desk's workspaces, from 1. The native `index` is only set on workspaces the compositor
-- made, so the shell's own come from tree order.
function Desk:position(ws)
    for i, c in ipairs(self.space.children) do
        if c == ws then
            return i
        end
    end
end
-- Slides to `ws`. `done` (0 to 1, default 0) is how much of the slide a touchpad gesture has already made: the
-- slide carries on from where the workspaces are and takes the rest of its time.
function Desk:show(ws, done)
    local from = self.active
    if not ws or ws == from then
        return
    end
    done = done or 0
    local dir, W = self:position(ws) > self:position(from) and 1 or -1, self.width
    ws.x, ws.visible, ws.active, from.active, self.active = dir * W * (1 - done), true, true, false, ws
    if done > 0 then
        from.x = -dir * W * done
    end
    local seconds = motion(0.25 * (1 - done))
    from:to(seconds, { x = -dir * W }, ease.out_cubic)
    await(ws:to(seconds, { x = 0 }, ease.out_cubic))
    if self.active ~= from then
        from.visible, from.x = false, 0
    end
    self:sync_fullscreen()
end
function Desk:step(n)
    self:show(self.space.children[self:position(self.active) + n])
end
-- Removes a workspace from the Desk. Its top-level windows move to the neighbour before it (the one after it, for the
-- first), and that neighbour shows if the removed one was active. Refused (false) for a workspace of another Desk, or
-- the last one.
function Desk:remove_workspace(ws)
    local list = self.space.children
    local at
    for i, c in ipairs(list) do
        if c == ws then
            at = i
        end
    end
    if not at or #list < 2 then
        return false
    end
    local to = list[at - 1] or list[at + 1]
    for _, w in ipairs(ws:all("> Window")) do
        to:add(w)
    end
    if self.active == ws then
        self.active, to.active, to.visible, to.x = to, true, true, 0
    end
    ws:destroy()
    self:arrange()
    return true
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
-- The Desk of each Screen, kept after the Screen is destroyed: a Screen's Desk loses its screen reference then.
local desk_by_screen = setmetatable({}, { __mode = "k" })
-- Desks whose output was unplugged with no other output left. Their windows wait here, hidden, for the next output.
local parked_desks = {}
function Screen:on_ready()
    local parked = table.remove(parked_desks, 1)
    if parked then -- an output came back: the parked Desk and its windows return to it
        parked.screen = self
        desk_by_screen[self] = parked
        parked:arrange()
        parked.active.visible = true
        if not seat.focus then
            seat.focus = parked.active:all(Window)[1] or parked:all(Window)[1]
        end
        return
    end
    desk_by_screen[self] = root:add(Desk({ screen = self }))
    workspace_sets() -- the user's sets are checked now that a desk has the workspaces they name
end
function Screen:on_destroyed() -- unplugged: its windows move to another desk, or wait hidden for the next output
    local gone = desk_by_screen[self]
    desk_by_screen[self] = nil
    if not gone then
        return
    end
    local to
    for _, d in ipairs(root:all(Desk)) do
        if d ~= gone and d.screen then
            to = d
            break
        end
    end
    if to then
        for _, w in ipairs(gone:all("Workspace > Window")) do
            to.active:add(w)
        end
        to:arrange()
        gone:destroy()
        return
    end
    for _, ws in ipairs(gone.space.children) do
        ws.visible = false
    end
    local f = seat.focus
    if f and f:closest(Desk) == gone then
        seat.focus = nil -- a window with no output cannot hold focus
    end
    table.insert(parked_desks, gone)
end
-- Focus and raise policy: the window_management focus keys the compositor publishes as readonly seat props.
function focus_settings()
    return Focus.settings({
        focus_mode = seat.focus_mode,
        focus_new_windows = seat.focus_new_windows,
        raise_on_click = seat.raise_on_click,
        auto_raise = seat.auto_raise,
        auto_raise_delay = seat.auto_raise_delay,
        focus_change_on_pointer_rest = seat.focus_change_on_pointer_rest,
    })
end
-- Window rules: seat.data.window_rules is the user's list. A new list is compiled once, when it is first read. A list
-- that does not compile (a bad pattern, an unknown field, an opacity outside [0, 1]) is reported once and refused
-- whole: the rules in force stay. Assign a new table to change the rules; editing the table in place is not seen.
local rules_src, rules_compiled = nil, {}
function window_rules()
    local src = seat and seat.data.window_rules or nil
    if src ~= rules_src then
        rules_src = src
        local ok, compiled = pcall(Rules.compile, src or {})
        if ok then
            rules_compiled = compiled
        else
            report(("window rules refused, the rules in force stay: %s"):format(tostring(compiled)))
        end
    end
    return rules_compiled
end

-- Workspace sets: seat.data.workspace_sets is the user's list of { name, entries = { { workspace, output? } } }. An
-- entry names a workspace of the Desk (its name, "1" to "9" in the stock Desk) and the output it shows on; no output
-- means the primary one. A new list is checked once, when it is first read: a list that does not check (empty or
-- duplicate name, no entries, a workspace no Desk has, two entries for one output) is reported once and refused whole,
-- and the sets in force stay. Assign a new table to change the sets; editing the table in place is not seen.
local sets_src, sets_list, sets_active = nil, {}, nil
local function declared_workspaces()
    local out = {}
    for _, d in ipairs(root:all(Desk)) do
        for _, ws in ipairs(d.space.children) do
            out[ws.name] = true
        end
    end
    return out
end
-- The sets as Sets.define reads them: workspace names as text, whatever the user wrote.
local function set_entries(list)
    local out = {}
    for i, s in ipairs(list) do
        local entries = {}
        for j, e in ipairs(s.entries or {}) do
            entries[j] = { workspace = tostring(e.workspace), output = e.output }
        end
        out[i] = { name = s.name, entries = entries }
    end
    return out
end
local SET_ERRORS = {
    empty_name = function()
        return "a set has an empty name"
    end,
    duplicate_name = function(e)
        return ("two sets are named %q"):format(e.name)
    end,
    empty_set = function(e)
        return ("set %q has no entries"):format(e.name)
    end,
    undeclared_workspace = function(e)
        return ("set %q names workspace %q, which no desk has"):format(e.set, tostring(e.workspace))
    end,
    duplicate_output = function(e)
        return ("set %q has two entries for output %q"):format(e.set, e.output)
    end,
    unknown_set = function(e)
        return ("no set is named %q"):format(tostring(e.name))
    end,
}
local function explain_set_error(err)
    local f = SET_ERRORS[err.kind]
    return f and f(err) or tostring(err.kind)
end
function workspace_sets()
    local src = seat and seat.data.workspace_sets or nil
    if src ~= sets_src then
        sets_src = src
        local ok, list, err = pcall(function()
            return Sets.define(sets_list, set_entries(src or {}), declared_workspaces())
        end)
        if not ok then
            err = { kind = tostring(list) }
            list = nil
        end
        if list then
            sets_list = list
        else
            report(("workspace sets refused, the sets in force stay: %s"):format(explain_set_error(err)))
        end
    end
    return sets_list
end
-- Makes the set `name` live: each entry shows its workspace on its output (the Desk of the Screen with that name), and
-- an entry whose output is gone shows on the primary output, with a report. Activating the live set again changes
-- nothing. Refused (false, and a report) for an unknown set or a workspace that has since gone.
function activate_set(name)
    local list = workspace_sets()
    local outputs, primary = {}, nil
    for _, s in ipairs(root:all(Screen)) do
        outputs[#outputs + 1] = s.name
        if s.primary then
            primary = s.name
        end
    end
    local plan, err = Sets.activate(list, name, {
        declared = declared_workspaces(),
        outputs = outputs,
        primary = primary or outputs[1],
        active = sets_active,
    })
    if not plan then
        report(("workspace set refused: %s"):format(explain_set_error(err)))
        return false
    end
    for _, w in ipairs(plan.warnings) do
        report(
            ("workspace set %q: output %q is not connected; its workspace shows on the primary output"):format(
                w.set,
                w.output
            )
        )
    end
    if not plan.changed then
        return true
    end
    sets_active = name
    for _, p in ipairs(plan.placements) do
        for _, s in ipairs(root:all(Screen)) do
            local d = s.name == p.output and desk_of(s)
            if d then
                for _, ws in ipairs(d.space.children) do
                    if ws.name == p.workspace then
                        go(function()
                            d:show(ws)
                        end)
                    end
                end
            end
        end
    end
    return true
end

function Window:on_ready()
    local p = self.parent_window
    if p then
        p:add(self) -- a dialog moves with its parent
        self.x, self.y = (p.width - self.width) // 2, (p.height - self.height) // 2
    else
        local d = desk()
        local target = self:rule_workspace(d) or d.active
        target:add(self)
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
-- The fields the rules own, applied in this order (fullscreen before maximised: a fullscreen window does not maximise),
-- with the value each takes when no rule sets it. Rules are the only writer of these fields.
local RULE_FIELDS = { "secure", "fullscreen", "maximised", "opacity" }
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
    if (fx.workspace or fx.maximised) and not d then
        return -- not on a desk yet: on_ready applies the rules once the window is placed
    end
    state.key = key
    rule_state[self] = state
    self.data.xr = fx.xr -- read by xr.lua: `xr = false` keeps the window out of the headset's room
    for _, field in ipairs(RULE_FIELDS) do
        if field == "maximised" and fx.maximised ~= nil then
            if fx.maximised then
                self:maximise()
            else
                self:unmaximise()
            end
            state.wrote[field] = true
        elseif field == "maximised" and state.wrote[field] then
            self:unmaximise()
            state.wrote[field] = nil
        elseif field == "fullscreen" and fx.fullscreen ~= nil then
            if fx.fullscreen then
                self:enter_fullscreen()
            else
                self:leave_fullscreen()
            end
            state.wrote[field] = true
        elseif field == "fullscreen" and state.wrote[field] then
            self:leave_fullscreen()
            state.wrote[field] = nil
        elseif fx[field] ~= nil then
            self[field] = fx[field]
            state.wrote[field] = true
        elseif state.wrote[field] then
            self[field] = RULE_DEFAULTS[field]
            state.wrote[field] = nil
        end
    end
    if fx.workspace and d and not self.parent_window then -- a dialog follows its parent, whatever a rule says
        local target
        for _, ws in ipairs(d.space.children) do
            if ws.name == tostring(fx.workspace.id) then
                target = ws
            end
        end
        if not target then -- reported once per match: the key above stops a repeat. The window stays where it is.
            report(
                ("window rule: workspace %q does not exist; %s stays where it is"):format(
                    tostring(fx.workspace.id),
                    self.app_id ~= "" and self.app_id or "the window"
                )
            )
        elseif self:closest(Workspace) ~= target then
            target:add(self)
        end
    end
end
-- The workspace of `d` a rule sends this window to, or nil. It is read before the window is placed, so the window is
-- inserted once: on the real core a second insert of one node in the same frame does not take effect, and the window
-- would stay where the first put it.
function Window:rule_workspace(d)
    if self.parent_window then
        return nil
    end
    local fx = Rules.effects(window_rules(), rule_view(self))
    if not fx.workspace then
        return nil
    end
    for _, ws in ipairs(d.space.children) do
        if ws.name == tostring(fx.workspace.id) then
            return ws
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
function Window:on_activate()
    self:present()
end
-- An activation request raises and focuses a window on the active workspace, and brings a minimised one back.
function Window:present()
    if self:closest(Workspace) == desk().active then
        if self.minimised then
            self:unminimise()
        end
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
-- The bottom of its siblings, so it draws under them. A lowered window keeps its focus and stacking otherwise.
function Window:lower()
    self.parent:add(self, 1)
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
    self:leave_fullscreen() -- the bars come back
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
-- The rectangle a fullscreen window covers, in its workspace's space: the whole Screen, under the bars. Nil when the
-- window is on no Desk.
local function cover_rect(w)
    local d = w:closest(Desk)
    return d and { x = 0, y = -d.space.y, w = d.width, h = d.height }
end
-- Bars and the Desk's clip give way to a fullscreen window that is showing, and come back when none is.
function Desk:sync_fullscreen()
    local covered = false
    for _, w in ipairs(self.active and self.active:all("> Window") or {}) do
        covered = covered or (w.fullscreen and not w.minimised)
    end
    for _, bar in ipairs(self:all("> Bar")) do
        bar.visible = not covered
    end
    self.space.clip = not covered
end
-- Covers the whole output, bars included, and goes last among its siblings. The rectangle it had is kept, so leaving
-- fullscreen returns a floating, snapped or maximised window to it. Refused (false) when it is fullscreen already.
function Window:enter_fullscreen()
    if self.fullscreen then
        return false
    end
    local r = cover_rect(self)
    if r then
        self.data.windowed = { x = self.x, y = self.y, w = self.width, h = self.height }
    end
    self.fullscreen = true
    if r then
        self.x, self.y, self.width, self.height = r.x, r.y, r.w, r.h
        self:raise()
        self:closest(Desk):sync_fullscreen()
    end
    return true
end
function Window:leave_fullscreen()
    if not self.fullscreen then
        return false
    end
    self.fullscreen = false
    local r = self.data.windowed
    self.data.windowed = nil
    if r then
        self.x, self.y, self.width, self.height = r.x, r.y, r.w, r.h
    end
    local d = self:closest(Desk)
    if d then
        d:sync_fullscreen()
    end
    return true
end
-- The window the seat's focus passes to when `w` leaves the screen: the topmost sibling still showing, or none.
local function next_focus(w)
    local siblings = w.parent and w.parent.children or {}
    for i = #siblings, 1, -1 do
        local c = siblings[i]
        if c ~= w and c:is(Window) and not c.minimised then
            return c
        end
    end
end
-- Hides the window and hands the seat's focus on, as the window manager did. Refused (false) when it is minimised.
function Window:minimise()
    if self.minimised then
        return false
    end
    self.minimised = true
    self.visible = false
    if seat and seat.focus == self then
        seat.focus = next_focus(self)
    end
    local d = self:closest(Desk)
    if d then
        d:sync_fullscreen()
    end
    return true
end
-- Shows the window again. The seat's focus goes to it only when nothing holds it. Refused (false) when it is not
-- minimised.
function Window:unminimise()
    if not self.minimised then
        return false
    end
    self.minimised = false
    self.visible = true
    if seat and seat.focus == nil then
        seat.focus = self
    end
    local d = self:closest(Desk)
    if d then
        d:sync_fullscreen()
    end
    return true
end
-- Re-fits a maximised or snapped window to a work area that has changed (a bar was added, removed or resized). A
-- fullscreen window keeps covering the Screen.
function Window:refit(a)
    if self.fullscreen then
        local r = cover_rect(self)
        if r then
            self.x, self.y, self.width, self.height = r.x, r.y, r.w, r.h
        end
    elseif self.maximised then
        self.x, self.y, self.width, self.height = a.x, a.y, a.w, a.h
    elseif self.data.snapped then
        local rect = Snap.tile_rect(self.data.snapped, a)
        self.x, self.y, self.width, self.height = rect.x, rect.y, rect.w, rect.h
    end
end
-- A rectangle moved to another output: shrunk to the area when larger, then moved inside it.
local function fit_into(r, a)
    return Placement.clamp({ x = r.x, y = r.y, w = math.min(r.w, a.w), h = math.min(r.h, a.h) }, a)
end
-- Moves the window to another workspace, on any Desk. Refused (false) for a fullscreen window, a dialog (it moves
-- only with its parent), a workspace in no Desk, or the one it is already on. Its rectangles are refitted to the new
-- Desk's work area.
function Window:move_to(ws)
    local to = ws and ws:is(Workspace) and ws:closest(Desk)
    if not to or self.fullscreen or self.parent_window or self.parent == ws then
        return false
    end
    local from = self.parent
    local was_focused = seat.focus == self
    ws:add(self)
    if was_focused and not ws.active then -- the window left the screen, so focus passes to a window that stays
        local next_focus
        for _, o in ipairs(from:all(Window)) do
            if o ~= self then
                next_focus = o
                break
            end
        end
        seat.focus = next_focus
    end
    local a = to:work_area()
    self:refit(a)
    if not self.maximised and not self.data.snapped then
        local r = fit_into({ x = self.x, y = self.y, w = self.width, h = self.height }, a)
        self.x, self.y, self.width, self.height = r.x, r.y, r.w, r.h
    end
    if self.data.floating then
        self.data.floating = fit_into(self.data.floating, a)
    end
    return true
end
-- Moves the window to the active workspace of the Desk on `screen`. Refused (false) when no Desk shows that screen.
function Window:move_to_output(screen)
    local to = desk_of(screen)
    if not to then
        return false
    end
    return self:move_to(to.active)
end
-- The client asks for a state change; the shell decides and the window's properties echo the answer. The
-- compositor turns `maximised`, `fullscreen`, `minimised` and the size into the client's configure, and runs `close`.
function Window:on_request(e)
    local request = e.request
    if request == "maximize" then
        self:maximise()
    elseif request == "unmaximize" then
        self:unmaximise()
    elseif request == "fullscreen" then
        self:enter_fullscreen()
    elseif request == "unfullscreen" then
        self:leave_fullscreen()
    elseif request == "minimize" then
        self:minimise()
    elseif request == "unminimize" then
        self:unminimise()
    elseif request == "close" then
        self:close()
    elseif request == "raise" then
        self:raise()
    elseif request == "lower" then
        self:lower()
    elseif request == "activate" then
        self:present()
    end
end
-- Super+Left and Super+Right move focus between the columns of a Strip; on any other layout they do nothing.
-- A shell loaded without a seat (the Lua tests) has no keys to bind.
if seat then
    seat:bind({
        ["<Super>Left"] = function()
            if desk().active:is(Strip) then
                desk().active:focus_left()
            end
        end,
        ["<Super>Right"] = function()
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
require("gnoblin.shell.gestures")
require("gnoblin.shell.xr")
require("gnoblin.shell.decorations")
