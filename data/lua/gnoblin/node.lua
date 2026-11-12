-- data/lua/gnoblin/node.lua: classes, properties, bindings, signals, motion, selectors, scheduler.
-- Plain Lua 5.5 over the native core. The host runs it once and calls the function it returns.
local core = ...
local CLASS, ID, VALS, BOUND, HANDLERS = {}, {}, {}, {}, {} -- private keys: no name can collide
local PAINT = {} -- the key of the binding that paints a Canvas's `draw` into its `content`
local nodes, classes, timers = {}, {}, {} -- id -> node, name -> class
local now, seq, current, queue = 0, 0, nil, {}
local Class, Instance, Signal = {}, {}, {} -- the three metatables
signal = setmetatable({}, { __name = "signal" }) -- marker: `pressed = signal`
local function meta(c)
    return rawget(c, CLASS)
end

-- Reactivity. A binding re-runs when a property it read changes, never because of its own write.
local readers = setmetatable({}, { __mode = "k" }) -- readers[node][key] = { [binding] = true }
local function track(n, k)
    if not current then
        return
    end
    local by = readers[n] or {}
    readers[n] = by
    local set = by[k] or {}
    by[k] = set
    set[current] = true
    current.sources[#current.sources + 1] = set
end
local function touch(n, k)
    for b in pairs(readers[n] and readers[n][k] or {}) do
        if b ~= current and not b.queued then
            b.queued = true
            queue[#queue + 1] = b
        end
    end
end
local function run(b)
    for _, set in ipairs(b.sources) do
        set[b] = nil
    end
    b.sources, b.queued = {}, false
    local outer = current
    current = b
    local ok, err = core.pcall(b.fn)
    current = outer
    if not ok then
        core.report(b.label .. ": " .. tostring(err))
    end
end
local function bind(fn, label)
    seq = seq + 1
    local b = { fn = fn, seq = seq, sources = {}, label = label or "binding" }
    run(b)
    return b
end
local function flush()
    for _ = 1, 100 do
        if #queue == 0 then
            return
        end
        local batch = queue
        queue = {}
        table.sort(batch, function(a, b)
            return a.seq < b.seq
        end) -- creation order: deterministic
        for _, b in ipairs(batch) do
            if not b.dead then
                run(b)
            end
        end
    end
    for _, b in ipairs(queue) do
        b.queued = false
    end -- bindings still waiting are dropped, not retried
    queue = {}
    core.report("bindings did not settle in 100 passes: two bindings feed each other")
end

-- Scheduler. Every handler runs in its own coroutine, so `await` works anywhere but in a binding.
-- A coroutine belongs to the node whose handler started it and stops when that node is destroyed.
local owners = setmetatable({}, { __mode = "k" })
local labels = setmetatable({}, { __mode = "k" }) -- what a coroutine is, named in a report of its failure
local fail_hooks = setmetatable({}, { __mode = "k" }) -- what to do when a coroutine fails
local function resume(co, ...)
    local o = owners[co]
    if o and not nodes[rawget(o, ID)] then
        return
    end
    local outer = current
    current = nil -- a handler may await even when a binding started it
    local ok, err = core.resume(co, ...)
    current = outer
    if not ok then
        core.report((labels[co] or "handler") .. ": " .. tostring(err))
        local hook = fail_hooks[co]
        if hook then
            hook()
        end
    end
end
local function start(o, label, fn, ...)
    local co = coroutine.create(fn)
    owners[co] = o
    labels[co] = label
    resume(co, ...)
    return co
end
function go(fn, ...)
    return start(owners[coroutine.running()], "go", fn, ...)
end
local function emit(n, name, ...)
    local chain, c = {}, rawget(n, CLASS)
    while c do
        table.insert(chain, 1, c)
        c = meta(c).base
    end -- base class handlers first
    for _, k in ipairs(chain) do
        for _, fn in ipairs(meta(k).handlers[name] or {}) do
            start(n, ("%s on_%s handler"):format(meta(k).name, name), fn, n, ...)
        end
    end
    local l = rawget(n, HANDLERS)
    local list, keep = l[name] or {}, {}
    l[name] = nil
    for _, h in ipairs(list) do
        if h.fn then
            keep[#keep + 1] = h
            start(n, ("%s %s handler"):format(meta(rawget(n, CLASS)).name, name), h.fn, n, ...)
        else
            resume(h.co, ...)
        end
    end
    for _, h in ipairs(l[name] or {}) do
        keep[#keep + 1] = h
    end -- handlers added meanwhile
    l[name] = #keep > 0 and keep or nil
end
local function listen(n, name, h)
    local l = rawget(n, HANDLERS)
    l[name] = l[name] or {}
    table.insert(l[name], h)
end
Signal.__index = Signal
function Signal:connect(fn)
    listen(self.node, self.name, { fn = fn })
end
function Signal:emit(...)
    emit(self.node, self.name, ...)
end
function await(x)
    if current then
        error("await inside a binding", 2)
    end
    local co, main = coroutine.running()
    if main then
        error("await outside a handler", 2)
    end
    if type(x) == "number" then
        seq = seq + 1
        timers[#timers + 1] = { at = now + x, seq = seq, co = co }
    else
        listen(x.node, x.name, { co = co })
    end
    return coroutine.yield()
end
function every(seconds, fn)
    return go(function()
        while true do
            fn()
            await(seconds)
        end
    end)
end
-- Logs `message` as a script report. It never raises, so a refused rule or set does not stop the handler reporting it.
function report(message)
    core.report(tostring(message))
end

-- Slots. A slot is a prompt the compositor serves through whoever claims it: `portal.ui` is the consent dialog an app's
-- portal request asks, `prompt.secret` the passphrase prompt. The compositor's own dialog is the default and the
-- fallback, so a shell that claims a slot replaces the dialog, and one that cannot answer lets the compositor's serve.
--
--   slots.claim("portal.ui", function(offer) ... end)   take the slot; refused with the compositor's code when it is
--                                                       held, or when this is not the session owner's connection
--   slots.release("portal.ui")                          give it back; the compositor's dialog serves again
--
-- Each request for the slot reaches the handler as an offer, in a coroutine of its own, so the handler can `await`.
-- An offer has `request`, `slot`, `title` (what asked), `message` (the question), `subject` (who asked: the app id
-- the portal verified, never text the app wrote) and `deadline`, and:
--
--   offer:answer(text)       settle it. `portal.ui` takes exactly "allow" or "deny"; `prompt.secret` takes the
--                            passphrase. An answer is never logged or kept: it goes to the compositor and no further.
--   offer:decline()          this shell will not answer; the compositor's own dialog serves the request
--   offer:on_finished(fn)    `fn(offer, completion)` when the request ends without this shell: "ended" (its
--                            requester left), "timed-out", "refused", or "lost" (the connection went)
--   offer.finished           true once it has ended or been settled
--
-- A handler that raises is reported and declines its offer. One that never answers lets the deadline pass, and the
-- compositor takes the request back.
local slot_handlers, live_offers = {}, {}
local Offer = {}
Offer.__index = Offer
local function finish(offer, completion)
    if offer.finished then
        return
    end
    offer.finished, offer.completion = true, completion
    live_offers[offer.request] = nil
    local callbacks = offer.callbacks
    offer.callbacks = {}
    for _, fn in ipairs(callbacks) do
        start(nil, "offer finished handler", fn, offer, completion)
    end
end
function Offer:answer(text)
    if self.finished then
        error(("the offer is over (%s)"):format(self.completion), 2)
    end
    local ok, err = pcall(core.request_answer, self.request, text)
    if not ok then
        finish(self, "refused")
        error(err, 2)
    end
    finish(self, "answered")
end
function Offer:decline()
    if self.finished then
        return
    end
    local ok, err = pcall(core.request_decline, self.request)
    finish(self, "declined")
    if not ok then
        error(err, 2)
    end
end
function Offer:on_finished(fn)
    if self.finished then
        start(nil, "offer finished handler", fn, self, self.completion)
    else
        table.insert(self.callbacks, fn)
    end
end
slots = {}
function slots.claim(name, handler)
    if type(name) ~= "string" or type(handler) ~= "function" then
        error("slots.claim(name, handler): a slot name and a function(offer)", 2)
    end
    if slot_handlers[name] then
        error(("slots.claim: this shell already claims %s"):format(name), 2)
    end
    core.slot_claim(name)
    slot_handlers[name] = handler
end
function slots.release(name)
    if not slot_handlers[name] then
        return
    end
    slot_handlers[name] = nil
    for _, offer in pairs(live_offers) do
        if offer.slot == name then
            finish(offer, "ended")
        end
    end
    core.slot_release(name)
end
function slots.claimed(name)
    return slot_handlers[name] ~= nil
end
local function offered(e)
    local offer = setmetatable({
        request = e.request,
        slot = e.slot,
        title = e.title,
        message = e.message,
        subject = e.subject,
        deadline = e.deadline,
        callbacks = {},
        finished = false,
    }, Offer)
    -- Claiming is what makes this shell the one that answers. A request someone else took, or that is already over,
    -- is not ours to draw.
    if not pcall(core.request_claim, e.request) then
        return
    end
    local handler = slot_handlers[e.slot]
    if not handler then
        pcall(core.request_decline, e.request)
        return
    end
    live_offers[e.request] = offer
    -- No pcall around the handler: it may await, and a pcall of the host's cannot yield. A failure ends the coroutine,
    -- is reported by `resume`, and runs the hook that declines the offer.
    local co = coroutine.create(handler)
    labels[co] = ("slot %s handler"):format(e.slot)
    fail_hooks[co] = function()
        if not offer.finished then
            pcall(offer.decline, offer)
        end
    end
    resume(co, offer)
end
local function slot_event(e)
    if e.kind == "offered" then
        offered(e)
    elseif e.kind == "finished" then
        local offer = live_offers[e.request]
        if offer then
            finish(offer, e.completion)
        end
    elseif e.kind == "lost" then -- the connection went: every claim and offer with it
        slot_handlers = {}
        for _, offer in pairs(live_offers) do
            finish(offer, "lost")
        end
    end
end

-- The paint list. A Canvas's `draw` is a Lua array of drawing ops; `paint_encode` turns it into the bytes of the
-- `content` property, which the compositor decodes (crates/gnoblin-paint/src/list.rs describes the format and
-- the ops are documented in docs/shell-api/canvas.md). Colours are "#rgb", "#rgba", "#rrggbb" or "#rrggbbaa".
local function paint_colour(value, default)
    if value == nil then
        return default
    end
    local hex = type(value) == "string" and value:match("^#(%x+)$")
    if hex and (#hex == 3 or #hex == 4) then
        hex = (hex:gsub(".", "%0%0"))
    end
    if hex and #hex == 6 then
        hex = hex .. "ff"
    end
    if not hex or #hex ~= 8 then
        error(("a colour is #rgb, #rgba, #rrggbb or #rrggbbaa, not %s"):format(tostring(value)), 0)
    end
    return tonumber(hex, 16)
end
local function paint_box(op, key)
    local box = op[key]
    if type(box) ~= "table" or #box ~= 4 then
        error(("%s takes {x, y, width, height}"):format(key), 0)
    end
    return box[1], box[2], box[3], box[4]
end
local function paint_number(value, default)
    if value == nil then
        return default
    end
    if type(value) ~= "number" or value ~= value or value == math.huge or value == -math.huge then
        error(("expected a finite number, not %s"):format(tostring(value)), 0)
    end
    return value
end
local function paint_op(op)
    local pack = string.pack
    if op.rect then
        local x, y, w, h = paint_box(op, "rect")
        return pack("<Bfffff", 1, x, y, w, h, paint_number(op.radius, 0))
            .. pack("<I4I4f", paint_colour(op.fill, 0), paint_colour(op.stroke, 0), paint_number(op.stroke_width, 0))
    elseif op.line then
        local l = op.line
        if type(l) ~= "table" or #l ~= 4 then
            error("line takes {x1, y1, x2, y2}", 0)
        end
        return pack(
            "<BfffffI4",
            2,
            l[1],
            l[2],
            l[3],
            l[4],
            paint_number(op.width, 1),
            paint_colour(op.color, 0xffffffff)
        )
    elseif op.text then
        return pack(
            "<BfffI4s2s4",
            3,
            paint_number(op.x, 0),
            paint_number(op.y, 0),
            paint_number(op.size, 14),
            paint_colour(op.color, 0xffffffff),
            op.font or "sans",
            tostring(op.text)
        )
    elseif op.icon then
        return pack(
            "<BfffI4s2",
            4,
            paint_number(op.x, 0),
            paint_number(op.y, 0),
            paint_number(op.size, 16),
            paint_colour(op.color, 0xffffffff),
            tostring(op.icon)
        )
    elseif op.clip ~= nil then
        if op.clip == false then
            return pack("<B", 6)
        end
        local x, y, w, h = paint_box(op, "clip")
        return pack("<Bfffff", 5, x, y, w, h, paint_number(op.radius, 0))
    elseif op.opacity ~= nil then
        if op.opacity == false then
            return pack("<B", 8)
        end
        return pack("<Bf", 7, paint_number(op.opacity, 1))
    end
    error("an op is a rect, line, text, icon, clip or opacity", 0)
end
local function paint_encode(list)
    local parts = {}
    for i, op in ipairs(list) do
        local ok, bytes = pcall(paint_op, op)
        if not ok then
            error(("draw op %d: %s"):format(i, tostring(bytes)), 0)
        end
        parts[i] = bytes
    end
    return "GNPL" .. string.pack("<BI4", 1, #parts) .. table.concat(parts)
end

-- Nodes. Values of native properties live in the native tree; Lua properties live in VALS.
local readied = {} -- node id -> true once its `ready` has been emitted
local ready -- emits `ready` once per node; defined with the snapshot code below
local make
local function wrap(id) -- a node Gnoblin made gets its Lua face
    if id == nil then
        return nil
    end
    if nodes[id] then
        return nodes[id]
    end
    local known, name = pcall(core.get, id, "class")
    if not known then -- a node the compositor has dropped since the reference was written: nothing to wrap
        return nil
    end
    local cls = classes[name]
    if not cls then
        return nil
    end -- a native node the class table does not know
    return make(cls, id)
end
local function read(n, k, p)
    local b = rawget(n, BOUND)[k]
    if b and b.queued then
        run(b)
    end -- a stale binding settles before a read
    track(n, k)
    if not p.native then
        return rawget(n, VALS)[k]
    end
    local v = core.get(rawget(n, ID), k)
    if p.type == "node" then
        return wrap(v)
    end
    if p.type == "nodes" then
        local out = {}
        for i, id in ipairs(v) do
            out[i] = wrap(id)
        end
        return out
    end
    return v
end
local function write(n, k, p, v, seconds, curve)
    if p.native then
        local ok, err = pcall(core.set, rawget(n, ID), k, p.type == "node" and v and rawget(v, ID) or v, seconds, curve)
        if not ok then
            error(("%s.%s: %s"):format(meta(rawget(n, CLASS)).name, k, tostring(err)), 3)
        end
    else
        local t = p.type
        if v ~= nil and t ~= "any" and (type(t) == "string" and type(v) ~= t or type(t) == "table" and not v:is(t)) then
            error(("%s.%s takes %s, not %s"):format(meta(rawget(n, CLASS)).name, k, tostring(t), tostring(v)), 3)
        end
        if rawget(n, VALS)[k] == v then
            return
        end
        rawget(n, VALS)[k] = v
    end
    touch(n, k)
end
Instance.__index = function(n, k)
    local m = meta(rawget(n, CLASS))
    local p = m.props[k]
    if p then
        return read(n, k, p)
    end
    if m.signals[k] then
        return setmetatable({ node = n, name = k }, Signal)
    end
    return rawget(n, CLASS)[k] -- methods, through the class chain
end
Instance.__newindex = function(n, k, v)
    local m = meta(rawget(n, CLASS))
    local name = type(k) == "string" and k:match("^on_(.+)$")
    if name and m.signals[name] then
        return listen(n, name, { fn = v })
    end
    local p = m.props[k] or error(("%s has no property '%s'"):format(m.name, tostring(k)), 2)
    local old = rawget(n, BOUND)[k]
    if old then
        old.dead = true
        rawget(n, BOUND)[k] = nil
    end
    if type(v) == "function" then -- a function value is a binding
        rawget(n, BOUND)[k] = bind(function()
            write(n, k, p, v(n))
        end, ("%s.%s binding"):format(m.name, k))
    else
        write(n, k, p, v)
    end
end
Instance.__call = function(n, selector)
    return n:find(selector)
end
Instance.__tostring = function(n)
    return meta(rawget(n, CLASS)).name .. "#" .. tostring(n.name)
end

-- Classes. `Panel{ ... }` constructs; `function Panel:on_x()` adds a handler for every Panel.
function make(cls, id, spec) -- spec is nil for a node Gnoblin made
    local m = meta(cls)
    local n = setmetatable({ [CLASS] = cls, [ID] = id, [VALS] = {}, [BOUND] = {}, [HANDLERS] = {} }, Instance)
    nodes[id] = n
    local keys = {}
    for k in pairs(spec or {}) do
        if type(k) == "string" then
            keys[#keys + 1] = k
        end
    end
    table.sort(keys)
    for pass = 1, 3 do -- plain defaults, the call's fields, bound defaults
        if pass == 2 then
            for _, k in ipairs(keys) do
                n[k] = spec[k]
            end
        end
        for _, k in ipairs(pass ~= 2 and m.order or {}) do
            local p, d = m.props[k], m.props[k].default
            local skip = spec and spec[k] ~= nil or not spec and p.native or (type(d) == "function") ~= (pass == 3)
            if type(d) == "table" and getmetatable(d) == nil then
                local t = {}
                for i, v in pairs(d) do
                    t[i] = v
                end
                d = t
            end
            if not skip then
                n[k] = d
            end
        end
    end
    if spec ~= nil then -- wrapped nodes already have their children natively
        for _, c in ipairs(n:build() or {}) do
            n:add(c)
        end
        for _, c in ipairs(spec) do
            n:add(c)
        end
        if n.arrange ~= classes.Node.arrange then
            rawget(n, BOUND)[CLASS] = bind(function()
                n:arrange()
            end, m.name .. ":arrange")
        end
        if m.props.draw and m.props.content then -- a Canvas paints what its `draw` says, when that changes
            local sent
            rawget(n, BOUND)[PAINT] = bind(function()
                local list = n.draw
                if type(list) ~= "table" then
                    return
                end
                local bytes = paint_encode(list)
                if bytes ~= sent then
                    sent = bytes
                    n.content = bytes
                end
            end, m.name .. ":draw")
        end
    end
    return n
end
local function construct(cls, spec)
    return make(cls, core.new(meta(cls).native), spec or {})
end
Class.__index = function(c, k)
    local base = meta(c).base
    return base and base[k]
end
Class.__call = construct
Class.__tostring = function(c)
    return meta(c).name
end
Class.__newindex = function(c, k, v) -- `on_` names add; every other name replaces
    local name = type(k) == "string" and k:match("^on_(.+)$")
    if not (name and meta(c).signals[name]) then
        return rawset(c, k, v)
    end
    local h = meta(c).handlers
    h[name] = h[name] or {}
    table.insert(h[name], v)
end
local function declare(base, name, native)
    local bm = base and meta(base)
    local m = {
        name = name,
        base = base,
        native = native or bm.native,
        handlers = {},
        order = {},
        props = setmetatable({}, { __index = bm and bm.props }),
        signals = setmetatable({}, { __index = bm and bm.signals }),
    }
    local c = setmetatable({ [CLASS] = m }, Class)
    classes[name] = c
    _G[name] = c -- usable everywhere by its name
    return c, m
end
local function settle(m) -- every inherited default, sorted by name
    local seen, t = {}, m.props
    m.order = {}
    while t do
        for k, p in pairs(t) do
            if not seen[k] then
                seen[k] = true
                if m.props[k].default ~= nil and not m.props[k].readonly then
                    m.order[#m.order + 1] = k
                end
            end
        end
        t = getmetatable(t).__index
    end
    table.sort(m.order)
end
local function extend(base, name)
    if getmetatable(base) ~= Class then
        error("extend: base must be a class", 2)
    end
    if classes[name] then
        error("class " .. name .. " is already declared", 2)
    end
    return function(spec)
        local c, m = declare(base, name)
        for k, v in pairs(spec) do
            local p = m.props[k]
            if v == signal then
                m.signals[k] = true
            elseif p then
                m.props[k] = { native = p.native, type = p.type, default = v } -- a new default
            elseif getmetatable(v) == Class then
                m.props[k] = { type = v } -- a node of that class
            else
                m.props[k] = { type = type(v) == "function" and "any" or type(v), default = v }
            end
        end
        settle(m)
        return c
    end
end

-- Chord and gesture claims. The Lua side allocates each claim id (per VM, from 1, never reused, one number space for
-- both) and remembers which seat made it, so a chord or gesture event from the compositor can be routed to that seat.
local claims, next_claim = {}, 0

-- A claim's text: the chord or gesture kind, behind the name of the device the claim takes it from and a line feed when
-- it is scoped to one device. The device is a name from `Seat.device`, matched exactly, one line; nil is every device.
local function claim_text(subject, device)
    if device == nil then
        return subject
    end
    if type(device) ~= "string" or device == "" or device:find("%c") then
        error("a device selector is the name of a device, on one line: " .. tostring(device), 3)
    end
    return device .. "\n" .. subject
end

local function install_claims(c)
    -- Claim a chord, from every device or, with `device` (a name from `Seat.device`), from the devices that report
    -- that name only.
    rawset(c, "claim", function(seat, chord, grab, device)
        local text = claim_text(chord, device)
        next_claim = next_claim + 1
        local id = next_claim
        claims[id] = seat
        core.call(rawget(seat, ID), "claim", string.pack("<i8i8", id, grab and 1 or 0), text)
        return id
    end)
    -- A gesture of `kind` ("swipe" or "pinch") with `fingers` contacts: the compositor sends it to this seat as
    -- `gesture` events, from its begin to its end, and to no client. With `device` it is the gesture of the touchpads
    -- that report that name only.
    rawset(c, "claim_gesture", function(seat, kind, fingers, device)
        local text = claim_text(kind, device)
        next_claim = next_claim + 1
        local id = next_claim
        claims[id] = seat
        core.call(rawget(seat, ID), "claim_gesture", string.pack("<i8i8", id, fingers), text)
        return id
    end)
    rawset(c, "unclaim", function(seat, id)
        claims[id] = nil
        core.call(rawget(seat, ID), "unclaim", string.pack("<i8", id))
    end)
end

-- The native classes, parent first, from the schema. A method that takes time returns a signal.
for _, s in ipairs(core.classes()) do
    local c, m = declare(s.base and classes[s.base], s.name, s.name)
    for k, p in pairs(s.props) do
        m.props[k] = { native = true, type = p.type, default = p.default, readonly = p.readonly }
    end
    for _, sig in ipairs(s.signals) do
        m.signals[sig] = true
    end
    for _, f in ipairs(s.methods) do
        rawset(c, f, function(n, ...)
            local v = core.call(rawget(n, ID), f, ...)
            if s.slow and s.slow[f] then
                return setmetatable({ node = n, name = tostring(v) }, Signal)
            end
            return v
        end)
    end
    if s.name == "Seat" then
        install_claims(c)
    end
    if s.name == "Canvas" then
        -- `canvas:measure(text, size, font)` asks the compositor how much room `text` takes. It answers later, so the
        -- call returns a signal that fires with "<width> <height>" in whole pixels, and an error if there is one.
        rawset(c, "measure", function(n, text, size, font)
            local token = core.call(
                rawget(n, ID),
                "measure",
                string.pack("<d", size or 14),
                (font or "sans") .. "\n" .. tostring(text)
            )
            return setmetatable({ node = n, name = tostring(token) }, Signal)
        end)
    end
    if s.name == "View" then
        -- `view:capture(path)` writes a PNG of what the View shows to `path`, which must be an absolute `.png` path inside
        -- the session's runtime directory. It answers later, so the call returns a signal that fires with the path
        -- written, and an error if the compositor refused (`locked`, `unavailable`, `bad-value`, `too-large`,
        -- `io-error`). A View has no cursor, so there is no cursor option.
        rawset(c, "capture", function(n, path)
            if type(path) ~= "string" then
                error("capture: the path must be a string", 2)
            end
            local token = core.call(rawget(n, ID), "capture", nil, path)
            return setmetatable({ node = n, name = tostring(token) }, Signal)
        end)
    end
    settle(m)
end
local Node = classes.Node
if classes.Canvas then -- the paint list: an array of ops, or a function returning one
    meta(classes.Canvas).props.draw = { type = "any" }
end
meta(Node).props.tags = { type = "table", default = {} }
meta(Node).props.data = { type = "table", default = {} }
-- `on_ready` and `on_destroyed` are handlers, so they must be signals: without this the class
-- keeps `on_ready` as a plain method and nothing ever calls it.
meta(Node).signals.ready = true
meta(Node).signals.destroyed = true
-- `Floating:on_child_added` is a handler only when the signal is declared before the classes settle.
meta(Node).signals.child_added = true
-- The input signals the compositor sends as events for a node (design/node-api-refactor.md, section 3), and `request`
-- for a window's client request. The tree schema declares only `destroyed`, so without these `Window:on_press` and
-- `Entry:on_key` would be plain methods that nothing ever calls.
for _, name in ipairs({ "press", "release", "motion", "scroll", "enter", "leave", "key", "request" }) do
    meta(Node).signals[name] = true
end
for _, c in pairs(classes) do
    settle(meta(c))
end
rawset(Node, "extend", extend)
function Node:build() end
function Node:arrange() end
root = wrap(core.root())
local compound
function Node:is(sel) -- a class, or any selector without spaces
    if getmetatable(sel) ~= Class then
        return compound(sel)(self)
    end
    local k = rawget(self, CLASS)
    while k and k ~= sel do
        k = meta(k).base
    end
    return k ~= nil
end
function Node:add(child, index)
    if type(child) == "string" then
        child = classes.Text({ text = child })
    end
    local first = core.get(rawget(child, ID), "parent") == nil
    core.insert(rawget(self, ID), rawget(child, ID), index)
    touch(self, "children")
    touch(child, "parent")
    if first then
        ready(child)
    end
    emit(self, "child_added", child)
    return child
end
function Node:destroy()
    for _, c in ipairs(self.children) do
        c:destroy()
    end
    for _, b in pairs(rawget(self, BOUND)) do
        b.dead = true
    end
    local p = self.parent
    local cleared = core.destroy(rawget(self, ID)) -- references to it become nil
    nodes[rawget(self, ID)] = nil
    readied[rawget(self, ID)] = nil
    if p then
        touch(p, "children")
    end
    for _, c in ipairs(cleared) do
        touch(wrap(c[1]), c[2])
    end
end
function Node:raise()
    self.parent:add(self)
end -- the last child draws on top

-- Motion. `node:to(0.2, { x = 0 }, ease.out)` hands Gnoblin a sampled curve and returns a signal.
ease = {
    linear = function(t)
        return t
    end,
    out = function(t)
        return 1 - (1 - t) ^ 2
    end,
    out_cubic = function(t)
        return 1 - (1 - t) ^ 3
    end,
    in_cubic = function(t)
        return t ^ 3
    end,
    in_out = function(t)
        return t < 0.5 and 2 * t * t or 1 - (2 - 2 * t) ^ 2 / 2
    end,
}
function Node:to(seconds, props, curve_fn)
    local curve -- a zero duration is an instant write: no seconds and no curve go to the core
    if seconds > 0 then
        curve = {}
        for i = 0, 32 do
            curve[i + 1] = (curve_fn or ease.out)(i / 32)
        end
    else
        seconds = nil
    end
    local keys = {}
    for k in pairs(props) do
        keys[#keys + 1] = k
    end
    table.sort(keys)
    for _, k in ipairs(keys) do
        write(self, k, meta(rawget(self, CLASS)).props[k] or error("no property " .. k, 2), props[k], seconds, curve)
    end
    local done = setmetatable({ node = self, name = {} }, Signal) -- a fresh name per motion
    seq = seq + 1
    timers[#timers + 1] = { at = now + (seconds or 0), seq = seq, signal = done }
    return done
end

-- Selectors: `Type.tag#name[prop=value]`, joined by a space (descendant) or `>` (child).
local compiled = {}
function compound(s)
    local tests, t = {}, s:match("^[%a_][%w_]*")
    if t then
        local c = classes[t] or error("no class " .. t, 4)
        tests[1] = function(n)
            return n:is(c)
        end
    end
    for kind, word in s:gmatch("([.#])([%w_-]+)") do
        tests[#tests + 1] = kind == "." and function(n)
            return n.tags[word]
        end or function(n)
            return n.name == word
        end
    end
    for key, val in s:gmatch("%[([%w_]+)=?([^%]]*)%]") do
        tests[#tests + 1] = function(n)
            local v = n[key]
            if val == "" then
                return v ~= nil and v ~= false
            end
            return tostring(v) == val
        end
    end
    return function(n)
        for _, f in ipairs(tests) do
            if not f(n) then
                return false
            end
        end
        return true
    end
end
local function compile(sel)
    local s = tostring(sel)
    if not compiled[s] then
        local steps = {}
        for comb, part in (" " .. s):gmatch("%s*(>?)%s*([^%s>]+)") do
            steps[#steps + 1] = { child = comb == ">", test = compound(part) }
        end
        compiled[s] = steps
    end
    return compiled[s]
end
local function matches(n, steps, i, scope)
    if not steps[i].test(n) then
        return false
    end
    local p = n.parent
    if i == 1 then
        return not steps[1].child or p == scope
    end
    if steps[i].child then
        return p ~= scope and matches(p, steps, i - 1, scope)
    end
    while p and p ~= scope do
        if matches(p, steps, i - 1, scope) then
            return true
        end
        p = p.parent
    end
    return false
end
function Node:all(sel)
    local steps, out = compile(sel), {}
    local shallow = #steps == 1 and steps[1].child
    local function walk(n)
        for _, c in ipairs(n.children) do
            if matches(c, steps, #steps, self) then
                out[#out + 1] = c
            end
            if not shallow then
                walk(c)
            end
        end
    end
    walk(self)
    return out
end
function Node:unparented()
    local root_id, ids, out = core.root(), {}, {}
    for id in pairs(nodes) do
        if id ~= root_id and pcall(core.get, id, "parent") and core.get(id, "parent") == nil then
            ids[#ids + 1] = id
        end
    end
    table.sort(ids)
    for _, id in ipairs(ids) do
        out[#out + 1] = nodes[id]
    end
    return out
end
function Node:find(sel)
    return self:all(sel)[1]
end
function Node:closest(sel)
    local test, n = compound(tostring(sel)), self
    while n and not test(n) do
        n = n.parent
    end
    return n
end
function Node.describe(c) -- for the stub and docs generator
    local m, props, signals = meta(c), {}, {}
    for k, p in pairs(m.props) do
        props[k] = { type = tostring(p.type), default = p.default, native = p.native }
    end
    for k in pairs(m.signals) do
        signals[#signals + 1] = k
    end
    table.sort(signals)
    return { name = m.name, base = m.base and meta(m.base).name, props = props, signals = signals }
end

-- `ready` fires once per node: when it is first added to a parent, when the attach snapshot
-- installs it, or when Gnoblin creates it unparented (windows, which the shell places).
local booted = false
ready = function(n)
    if not booted then -- the snapshot pass announces nodes that exist before the shell starts
        return
    end
    local id = rawget(n, ID)
    if readied[id] then
        return
    end
    readied[id] = true
    emit(n, "ready")
end
local function announce_snapshot()
    local ids = {}
    local function collect(id)
        ids[#ids + 1] = id
        for _, c in ipairs(core.get(id, "children") or {}) do
            collect(c)
        end
    end
    collect(core.root())
    for _, id in ipairs(ids) do
        local n = wrap(id)
        if n then
            ready(n)
        end
    end
end

-- An input event can name a node the shell destroyed since: a key's release arrives after the press closed its
-- dialog. Such an event has no one to go to.
local function live(id)
    local ok, n = pcall(wrap, id)
    if ok then
        return n
    end
end
-- The one entry the host calls each frame: the frame clock, changed native properties, events,
-- the ids of nodes Gnoblin created this batch, and what the compositor sent for the slots this shell claims.
return function(t, changes, events, created, offers)
    now = t
    if not booted then
        booted = true
        announce_snapshot()
    end
    for _, id in ipairs(created or {}) do
        local n = wrap(id)
        if n then
            ready(n)
        end
    end

    for _, c in ipairs(changes) do
        local n = wrap(c[1])
        if n then
            touch(n, c[2])
        end
    end
    for _, o in ipairs(offers or {}) do
        slot_event(o)
    end
    for _, e in ipairs(events) do
        -- An input or request event carries its details (`x`, `y`, `button`, `dx`, `dy`, `key`, `text`, `pressed`,
        -- `request`) as fields of the event table its handlers receive.
        if e.fields then
            for k, v in pairs(e.fields) do
                e[k] = v
            end
        end
        if e.chord then -- a claimed chord: routed to the seat that made the claim
            local seat = claims[e.chord.claim]
            if seat and nodes[rawget(seat, ID)] then
                emit(seat, "chord", e.chord)
            end
        elseif e.gesture then -- a claimed gesture: routed the same way
            local seat = claims[e.gesture.claim]
            if seat and nodes[rawget(seat, ID)] then
                emit(seat, "gesture", e.gesture)
            end
        elseif e.bubbles then -- input: the path is fixed first, as in the DOM
            e.target = live(e.target)
            e.stop = function(self)
                self.stopped = true
            end
            local path, n = {}, e.target
            while n do
                path[#path + 1] = n
                n = n.parent
            end
            for _, at in ipairs(path) do
                if e.stopped then
                    break
                end
                if nodes[rawget(at, ID)] then
                    emit(at, e.type, e)
                end
            end
        else
            -- A destroyed node is already gone from the mirror when its event arrives, so only a face Lua made earlier
            -- can still run its handlers; one Lua destroyed itself has none left, and there is nothing to wrap.
            local n
            if e.type == "destroyed" then
                n = nodes[e.target]
            else
                n = live(e.target)
            end
            if n then
                if e.fields then -- a request, an enter or a leave: the node alone, with the event's details
                    e.target = n
                    e.stop = function(self)
                        self.stopped = true
                    end
                    emit(n, e.type, e)
                else
                    emit(n, e.type, table.unpack(e.args or {}))
                end
            end
            if e.type == "destroyed" then
                if n then -- its bindings must not run again against a node that is gone
                    for _, b in pairs(rawget(n, BOUND)) do
                        b.dead = true
                    end
                end
                nodes[e.target] = nil
                readied[e.target] = nil
            end -- Gnoblin ended it; handlers have run
        end
    end
    while true do
        table.sort(timers, function(a, b)
            return a.at < b.at or a.at == b.at and a.seq < b.seq
        end) -- a timer may have been added by a resumed handler
        if not timers[1] or timers[1].at > now then
            break
        end
        local due = table.remove(timers, 1)
        if due.signal then
            due.signal:emit()
        else
            resume(due.co)
        end
    end
    flush()
    return timers[1] and timers[1].at -- when to call again without input
end
