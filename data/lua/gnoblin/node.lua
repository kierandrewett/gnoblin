-- data/lua/gnoblin/node.lua: classes, properties, bindings, signals, motion, selectors, scheduler.
-- Plain Lua 5.5 over the native core. The host runs it once and calls the function it returns.
local core = ...
local CLASS, ID, VALS, BOUND, HANDLERS = {}, {}, {}, {}, {} -- private keys: no name can collide
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
    local ok, err = pcall(b.fn)
    current = outer
    if not ok then
        core.report(err)
    end
end
local function bind(fn)
    seq = seq + 1
    local b = { fn = fn, seq = seq, sources = {} }
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
local function resume(co, ...)
    local o = owners[co]
    if o and not nodes[rawget(o, ID)] then
        return
    end
    local outer = current
    current = nil -- a handler may await even when a binding started it
    local ok, err = coroutine.resume(co, ...)
    current = outer
    if not ok then
        core.report(err)
    end
end
local function start(o, fn, ...)
    local co = coroutine.create(fn)
    owners[co] = o
    resume(co, ...)
    return co
end
function go(fn, ...)
    return start(owners[coroutine.running()], fn, ...)
end
local function emit(n, name, ...)
    local chain, c = {}, rawget(n, CLASS)
    while c do
        table.insert(chain, 1, c)
        c = meta(c).base
    end -- base class handlers first
    for _, k in ipairs(chain) do
        for _, fn in ipairs(meta(k).handlers[name] or {}) do
            start(n, fn, n, ...)
        end
    end
    local l = rawget(n, HANDLERS)
    local list, keep = l[name] or {}, {}
    l[name] = nil
    for _, h in ipairs(list) do
        if h.fn then
            keep[#keep + 1] = h
            start(n, h.fn, n, ...)
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

-- Nodes. Values of native properties live in the native tree; Lua properties live in VALS.
local make
local function wrap(id) -- a node Gnoblin made gets its Lua face
    if id == nil then
        return nil
    end
    if nodes[id] then
        return nodes[id]
    end
    local cls = classes[core.get(id, "class")]
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
        core.set(rawget(n, ID), k, p.type == "node" and v and rawget(v, ID) or v, seconds, curve)
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
        end)
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
            end)
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

-- Chord claims. The Lua side allocates each claim id (per VM, from 1, never reused) and remembers which seat
-- made it, so a chord event from the compositor can be routed to that seat.
local claims, next_claim = {}, 0
local function install_claims(c)
    rawset(c, "claim", function(seat, chord, grab)
        next_claim = next_claim + 1
        local id = next_claim
        claims[id] = seat
        core.call(rawget(seat, ID), "claim", string.pack("<i8i8", id, grab and 1 or 0), chord)
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
                return setmetatable({ node = n, name = v }, Signal)
            end
            return v
        end)
    end
    if s.name == "Seat" then
        install_claims(c)
    end
    settle(m)
end
local Node = classes.Node
meta(Node).props.tags = { type = "table", default = {} }
meta(Node).props.data = { type = "table", default = {} }
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
        emit(child, "ready")
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
    local curve = {}
    for i = 0, 32 do
        curve[i + 1] = (curve_fn or ease.out)(i / 32)
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
    timers[#timers + 1] = { at = now + seconds, seq = seq, signal = done }
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

-- The one entry the host calls each frame: the frame clock, changed native properties, events.
return function(t, changes, events)
    now = t
    for _, c in ipairs(changes) do
        local n = wrap(c[1])
        if n then
            touch(n, c[2])
        end
    end
    for _, e in ipairs(events) do
        if e.chord then -- a claimed chord: routed to the seat that made the claim
            local seat = claims[e.chord.claim]
            if seat and nodes[rawget(seat, ID)] then
                emit(seat, "chord", e.chord)
            end
        elseif e.bubbles then -- input: the path is fixed first, as in the DOM
            e.target = wrap(e.target)
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
            local n = wrap(e.target)
            if n then
                emit(n, e.type, table.unpack(e.args or {}))
            end
            if e.type == "destroyed" then
                nodes[e.target] = nil
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
